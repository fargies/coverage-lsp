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
use std::{
    collections::{BTreeMap, HashMap},
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    time::SystemTime,
};

use lcov::{Reader as LCOVReader, Record};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{ColorInformation, Url};

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
        self.mtime = self
            .path
            .metadata()
            .and_then(|m| m.modified())
            .map_err(|err| make_error(format!("failed update metadata: {err:?}")))?;
        if is_go_profile(&self.path) {
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

    /// Load a Go cover profile (`go test -coverprofile=coverage.out`)
    fn load_go(&mut self, root_uri: &Url) -> Result<()> {
        let content = std::fs::read_to_string(&self.path)
            .map_err(|err| make_error(format!("failed to read: {err:?}")))?;
        let root = root_uri.to_file_path().unwrap_or_default();

        let mut files: HashMap<&str, BTreeMap<u32, u64>> = HashMap::new();
        for line in content.lines().skip(1) {
            let Some((name, start, end, count)) = parse_go_block(line) else {
                tracing::warn!(line, "malformed go coverage line");
                continue;
            };
            let lines = files.entry(name).or_default();
            for l in start..=end {
                let entry = lines.entry(l.saturating_sub(1)).or_insert(0);
                *entry = (*entry).max(count);
            }
        }

        for (name, lines) in files {
            let path = resolve_go_path(&root, name);
            let url = match root_uri.join(&path.to_string_lossy()) {
                Ok(url) => url,
                Err(err) => {
                    tracing::error!(?err, "failed to make Url from {path:?}");
                    continue;
                }
            };
            let mut cov = FileCoverage::new(url);
            for (line, count) in lines {
                cov.add(line, count);
            }
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

fn is_go_profile(path: &Path) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let mut first_line = String::new();
    BufReader::new(file).read_line(&mut first_line).is_ok() && first_line.starts_with("mode:")
}

/// Parse `name.go:startLine.startCol,endLine.endCol numStmts count`
fn parse_go_block(line: &str) -> Option<(&str, u32, u32, u64)> {
    let (name, rest) = line.rsplit_once(':')?;
    let (range, rest) = rest.split_once(' ')?;
    let (_, count) = rest.split_once(' ')?;
    let (start, end) = range.split_once(',')?;
    let start = start.split_once('.')?.0.parse().ok()?;
    let end = end.split_once('.')?.0.parse().ok()?;
    Some((name, start, end, count.trim().parse().ok()?))
}

/// Go profiles use import paths (`example.com/mod/pkg/a.go`): find the longest
/// suffix that exists in the workspace.
fn resolve_go_path(root: &Path, name: &str) -> PathBuf {
    let mut rest = name;
    loop {
        if root.join(rest).is_file() {
            return PathBuf::from(rest);
        }
        match rest.split_once('/') {
            Some((_, tail)) => rest = tail,
            None => return PathBuf::from(name),
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
}
