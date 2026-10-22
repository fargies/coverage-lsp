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
** Created on: 2026-03-06T09:02:59
** Author: Sylvain Fargier <fargier.sylvain@gmail.com>
*/

use std::collections::{HashSet, VecDeque};
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::SystemTime;

use serde_json::Value;
use tokio::{
    sync::RwLock,
    task::{self, JoinHandle},
};
use tower_lsp::lsp_types::{ConfigurationItem, Registration, Unregistration};
use tower_lsp::{Client, lsp_types::Url};

use crate::{CoverageReport, Settings, logger::LogPump};

#[derive(Debug)]
pub struct CoverageLanguageServer {
    pub internal: Arc<CoverageLanguageServerContext>,
}

impl CoverageLanguageServer {
    pub fn new(client: Client) -> Self {
        Self {
            internal: CoverageLanguageServerContext::new(client),
        }
    }

    pub fn start(&self) {
        self.internal.start()
    }

    pub fn stop(&self) -> impl Future<Output = ()> {
        self.internal.stop()
    }
}

impl std::ops::Deref for CoverageLanguageServer {
    type Target = CoverageLanguageServerContext;

    fn deref(&self) -> &Self::Target {
        &self.internal
    }
}

#[derive(Debug)]
pub struct CoverageLanguageServerContext {
    pub client: Client,
    pub root_uri: RwLock<Url>,
    pub report: RwLock<Option<CoverageReport>>,
    pub open_docs: RwLock<HashSet<Url>>,
    pub state: Mutex<Option<State>>,
    join_handle: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Debug, Default)]
pub struct State {
    pub is_registered: bool,
    pub has_dyn_color_provider: bool,
    pub last_crawl: Option<SystemTime>,
}

impl State {
    /// Return true if [path] is newer than [self.last_crawl]
    pub fn is_path_newer(&self, path: &Path) -> bool {
        match self.last_crawl.as_ref() {
            Some(mtime) => match path.metadata().and_then(|m| m.modified()) {
                Ok(path_mtime) => &path_mtime > mtime,
                Err(_) => true,
            },
            None => true,
        }
    }
}

struct ContextGuard<'a>(Option<State>, &'a CoverageLanguageServerContext);

impl<'a> ContextGuard<'a> {
    pub fn new(ls: &'a CoverageLanguageServerContext) -> Option<Self> {
        ls.state
            .lock()
            .unwrap()
            .take()
            .map(|context| Self(Some(context), ls))
    }
}

impl Deref for ContextGuard<'_> {
    type Target = State;

    fn deref(&self) -> &Self::Target {
        self.0.as_ref().unwrap()
    }
}

impl DerefMut for ContextGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.as_mut().unwrap()
    }
}

impl Drop for ContextGuard<'_> {
    fn drop(&mut self) {
        self.1.state.lock().unwrap().replace(self.0.take().unwrap());
    }
}

impl CoverageLanguageServerContext {
    pub fn new(client: Client) -> Arc<Self> {
        let ret = Arc::new(Self {
            client,
            root_uri: RwLock::new(
                Url::from_directory_path(std::env::current_dir().unwrap()).unwrap(),
            ),
            report: Default::default(),
            open_docs: Default::default(),
            state: Mutex::new(Some(State::default())),
            join_handle: Default::default(),
        });
        LogPump::set_service(Arc::downgrade(&ret));
        ret
    }

    pub fn start(self: &Arc<Self>) {
        self.join_handle.lock().unwrap().get_or_insert_with(|| {
            let weak = Arc::downgrade(self);
            task::spawn(CoverageLanguageServerContext::run(weak))
        });
    }

    pub async fn stop(self: &Arc<Self>) {
        let join_handle = self.join_handle.lock().unwrap().take();
        if let Some(join_handle) = join_handle {
            join_handle.abort();
            if let Err(err) = join_handle.await {
                tracing::error!(?err, "parsing thread join error");
            }
        }
    }

    async fn run(weak: Weak<Self>) {
        while let Some(ctx) = weak.upgrade() {
            let has_report = ctx.update().await;

            drop(ctx);
            let interval = if has_report {
                Settings::get().interval
            } else {
                Settings::get().crawl_interval
            };
            tokio::time::sleep(interval).await;
        }
    }

    /// get file to parse
    ///
    /// returns [None] if there's nothing to parse or if the report is already
    /// up to date.
    async fn get_update_file(&self, state: &mut State) -> Option<PathBuf> {
        let setting_file_guard = Settings::get().lcov_file.as_ref().map(Arc::clone);
        let setting_file = setting_file_guard.as_ref().map(|v| v.as_ref());
        match self.report.read().await.as_ref() {
            Some(report)
                if setting_file.is_some_and(|setting_file| setting_file != &report.path) =>
            {
                /* lcov_file in settings has been updated */
                Some(setting_file.unwrap().clone())
            }
            Some(report) if report.is_outdated() => Some(report.path.clone()),
            Some(_) => None,
            None => match setting_file {
                Some(file) => Some(file.clone()),
                None => self.find_lcov_file(state).await,
            },
        }
    }

    /// Update report if necessary
    ///
    /// Will crawl to find a file, and re-use the same unless settings changes
    /// once found
    ///
    /// Returns true if a report is valid and available
    pub async fn update(&self) -> bool {
        let mut context = match ContextGuard::new(self) {
            Some(ctx) => ctx,
            None => {
                tracing::warn!("no context to run update");
                return false;
            }
        };
        if let Some(file) = self.get_update_file(&mut context).await {
            tracing::info!(?file, "(re)loading file");
            if context.is_registered && context.has_dyn_color_provider {
                if let Err(err) = self
                    .client
                    .unregister_capability(
                        [Unregistration {
                            id: "coverage-lsp".into(),
                            method: "textDocument/documentColor".into(),
                        }]
                        .into(),
                    )
                    .await
                {
                    tracing::warn!(
                        "failed to unregister [textDocument/documentColor] service: {err:?}"
                    );
                }
                context.is_registered = false;
            }
            let mut report = match CoverageReport::try_from(file) {
                Ok(report) => report,
                Err(err) => {
                    tracing::error!(?err, "failed to load report");
                    self.report.write().await.take();
                    return false;
                }
            };
            let root_uri = self.root_uri.read().await.clone();

            if let Err(err) = report.load(&root_uri) {
                tracing::error!(?err, "failed to parse report");
            } else {
                self.report.write().await.replace(report);
            }

            if !context.is_registered && context.has_dyn_color_provider {
                if let Err(err) = self
                    .client
                    .register_capability(
                        [Registration {
                            id: "coverage-lsp".into(),
                            method: "textDocument/documentColor".into(),
                            register_options: None,
                        }]
                        .into(),
                    )
                    .await
                {
                    tracing::warn!(
                        "failed to register [textDocument/documentColor] service: {err:?}"
                    );
                }
                context.is_registered = true;
            }
            true
        } else {
            self.report.read().await.is_some()
        }
    }

    /// Crawl the workspace to find an '*.info' file
    pub async fn find_lcov_file(&self, state: &mut State) -> Option<PathBuf> {
        tracing::debug!("crawling for coverage file");
        let timer = std::time::Instant::now();
        let now = SystemTime::now();
        let mut dir_stack = VecDeque::with_capacity(64);
        let root_path = PathBuf::from(self.root_uri.read().await.path().to_string());
        dir_stack.push_back((1usize, state.is_path_newer(&root_path), root_path.clone()));
        let (mut dirs_count, mut files_count) = (0usize, 0usize);
        let (blacklist, ignore_hidden_dirs, max_depth) = {
            let settings = Settings::get();
            (
                settings.dirs_blacklist.as_ref().map(Arc::clone),
                settings.ignore_hidden_dirs,
                settings.max_depth,
            )
        };

        while let Some((depth, is_path_newer, path)) = dir_stack.pop_front() {
            let mut reader = match tokio::fs::read_dir(path).await {
                Ok(reader) => reader,
                Err(err) => {
                    tracing::warn!(?err, "failed to read_dir");
                    continue;
                }
            };
            while let Ok(Some(entry)) = reader.next_entry().await {
                let file_type = match entry.file_type().await {
                    Ok(file_type) => file_type,
                    Err(_) => continue,
                };

                if file_type.is_dir() {
                    let file_name = entry.file_name();
                    if depth <= max_depth
                        && blacklist.as_ref().is_none_or(|b| !b.contains(&file_name))
                        && (!ignore_hidden_dirs
                            || file_name.as_encoded_bytes().first() != Some(&b'.'))
                    {
                        let path = entry.path();
                        dir_stack.push_back((depth + 1, state.is_path_newer(&path), path));
                        dirs_count += 1;
                    }
                } else if is_path_newer {
                    let file_name = entry.file_name();
                    let file_name = file_name.as_encoded_bytes();
                    files_count += 1;
                    if file_name.len() > 5 && &file_name[file_name.len() - 5..] == b".info" {
                        let path = entry.path();
                        tracing::info!(
                            file = ?path.strip_prefix(&root_path).unwrap_or(&path),
                            "coverage file found"
                        );
                        state.last_crawl = Some(now);
                        return Some(path);
                    }
                }
            }
        }
        state.last_crawl = Some(now);
        tracing::debug!(elapsed = ?timer.elapsed(), dirs = dirs_count, files = files_count, "no coverage file found");
        None
    }

    pub async fn get_configuration(&self) -> Option<Value> {
        self.client
            .configuration(vec![ConfigurationItem::default()])
            .await
            .ok()
            .and_then(|mut v| v.pop())
    }
}

impl Drop for CoverageLanguageServer {
    fn drop(&mut self) {
        if let Some(join_handle) = self.join_handle.lock().unwrap().take() {
            join_handle.abort();
        }
    }
}
