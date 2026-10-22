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
** Created on: 2026-03-06T17:28:54
** Author: Sylvain Fargier <fargier.sylvain@gmail.com>
*/

use std::{
    collections::HashSet,
    ffi::OsString,
    ops::Deref,
    path::PathBuf,
    sync::{Arc, RwLock},
    time::Duration,
};

use ::serde::{Deserialize, Serialize};
use tower_lsp::lsp_types::Color;

mod serde;

static LSP_SETTINGS: RwLock<Settings> = RwLock::new(Settings::new());
const DEFAULT_DIRS_BLACKLIST: &[&str] = &[
    "node_modules",
    "deps",
    "incremental",
    "bin",
    "include",
    "lib",
];
const DEFAULT_MAX_DEPTH: usize = 3;
const DEFAULT_IGNORE_HIDDEN_DIRS: bool = true;
const DEFAULT_INTERVAL: Duration = Duration::from_secs(3);
const DEFAULT_CRAWL_INTERVAL: Duration = Duration::from_secs(15);

fn default_blacklist() -> Option<Arc<HashSet<OsString>>> {
    Some(Arc::new(HashSet::from_iter(
        DEFAULT_DIRS_BLACKLIST.iter().map(|&v| OsString::from(v)),
    )))
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// Display color for matched lines
    #[serde(with = "serde")]
    pub hit: Option<Color>,
    /// Display color un-matched (missed) lines
    #[serde(with = "serde")]
    pub miss: Option<Color>,
    /// Coverage report file-checking interval
    #[serde(with = "humantime_serde")]
    pub interval: Duration,
    /// Coverage report file to load
    pub lcov_file: Option<Arc<PathBuf>>,
    /// Directories to ignore during crawl
    #[serde(default = "default_blacklist")]
    pub dirs_blacklist: Option<Arc<HashSet<OsString>>>,
    /// Ignore directories starting with "." during crawl
    pub ignore_hidden_dirs: bool,
    /// Maximum directory depth during crawl
    pub max_depth: usize,
    /// Crawling interval (more CPU intensive than watching a single file)
    pub crawl_interval: Duration,
}

impl Default for Settings {
    fn default() -> Self {
        Self::new()
    }
}

impl Settings {
    pub const fn new() -> Self {
        Self {
            hit: Some(Color {
                red: 0.0,
                green: 1.0,
                blue: 0.0,
                alpha: 0.1,
            }),
            miss: Some(Color {
                red: 1.0,
                green: 0.0,
                blue: 0.0,
                alpha: 0.1,
            }),
            interval: DEFAULT_INTERVAL,
            lcov_file: None,
            dirs_blacklist: None,
            ignore_hidden_dirs: DEFAULT_IGNORE_HIDDEN_DIRS,
            max_depth: DEFAULT_MAX_DEPTH,
            crawl_interval: DEFAULT_CRAWL_INTERVAL,
        }
    }

    pub fn get() -> impl Deref<Target = Self> {
        LSP_SETTINGS.read().unwrap()
    }

    pub fn set(settings: Settings) {
        *LSP_SETTINGS.write().unwrap() = settings;
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, sync::Arc};

    use super::Settings;

    #[test]
    fn serde() -> serde_json::Result<()> {
        let settings: Settings = serde_json::from_str(
            r#"{ "hit": null, "miss": "red", "interval": "20s", "lcov_file": "./lcov.info" }"#,
        )?;
        let settings: Settings = serde_json::from_str(serde_json::to_string(&settings)?.as_str())?;
        assert_eq!(
            settings.lcov_file,
            Some(Arc::new(PathBuf::from("./lcov.info")))
        );
        assert_eq!(settings.interval, std::time::Duration::from_secs(20));

        serde_json::from_str::<Settings>("{}")?;
        Ok(())
    }
}
