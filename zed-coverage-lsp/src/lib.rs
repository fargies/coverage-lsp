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
** Created on: 2026-03-06T16:48:45
** Author: Sylvain Fargier <fargier.sylvain@gmail.com>
*/

use serde_json::{Value, json};
use std::fs;
use zed_extension_api::{self as zed, Result, settings::LspSettings};

#[inline]
fn bin_name() -> &'static str {
    if zed::current_platform().0 == zed::Os::Windows {
        "coverage-lsp.exe"
    } else {
        "coverage-lsp"
    }
}

struct CoverageExtension {}

impl CoverageExtension {
    fn language_server_binary_path(
        &mut self,
        id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<String> {
        let bin_name = bin_name();
        // Check if the binary is already installed by manually checking the path
        if let Some(path) = worktree.which(bin_name) {
            return Ok(path);
        }

        if let Some(binary_path) = Self::check_installed() {
            // silent to check for update.
            Ok(binary_path)
        } else {
            Err("coverage-lsp binary not found".into())
        }
    }

    fn check_installed() -> Option<String> {
        let entries = fs::read_dir(".").ok()?;
        for entry in entries.flatten().filter(|entry| entry.path().is_dir()) {
            let binary_path = entry.path().join(bin_name());
            if fs::metadata(&binary_path).is_ok_and(|stat| stat.is_file()) {
                return binary_path.to_str().map(|s| s.to_string());
            }
        }
        None
    }
}

impl zed::Extension for CoverageExtension {
    fn new() -> Self {
        Self {}
    }

    fn language_server_command(
        &mut self,
        id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        let command = self.language_server_binary_path(id, worktree)?;

        Ok(zed::Command {
            command,
            args: vec![],
            env: Default::default(),
        })
    }

    fn language_server_workspace_configuration(
        &mut self,
        id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<Option<Value>> {
        let settings = LspSettings::for_worktree(id.as_ref(), worktree)
            .ok()
            .and_then(|lsp_settings| lsp_settings.settings)
            .unwrap_or_default();

        Ok(Some(json!({
            id.as_ref(): settings
        })))
    }
}

zed::register_extension!(CoverageExtension);
