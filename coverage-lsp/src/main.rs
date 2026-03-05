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
use std::collections::VecDeque;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use tokio::sync::RwLock;
use tokio::task::{self, JoinHandle};
use tower_lsp::jsonrpc::{Error, ErrorCode, Result};
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};
use tracing::Level;
use tracing_subscriber::{EnvFilter, Registry, fmt, layer::SubscriberExt, util::SubscriberInitExt};

mod file_coverage;
use file_coverage::FileCoverage;

mod coverage_report;
use coverage_report::CoverageReport;

pub const LSP_NAME: &str = "coverage-lsp";

#[derive(Debug)]
struct CoverageLanguageServer {
    context: Arc<CoverageLanguageServerContext>,
}

impl CoverageLanguageServer {
    pub fn new(client: Client) -> Self {
        Self {
            context: CoverageLanguageServerContext::new(client),
        }
    }
}

impl std::ops::Deref for CoverageLanguageServer {
    type Target = CoverageLanguageServerContext;

    fn deref(&self) -> &Self::Target {
        &self.context
    }
}

#[derive(Debug)]
pub struct CoverageLanguageServerContext {
    client: Client,
    root_uri: RwLock<Url>,
    report: RwLock<Option<CoverageReport>>,
    join_handle: Mutex<Option<JoinHandle<()>>>,
}

pub fn make_error<T>(msg: T) -> Error
where
    T: Into<Cow<'static, str>>,
{
    let mut err = Error::new(ErrorCode::ServerError(0));
    err.message = msg.into();
    err
}

impl CoverageLanguageServerContext {
    pub fn new(client: Client) -> Arc<Self> {
        let ret = Arc::new(Self {
            client,
            root_uri: RwLock::new(
                Url::from_directory_path(std::env::current_dir().unwrap()).unwrap(),
            ),
            report: Default::default(),
            join_handle: Default::default(),
        });

        {
            let weak = Arc::downgrade(&ret);
            ret.join_handle
                .lock()
                .unwrap()
                .replace(task::spawn(CoverageLanguageServerContext::run(weak)));
        }
        ret
    }

    async fn run(weak: Weak<Self>) {
        while let Some(ctx) = weak.upgrade() {
            ctx.update().await;

            drop(ctx);
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    }

    async fn update(&self) {
        if let Some(file) = self.find_lcov_file().await
            && self
                .report
                .read()
                .await
                .as_ref()
                .is_none_or(|stamp| stamp.path != file || stamp.is_outdated())
        {
            self.client
                .log_message(MessageType::INFO, "loading file")
                .await;
            let mut report = match CoverageReport::try_from(file) {
                Ok(report) => report,
                Err(err) => {
                    self.client
                        .log_message(
                            MessageType::ERROR,
                            format!("failed to load report: {err:?}"),
                        )
                        .await;
                    return;
                }
            };
            let root_uri = self.root_uri.read().await.clone();

            if let Err(err) = report.load(&root_uri) {
                self.client
                    .log_message(
                        MessageType::ERROR,
                        format!("failed to parse report: {err:?}"),
                    )
                    .await
            } else {
                self.client
                    .log_message(MessageType::INFO, format!("loaded: {:?}", report.db.keys()))
                    .await;
                self.report.write().await.replace(report);
            }
        }
    }

    async fn find_lcov_file(&self) -> Option<PathBuf> {
        self.client
            .log_message(MessageType::INFO, "crawling for coverage file")
            .await;
        let mut dir_stack = VecDeque::with_capacity(64);
        dir_stack.push_back(PathBuf::from(self.root_uri.read().await.path()));

        while let Some(path) = dir_stack.pop_front() {
            let mut reader = match tokio::fs::read_dir(path).await.ok() {
                Some(reader) => reader,
                None => {
                    self.client
                        .log_message(MessageType::WARNING, "failed to read_dir: {path:?}")
                        .await;
                    continue;
                }
            };
            while let Ok(Some(entry)) = reader.next_entry().await {
                let path = entry.path();
                if path.is_dir() {
                    dir_stack.push_back(path);
                } else if path.extension().is_some_and(|ext| ext == "info") {
                    self.client
                        .log_message(MessageType::INFO, format!("coverage file found: {path:?}"))
                        .await;
                    return Some(path);
                }
            }
        }
        None
    }
}

impl Drop for CoverageLanguageServerContext {
    fn drop(&mut self) {
        if let Some(join_handle) = self.join_handle.lock().unwrap().take() {
            join_handle.abort();
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for CoverageLanguageServer {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        if let Some(root_uri) = params
            .root_uri
            .as_ref()
            .and_then(|uri| Url::from_directory_path(uri.as_str()).ok())
        {
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
                color_provider: Some(ColorProviderCapability::Simple(true)),
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
                #[cfg(feature = "diagnostics")]
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
        // if let Err(err) = self.client.register_capability(vec![Registration { id: "1".into(), method: "workspace/didChangeWatchedFiles".into(), register_options: Some(json!({ "watchers": [ { "globPattern": "*.info" } ] })) }]).await {
        //     self.client.log_message(MessageType::WARNING, format!("failed to watch for info files: {err:?}")).await;
        // }
        // if let Err(err) = self.client.register_capability(vec![Registration { id: "2".into(), method: "workspace/didChangeWatchedFiles".into(), register_options: Some(json!({ "watchers": [ {"baseUri": "/home/fargie_s/work/perso/ppm/target/coverage/output", "pattern": "*.info"} ] })) }]).await {
        //     self.client.log_message(MessageType::WARNING, format!("failed to watch for info files: {err:?}")).await;
        // }
        self.client
            .log_message(
                MessageType::INFO,
                format!("{:?}", self.root_uri.read().await),
            )
            .await;

        self.client
            .log_message(MessageType::INFO, "server initialized!")
            .await;
        self.context.update().await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    #[cfg(feature = "diagnostics")]
    async fn workspace_diagnostic(
        &self,
        _params: WorkspaceDiagnosticParams,
    ) -> Result<WorkspaceDiagnosticReportResult> {
        match self.context.report.read().await.as_ref() {
            Some(report) => Ok(report.create_workspace_diagnostic()),
            None => Ok(WorkspaceDiagnosticReport::default().into()),
        }
    }

    #[cfg(feature = "diagnostics")]
    async fn diagnostic(
        &self,
        params: DocumentDiagnosticParams,
    ) -> Result<DocumentDiagnosticReportResult> {
        match self
            .context
            .report
            .read()
            .await
            .as_ref()
            .and_then(|report| {
                report.create_document_diagnostic(
                    &params.text_document.uri,
                    &params.previous_result_id,
                )
            }) {
            Some(report) => Ok(DocumentDiagnosticReportResult::Report(report)),
            None => Ok(DocumentDiagnosticReportResult::Report(
                RelatedUnchangedDocumentDiagnosticReport {
                    related_documents: None,
                    unchanged_document_diagnostic_report: UnchangedDocumentDiagnosticReport {
                        result_id: String::default(),
                    },
                }
                .into(),
            )),
        }
    }

    async fn document_color(&self, params: DocumentColorParams) -> Result<Vec<ColorInformation>> {
        match self
            .context
            .report
            .read()
            .await
            .as_ref()
            .map(|report| report.create_document_color(&params.text_document.uri))
        {
            Some(report) => Ok(report),
            None => Ok(Vec::default()),
        }
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        self.client
            .log_message(
                MessageType::INFO,
                format!("watched files change: {params:?}"),
            )
            .await;
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

    let (service, socket) = LspService::new(CoverageLanguageServer::new);
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
        let (service, _) = LspService::new(CoverageLanguageServer::new);
        // let report = service.inner().parse_report("lcov.info").await?;
        // tracing::trace!(report = ?report);
        Ok(())
    }
}
