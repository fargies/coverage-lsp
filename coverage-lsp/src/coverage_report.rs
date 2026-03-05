/*
** Copyright (C) 2026 Sylvain Fargier
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
** Created on: 2026-03-05T15:24:50
** Author: Sylvain Fargier <fargier.sylvain@gmail.com>
*/

use std::time::Duration;
use std::{collections::HashMap, path::PathBuf, time::SystemTime};

use lcov_parser::{FromFile, LCOVParser};
use tower_lsp::lsp_types::{
    ColorInformation, DiagnosticSeverity, DocumentDiagnosticReport, FullDocumentDiagnosticReport, RelatedFullDocumentDiagnosticReport, Url, WorkspaceDiagnosticReportResult
};
use tower_lsp::{jsonrpc::Result, lsp_types::WorkspaceDiagnosticReport};

use crate::{FileCoverage, make_error};

#[derive(Debug)]
pub struct CoverageReport {
    pub path: PathBuf,
    pub mtime: SystemTime,
    pub id: String,

    pub db: HashMap<Url, FileCoverage>,
}

impl TryFrom<PathBuf> for CoverageReport {
    type Error = std::io::Error;

    fn try_from(path: PathBuf) -> std::result::Result<Self, Self::Error> {
        let mtime = path.metadata()?.modified()?;
        Ok(Self {
            id: format!(
                "{path:?}:{:?}",
                mtime
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or(Duration::ZERO)
                    .as_secs()
            ),
            mtime,
            path,
            db: Default::default(),
        })
    }
}

impl CoverageReport {
    pub fn is_outdated(&self) -> bool {
        !self
            .path
            .metadata()
            .is_ok_and(|m| m.modified().is_ok_and(|m| m == self.mtime))
    }

    pub fn load(&mut self, root_uri: &Url) -> Result<()> {
        let mut parser = match LCOVParser::from_file(&self.path) {
            Ok(parser) => parser,
            Err(err) => {
                tracing::error!(?err, file = ?self.path, "parsing error");
                return Err(make_error(format!("failed to parse: {err:?}")));
            }
        };

        let mut file: Option<FileCoverage> = None;
        while let Some(record) = parser
            .next()
            .inspect_err(|err| tracing::error!(?err, file = ?self.path, "parsing error"))
            .ok()
            .flatten()
        {
            match record {
                lcov_parser::LCOVRecord::SourceFile(src) => {
                    if let Some(cov) = file.take() {
                        self.db.insert(cov.uri.clone(), cov);
                    }
                    let url = match root_uri.join(&src) {
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
                        cov.add(line_data);
                    }
                }
                _ => (),
            }
        }
        if let Some(cov) = file.take() {
            self.db.insert(cov.uri.clone(), cov);
        }
        Ok(())
    }

    pub fn create_workspace_diagnostic(&self) -> WorkspaceDiagnosticReportResult {
        WorkspaceDiagnosticReportResult::Report(WorkspaceDiagnosticReport {
            items: self
                .db
                .values()
                .map(|v| v.create_workspace_document_diagnostic())
                .collect(),
        })
    }

    pub fn create_document_diagnostic(
        &self,
        uri: &Url,
        last_id: &Option<String>,
    ) -> Option<DocumentDiagnosticReport> {
        if last_id.as_ref().is_some_and(|last_id| last_id == &self.id) {
            return None;
        }
        return Some(
            RelatedFullDocumentDiagnosticReport {
                related_documents: None,
                full_document_diagnostic_report: FullDocumentDiagnosticReport {
                    result_id: Some(self.id.clone()),
                    items: self
                        .db
                        .get(uri)
                        .map(|report| {
                            report.create_diagnostic(
                                Some(DiagnosticSeverity::INFORMATION),
                                Some(DiagnosticSeverity::WARNING),
                            )
                        })
                        .unwrap_or_default(),
                },
            }
            .into(),
        );
    }

    pub fn create_document_color(&self, uri: &Url) -> Vec<ColorInformation> {
        match self.db.get(uri) {
            Some(report) => report.create_document_color(),
            None => Vec::default(),
        }
    }
}
