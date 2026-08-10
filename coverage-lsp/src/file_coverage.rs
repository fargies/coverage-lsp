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

use tower_lsp::lsp_types::{ColorInformation, Position, Range, Url};

use crate::Settings;

#[derive(Debug)]
pub struct LineCoverageInfo {
    pub line: u32,
    pub count: u64,
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

    pub fn add(&mut self, line: u32, count: u64) {
        self.coverage.push(LineCoverageInfo { line, count });
    }

    pub fn create_document_color(&self) -> Vec<ColorInformation> {
        let mut ret = Vec::with_capacity(self.coverage.len());
        let (hit_color, miss_color) = {
            let settings = Settings::get();
            (settings.hit, settings.miss)
        };
        for cov in self.coverage.iter() {
            if cov.count != 0
                && let Some(color) = hit_color
            {
                ret.push(ColorInformation {
                    range: cov.range(),
                    color,
                });
            } else if cov.count == 0
                && let Some(color) = miss_color
            {
                ret.push(ColorInformation {
                    range: cov.range(),
                    color,
                });
            }
        }
        ret
    }
}
