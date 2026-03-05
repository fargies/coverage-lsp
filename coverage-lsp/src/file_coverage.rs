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

use lcov_parser::LineData;
use tower_lsp::lsp_types::{
    Color, ColorInformation, Diagnostic, DiagnosticSeverity, FullDocumentDiagnosticReport, Position, Range, Url, WorkspaceDocumentDiagnosticReport, WorkspaceFullDocumentDiagnosticReport
};

use crate::LSP_NAME;

#[derive(Debug)]
pub struct LineCoverageInfo {
    pub line: u32,
    pub count: u32,
}

impl LineCoverageInfo {
    pub fn range(&self) -> Range {
        Range {
            start: Position {
                line: self.line,
                character: 0,
            },
            end: Position {
                line: self.line,
                character: u32::MAX,
            },
        }
    }
}

impl From<LineData> for LineCoverageInfo {
    fn from(value: LineData) -> Self {
        Self {
            line: value.line - 1,
            count: value.count,
        }
    }
}

#[derive(Debug)]
pub struct FileCoverage {
    pub uri: Url,
    pub coverage: Vec<LineCoverageInfo>,
}

impl FileCoverage {
    pub fn new(file: Url) -> Self {
        FileCoverage {
            uri: file,
            coverage: Vec::with_capacity(64),
        }
    }

    pub fn add(&mut self, data: LineData) {
        self.coverage.push(data.into());
    }

    pub fn create_diagnostic(
        &self,
        hit: Option<DiagnosticSeverity>,
        missed: Option<DiagnosticSeverity>,
    ) -> Vec<Diagnostic> {
        let mut ret = Vec::with_capacity(self.coverage.len());
        for cov in self.coverage.iter() {
            if cov.count != 0 && hit.is_some() {
                ret.push(Diagnostic::new(
                    cov.range(),
                    hit,
                    None,
                    Some(LSP_NAME.into()),
                    "line covered".into(),
                    None,
                    None,
                ));
            } else if missed.is_some() {
                ret.push(Diagnostic::new(
                    cov.range(),
                    missed,
                    None,
                    Some(LSP_NAME.into()),
                    "line not covered".into(),
                    None,
                    None,
                ));
            }
        }
        ret
    }

    pub fn create_workspace_document_diagnostic(&self) -> WorkspaceDocumentDiagnosticReport {
        WorkspaceFullDocumentDiagnosticReport {
            version: None,
            full_document_diagnostic_report: FullDocumentDiagnosticReport {
                result_id: None,
                items: self.create_diagnostic(
                    Some(DiagnosticSeverity::INFORMATION),
                    Some(DiagnosticSeverity::WARNING),
                ),
            },
            uri: self.uri.clone(),
        }
        .into()
    }

    pub fn create_document_color(&self) -> Vec<ColorInformation> {
        let mut ret = Vec::with_capacity(self.coverage.len());
        for cov in self.coverage.iter() {
            if cov.count != 0 {
                ret.push(ColorInformation { range: cov.range(), color: Color { red: 0.0, green: 1.0, blue: 0.0, alpha: 0.1 }});
            } else {
                ret.push(ColorInformation { range: cov.range(), color: Color { red: 1.0, green: 0.0, blue: 0.0, alpha: 0.1 }});
            }
        }
        ret
    }
}
