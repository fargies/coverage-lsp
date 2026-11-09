/*
** Copyright (C) 2026 Beni Pranata
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
** Author: Beni Pranata
*/

use std::{
    collections::{BTreeMap, HashMap},
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::Url;

use super::CoverageReport;
use crate::{FileCoverage, make_error};

/// Default file names of Go cover profiles (see `go help testflag`)
pub(crate) fn is_profile_name(name: &[u8]) -> bool {
    name == b"coverage.out" || name == b"cover.out"
}

impl CoverageReport {
    /// Load a Go cover profile (`go test -coverprofile=coverage.out`)
    pub(super) fn load_go(&mut self, root_uri: &Url) -> Result<()> {
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
}

pub(super) fn is_profile(path: &Path) -> bool {
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

    #[test]
    fn profile_names() {
        assert!(is_profile_name(b"coverage.out"));
        assert!(is_profile_name(b"cover.out"));
        assert!(!is_profile_name(b"other.out"));
        assert!(!is_profile_name(b"xcover.out"));
    }
}
