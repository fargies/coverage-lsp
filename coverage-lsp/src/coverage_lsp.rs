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

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use serde_json::Value;
use tokio::{
    sync::RwLock,
    task::{self, JoinHandle},
};
use tower_lsp::lsp_types::{ConfigurationItem, Position, Range, TextEdit, WorkspaceEdit};
use tower_lsp::{
    Client,
    lsp_types::{MessageType, Url},
};

use crate::CoverageReport;

#[derive(Debug)]
pub struct CoverageLanguageServer {
    pub context: Arc<CoverageLanguageServerContext>,
}

impl CoverageLanguageServer {
    pub fn new(client: Client) -> Self {
        Self {
            context: CoverageLanguageServerContext::new(client),
        }
    }
}

impl std::ops::Deref for CoverageLanguageServer {
    type Target = CoverageLanguageServerContext;

    fn deref(&self) -> &Self::Target {
        &self.context
    }
}

#[derive(Debug)]
pub struct CoverageLanguageServerContext {
    pub client: Client,
    pub root_uri: RwLock<Url>,
    pub report: RwLock<Option<CoverageReport>>,
    pub open_docs: RwLock<HashSet<Url>>,
    join_handle: Mutex<Option<JoinHandle<()>>>,
}

impl CoverageLanguageServerContext {
    pub fn new(client: Client) -> Arc<Self> {
        Arc::new(Self {
            client,
            root_uri: RwLock::new(
                Url::from_directory_path(std::env::current_dir().unwrap()).unwrap(),
            ),
            report: Default::default(),
            open_docs: Default::default(),
            join_handle: Default::default(),
        })
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
            join_handle.await.unwrap();
        }
    }

    async fn run(weak: Weak<Self>) {
        while let Some(ctx) = weak.upgrade() {
            ctx.update().await;

            drop(ctx);
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    }

    pub async fn update(&self) {
        let file = match self.report.read().await.as_ref() {
            Some(report) if report.is_outdated() => Some(report.path.clone()),
            Some(_) => None,
            None => self.find_lcov_file().await,
        };
        if let Some(file) = file {
            self.client
                .log_message(MessageType::INFO, format!("(re)loading file: {file:?}"))
                .await;
            let mut report = match CoverageReport::try_from(file) {
                Ok(report) => report,
                Err(err) => {
                    self.client
                        .log_message(
                            MessageType::ERROR,
                            format!("failed to load report: {err:?}"),
                        )
                        .await;
                    self.report.write().await.take();

                    #[cfg(feature = "notifications")]
                    self.send_update_notification(true).await;
                    return;
                }
            };
            let root_uri = self.root_uri.read().await.clone();

            if let Err(err) = report.load(&root_uri) {
                self.client
                    .log_message(
                        MessageType::ERROR,
                        format!("failed to parse report: {err:?}"),
                    )
                    .await
            } else {
                self.report.write().await.replace(report);
                #[cfg(feature = "notifications")]
                self.send_update_notification(false).await;
            }
        }
    }

    /// Edit opened documents to trigger a coloration update
    #[cfg(feature = "notifications")]
    pub async fn send_update_notification(&self, forced: bool) {
        let opened = self.open_docs.read().await.clone();
        let mut changes = HashMap::with_capacity(1);
        let edit = Vec::from([TextEdit {
            range: Range::new(Position::new(0, 0), Position::new(0, 0)),
            new_text: " ".into(),
        }]);

        // if we update all docs at once, zed will open an "LSP Edits" tab
        // notifying editors one by ones silences it.
        for doc in opened.into_iter() {
            if !forced && self
                .report
                .read()
                .await
                .as_ref()
                .is_none_or(|report| !report.db.contains_key(&doc))
            {
                continue;
            }
            changes.clear();
            changes.insert(doc.clone(), edit.clone());
            if let Err(err) = self
                .client
                .apply_edit(WorkspaceEdit {
                    changes: Some(changes.clone()),
                    ..Default::default()
                })
                .await
            {
                tracing::error!(?err, "WorkspaceEdit error");
                return;
            }

            for (_, change) in changes.iter_mut() {
                let text_edit = change.first_mut().unwrap();
                text_edit.range.end.character = 1;
                text_edit.new_text = String::default();
            }
            /* for some reason if we send both edits at once the delete is done before write */
            if let Err(err) = self
                .client
                .apply_edit(WorkspaceEdit {
                    changes: Some(changes.clone()),
                    ..Default::default()
                })
                .await
            {
                tracing::error!(?err, "WorkspaceEdit error");
                return;
            }
        }
    }

    /// Crawl the workspace to find an '*.info' file
    pub async fn find_lcov_file(&self) -> Option<PathBuf> {
        self.client
            .log_message(MessageType::INFO, "crawling for coverage file")
            .await;
        let mut dir_stack = VecDeque::with_capacity(64);
        let root_path = self.root_uri.read().await.path().to_string();
        dir_stack.push_back(PathBuf::from(&root_path));

        while let Some(path) = dir_stack.pop_front() {
            let mut reader = match tokio::fs::read_dir(path).await.ok() {
                Some(reader) => reader,
                None => {
                    self.client
                        .log_message(MessageType::WARNING, "failed to read_dir: {path:?}")
                        .await;
                    continue;
                }
            };
            while let Ok(Some(entry)) = reader.next_entry().await {
                let path = entry.path();
                if path.is_dir() {
                    dir_stack.push_back(path);
                } else if path.extension().is_some_and(|ext| ext == "info") {
                    self.client
                        .show_message(
                            MessageType::INFO,
                            format!(
                                "coverage file found: {:?}",
                                path.strip_prefix(&root_path).unwrap_or(&path)
                            ),
                        )
                        .await;
                    return Some(path);
                }
            }
        }
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

impl Drop for CoverageLanguageServerContext {
    fn drop(&mut self) {
        if let Some(join_handle) = self.join_handle.lock().unwrap().take() {
            join_handle.abort();
        }
    }
}
