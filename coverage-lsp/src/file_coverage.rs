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
    Diagnostic, DiagnosticSeverity, FullDocumentDiagnosticReport, Position, Range, Url,
    WorkspaceFullDocumentDiagnosticReport,
};

use crate::LSP_NAME;

pub struct FileCoverage {
    uri: Url,
    diag: Vec<Diagnostic>,
    is_hit: Option<bool>,
}

trait LineDataExt {
    fn start(&self) -> Position;
    fn end(&self) -> Position;
    fn range(&self) -> Range {
        Range::new(self.start(), self.end())
    }
    fn severity(&self) -> DiagnosticSeverity;
}

impl LineDataExt for LineData {
    fn start(&self) -> Position {
        Position {
            line: self.line,
            character: 0,
        }
    }

    fn end(&self) -> Position {
        Position {
            line: self.line,
            character: u32::MAX,
        }
    }

    fn severity(&self) -> DiagnosticSeverity {
        if self.count != 0 {
            DiagnosticSeverity::INFORMATION
        } else {
            DiagnosticSeverity::WARNING
        }
    }
}

impl FileCoverage {
    pub fn new(file: Url) -> Self {
        FileCoverage {
            uri: file,
            diag: Vec::with_capacity(64),
            is_hit: None,
        }
    }

    pub fn add(&mut self, data: &LineData) {
        let is_hit = data.count != 0;
        if self.is_hit.is_none_or(|prev_hit| prev_hit != is_hit) {
            self.diag.push(Diagnostic::new(
                data.range(),
                Some(data.severity()),
                None,
                Some(LSP_NAME.into()),
                (if is_hit {
                    "line covered"
                } else {
                    "lines not covered"
                })
                .into(),
                None,
                None,
            ));
            self.is_hit = Some(is_hit);
        } else {
            self.diag.last_mut().unwrap().range.end = data.end();
        }
    }
}

impl From<FileCoverage> for WorkspaceFullDocumentDiagnosticReport {
    fn from(value: FileCoverage) -> Self {
        WorkspaceFullDocumentDiagnosticReport {
            // FIXME
            uri: value.uri,
            version: None,
            full_document_diagnostic_report: FullDocumentDiagnosticReport {
                result_id: None,
                items: value.diag,
            },
        }
    }
}
