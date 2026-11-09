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

use lcov::{Reader as LCOVReader, Record};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{ColorInformation, Url};

use crate::{FileCoverage, make_error};

pub(crate) mod go;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Lcov,
    Go,
}

#[derive(Debug)]
pub struct CoverageReport {
    pub path: PathBuf,
    pub mtime: SystemTime,
    pub id: String,
    format: Format,

    pub db: HashMap<Url, FileCoverage>,
}

impl TryFrom<PathBuf> for CoverageReport {
    type Error = std::io::Error;

    fn try_from(path: PathBuf) -> std::result::Result<Self, Self::Error> {
        let mtime = path.metadata()?.modified()?;
        let format = if go::is_profile(&path) {
            Format::Go
        } else {
            Format::Lcov
        };
        Ok(Self {
            format,
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
        self.mtime = self
            .path
            .metadata()
            .and_then(|m| m.modified())
            .map_err(|err| make_error(format!("failed update metadata: {err:?}")))?;
        if self.format == Format::Go {
            return self.load_go(root_uri);
        }
        let parser = match LCOVReader::open_file(&self.path) {
            Ok(parser) => parser,
            Err(err) => {
                tracing::error!(?err, file = ?self.path, "parsing error");
                return Err(make_error(format!("failed to parse: {err:?}")));
            }
        };

        let mut file: Option<FileCoverage> = None;
        for parser_result in parser {
            let record = match parser_result {
                Ok(record) => record,
                Err(err) => {
                    tracing::error!(?err, file = ?self.path, "parsing error");
                    continue;
                }
            };

            match record {
                Record::SourceFile { path } => {
                    if let Some(cov) = file.take() {
                        tracing::info!(file = cov.uri.as_str(), "loaded");
                        self.db.insert(cov.uri.clone(), cov);
                    }
                    let url = match root_uri.join(&path.to_string_lossy()) {
                        Ok(url) => url,
                        Err(err) => {
                            tracing::error!(?err, "failed to make Url from {path:?}");
                            continue;
                        }
                    };
                    file = Some(FileCoverage::new(url));
                }
                Record::LineData { line, count, .. } => {
                    if let Some(cov) = file.as_mut() {
                        cov.add(line.saturating_sub(1), count);
                    }
                }
                _ => (),
            }
        }
        if let Some(cov) = file.take() {
            tracing::info!(file = cov.uri.as_str(), "loaded");
            self.db.insert(cov.uri.clone(), cov);
        }
        Ok(())
    }

    pub fn create_document_color(&self, uri: &Url) -> Vec<ColorInformation> {
        match self.db.get(uri) {
            Some(report) => report.create_document_color(),
            None => Vec::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Workspace(PathBuf);

    impl Workspace {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("coverage-lsp-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(dir.join("pkg")).unwrap();
            Self(dir)
        }

        fn root_uri(&self) -> Url {
            Url::from_directory_path(&self.0).unwrap()
        }

        fn load(&self, content: &str) -> CoverageReport {
            let profile = self.0.join("coverage.out");
            fs::write(&profile, content).unwrap();
            let mut report = CoverageReport::try_from(profile).unwrap();
            report.load(&self.root_uri()).unwrap();
            report
        }

        fn lines(&self, report: &CoverageReport, file: &str) -> Vec<(u32, u64)> {
            let uri = self.root_uri().join(file).unwrap();
            let mut lines: Vec<_> = report.db[&uri]
                .coverage
                .iter()
                .map(|c| (c.line, c.count))
                .collect();
            lines.sort();
            lines
        }
    }

    impl Drop for Workspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn go_profile_expands_blocks_to_lines() {
        let ws = Workspace::new("blocks");
        fs::write(ws.0.join("pkg/a.go"), "package pkg\n").unwrap();
        let report = ws.load(
            "mode: set\n\
             example.com/mod/pkg/a.go:3.10,5.2 2 1\n\
             example.com/mod/pkg/a.go:7.2,8.3 1 0\n",
        );
        assert_eq!(
            ws.lines(&report, "pkg/a.go"),
            vec![(2, 1), (3, 1), (4, 1), (6, 0), (7, 0)]
        );
    }

    #[test]
    fn go_profile_prefers_hit_when_blocks_share_a_line() {
        let ws = Workspace::new("shared");
        fs::write(ws.0.join("pkg/a.go"), "package pkg\n").unwrap();
        let report = ws.load(
            "mode: count\n\
             example.com/mod/pkg/a.go:3.10,4.2 1 5\n\
             example.com/mod/pkg/a.go:4.2,6.2 1 0\n",
        );
        assert_eq!(
            ws.lines(&report, "pkg/a.go"),
            vec![(2, 5), (3, 5), (4, 0), (5, 0)]
        );
    }

    #[test]
    fn go_profile_without_matching_file_keeps_full_path() {
        let ws = Workspace::new("missing");
        let report = ws.load("mode: atomic\nexample.com/mod/pkg/a.go:3.1,3.5 1 1\n");
        assert_eq!(ws.lines(&report, "example.com/mod/pkg/a.go"), vec![(2, 1)]);
    }

    #[test]
    fn go_profile_skips_malformed_lines() {
        let ws = Workspace::new("malformed");
        fs::write(ws.0.join("pkg/a.go"), "package pkg\n").unwrap();
        let report = ws.load(
            "mode: set\n\
             garbage\n\
             example.com/mod/pkg/a.go:x.1,3.5 1 1\n\
             example.com/mod/pkg/a.go:3.1,3.5 1 1\n",
        );
        assert_eq!(ws.lines(&report, "pkg/a.go"), vec![(2, 1)]);
    }

    #[test]
    fn lcov_still_loads() {
        let ws = Workspace::new("lcov");
        let report = ws.load("TN:\nSF:src/a.rs\nDA:3,1\nDA:4,0\nend_of_record\n");
        assert_eq!(ws.lines(&report, "src/a.rs"), vec![(2, 1), (3, 0)]);
    }

    #[test]
    fn format_is_detected_once_at_creation() {
        let ws = Workspace::new("format");
        let go = ws.0.join("cover.out");
        fs::write(&go, "mode: set\n").unwrap();
        assert_eq!(CoverageReport::try_from(go).unwrap().format, Format::Go);

        let lcov = ws.0.join("lcov.info");
        fs::write(&lcov, "TN:\n").unwrap();
        let mut report = CoverageReport::try_from(lcov.clone()).unwrap();
        assert_eq!(report.format, Format::Lcov);

        fs::write(&lcov, "mode: set\n").unwrap();
        report.load(&ws.root_uri()).unwrap();
        assert_eq!(report.format, Format::Lcov);
    }
}
