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
use std::ops::Deref;

use serde_json::Value;
use tower_lsp::jsonrpc::{Error, ErrorCode, Result};
use tower_lsp::lsp_types::*;
use tower_lsp::{LanguageServer, LspService, Server};
use tracing::Level;
use tracing_subscriber::{EnvFilter, Registry, fmt, layer::SubscriberExt, util::SubscriberInitExt};

mod file_coverage;
pub use file_coverage::FileCoverage;

mod coverage_report;
pub use coverage_report::CoverageReport;

mod coverage_lsp;
pub use coverage_lsp::CoverageLanguageServer;

mod settings;
pub use settings::Settings;

pub const LSP_NAME: &str = "coverage-lsp";

pub fn make_error<T>(msg: T) -> Error
where
    T: Into<Cow<'static, str>>,
{
    let mut err = Error::new(ErrorCode::ServerError(0));
    err.message = msg.into();
    err
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
                // Track opened/closed documents to send notifications on coverage file change
                #[cfg(feature = "notifications")]
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::NONE),
                        ..Default::default()
                    },
                )),
                #[cfg(feature = "color_provider")]
                color_provider: Some(ColorProviderCapability::Simple(true)),
                #[cfg(feature = "diagnostic_provider")]
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

        if let Some(mut config) = self.get_configuration().await
            && let Some(value) = config.get_mut(LSP_NAME).map(Value::take)
        {
            match serde_json::from_value::<Settings>(value) {
                Ok(settings) => Settings::set(settings),
                Err(err) => {
                    self.client
                        .log_message(
                            MessageType::ERROR,
                            format!("failed to parse settings: {err:?}"),
                        )
                        .await
                }
            }
        }
        self.update().await;
        self.context.start();
    }

    async fn shutdown(&self) -> Result<()> {
        self.context.stop().await;
        self.context.report.write().await.take();
        Ok(())
    }

    #[cfg(feature = "diagnostic_provider")]
    async fn workspace_diagnostic(
        &self,
        _params: WorkspaceDiagnosticParams,
    ) -> Result<WorkspaceDiagnosticReportResult> {
        match self.context.report.read().await.as_ref() {
            Some(report) => Ok(report.create_workspace_diagnostic()),
            None => Ok(WorkspaceDiagnosticReport::default().into()),
        }
    }

    #[cfg(feature = "diagnostic_provider")]
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

    #[cfg(feature = "color_provider")]
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

    #[cfg(feature = "notifications")]
    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.open_docs
            .write()
            .await
            .insert(params.text_document.uri);
    }

    #[cfg(feature = "notifications")]
    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.open_docs
            .write()
            .await
            .remove(&params.text_document.uri);
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        let mut params = params;
        if let Some(value) = params.settings.get_mut(LSP_NAME).map(Value::take) {
            match serde_json::from_value::<Settings>(value) {
                Ok(settings) if &settings != Settings::get().deref() => {
                    Settings::set(settings);
                    #[cfg(feature = "notifications")]
                    self.send_update_notification(false).await;
                }
                Ok(_) => { /* ignore change notification */ }
                Err(err) => {
                    self.client
                        .log_message(
                            MessageType::ERROR,
                            format!("failed to parse settings: {err:?}"),
                        )
                        .await
                }
            }
        }
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
        service
            .inner()
            .initialize(InitializeParams::default())
            .await?;
        service.inner().initialized(InitializedParams {}).await;
        assert!(service.inner().report.read().await.is_some());
        Ok(())
    }
}
