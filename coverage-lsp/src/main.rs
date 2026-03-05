/*
** Copyright (C) 2025 Sylvain Fargier
**
** This software is provided 'as-is', without any express or implied
** warranty.  In no event will the authors be held liable for any damages
** arising from the use of this software.
**
** Permission is granted to anyone to use this software for any purpose,
** including commercial applications, and to alter it and redistribute it
** freely, subject to the following restrictions:
**
** 1. The origin of this software must not be misrepresented; you must not
**    claim that you wrote the original software. If you use this software
**    in a product, an acknowledgment in the product documentation would be
**    appreciated but is not required.
** 2. Altered source versions must be plainly marked as such, and must not be
**    misrepresented as being the original software.
** 3. This notice may not be removed or altered from any source distribution.
**
** Author: Sylvain Fargier <fargier.sylvain@gmail.com>
*/

use std::borrow::Cow;

use lcov_parser::{FromFile, LCOVParser};
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::{Error, ErrorCode, Result};
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};
use tracing::Level;
use tracing_subscriber::{EnvFilter, Registry, fmt, layer::SubscriberExt, util::SubscriberInitExt};

mod file_coverage;
use file_coverage::FileCoverage;

pub const LSP_NAME: &str = "coverage-lsp";

#[derive(Debug)]
struct Backend {
    client: Client,
    root_uri: RwLock<Url>,
}

fn make_error<T>(msg: T) -> Error
where
    T: Into<Cow<'static, str>>,
{
    let mut err = Error::new(ErrorCode::ServerError(0));
    err.message = msg.into();
    err
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            root_uri: RwLock::new(
                Url::from_directory_path(std::env::current_dir().unwrap()).unwrap(),
            ),
        }
    }

    async fn parse_report<F>(&self, lcov_file: F) -> Result<WorkspaceDiagnosticReport>
    where
        F: AsRef<str>,
    {
        let mut report = WorkspaceDiagnosticReport::default();
        let mut parser = match LCOVParser::from_file(lcov_file.as_ref()) {
            Ok(parser) => parser,
            Err(err) => {
                tracing::error!(?err, file = lcov_file.as_ref(), "parsing error");
                return Err(make_error(format!("failed to parse: {err:?}")));
            }
        };

        let mut file: Option<FileCoverage> = None;
        while let Some(record) = parser
            .next()
            .inspect_err(|err| tracing::error!(?err, file = lcov_file.as_ref(), "parsing error"))
            .ok()
            .flatten()
        {
            match record {
                lcov_parser::LCOVRecord::SourceFile(src) => {
                    if let Some(cov) = file.take() {
                        report
                            .items
                            .push(WorkspaceDocumentDiagnosticReport::Full(cov.into()));
                    }
                    let url = match self.root_uri.read().await.join(&src) {
                        Ok(url) => url,
                        Err(err) => {
                            tracing::error!(?err, "failed to make Url from {src}");
                            continue;
                        }
                    };
                    file = Some(FileCoverage::new(url));
                }
                lcov_parser::LCOVRecord::Data(line_data) => {
                    if let Some(cov) = file.as_mut() {
                        cov.add(&line_data);
                    }
                }
                _ => (),
            }
        }
        if let Some(cov) = file.take() {
            report
                .items
                .push(WorkspaceDocumentDiagnosticReport::Full(cov.into()));
        }
        Ok(report)
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        if let Some(root_uri) = params.root_uri {
            *self.root_uri.write().await = root_uri;
        }
        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: LSP_NAME.into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
            capabilities: ServerCapabilities {
                workspace: Some(WorkspaceServerCapabilities {
                    /* FIXME ensure this is required */
                    workspace_folders: Some(WorkspaceFoldersServerCapabilities {
                        supported: Some(true),
                        change_notifications: Some(OneOf::Left(true)),
                    }),
                    file_operations: None,
                }),
                // text_document_sync: Some(TextDocumentSyncCapability::Options(
                //     TextDocumentSyncOptions {
                //         open_close: Some(true),
                //         change: Some(TextDocumentSyncKind::FULL),
                //         ..Default::default()
                //     },
                // )),
                // color_provider: Some(ColorProviderCapability::Simple(true)),
                // code_action_provider: Some(CodeActionProviderCapability::Options(
                //     CodeActionOptions {
                //         code_action_kinds: Some(vec![
                //             CodeActionKind::QUICKFIX,
                //             CodeActionKind::SOURCE_FIX_ALL,
                //         ]),
                //         ..Default::default()
                //     },
                // )),
                // hover_provider: Some(HoverProviderCapability::Simple(true)),
                diagnostic_provider: Some(DiagnosticServerCapabilities::Options(
                    DiagnosticOptions {
                        identifier: Some(LSP_NAME.to_string()),
                        inter_file_dependencies: true,
                        workspace_diagnostics: true,
                        work_done_progress_options: WorkDoneProgressOptions {
                            work_done_progress: Some(false),
                        },
                    },
                )),
                ..ServerCapabilities::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "server initialized!")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn workspace_diagnostic(
        &self,
        params: WorkspaceDiagnosticParams,
    ) -> Result<WorkspaceDiagnosticReportResult> {
        Ok(WorkspaceDiagnosticReportResult::Report(
            self.parse_report("target/coverage/output/lcov.info")
                .await?,
        ))
    }
}

#[tokio::main]
async fn main() {
    Registry::default()
        .with(
            EnvFilter::builder()
                .with_default_directive(Level::INFO.into())
                .from_env_lossy(),
        )
        .with(fmt::layer())
        .init();
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[ctor::ctor]
    fn log_init() {
        Registry::default()
            .with(
                EnvFilter::builder()
                    .with_default_directive(Level::TRACE.into())
                    .from_env_lossy(),
            )
            .with(fmt::layer())
            .init();
    }

    #[tokio::test]
    async fn parse() -> Result<()> {
        let (service, _) = LspService::new(Backend::new);
        let report = service.inner().parse_report("lcov.info").await?;
        tracing::trace!(report = ?report);
        Ok(())
    }
}
