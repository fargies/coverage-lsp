/*
** Created on: 2026-03-06T16:48:45
** Author: Sylvain Fargier <fargier.sylvain@gmail.com>
** Original-Author: https://github.com/huacnlee
** Derivative work from: https://github.com/huacnlee/color-lsp/blob/main/zed-color-highlight/src/lib.rs
*/

use serde_json::{Value, json};
use std::fs;
use zed_extension_api::{self as zed, Result, settings::LspSettings};

const GITHUB_REPO: &str = "fargies/coverage-lsp";

enum Status {
    None,
    Downloading,
    Failed(String),
}

fn update_status(id: &zed::LanguageServerId, status: Status) {
    match status {
        Status::None => zed::set_language_server_installation_status(
            id,
            &zed::LanguageServerInstallationStatus::None,
        ),
        Status::Downloading => zed::set_language_server_installation_status(
            id,
            &zed::LanguageServerInstallationStatus::Downloading,
        ),
        Status::Failed(msg) => zed::set_language_server_installation_status(
            id,
            &zed::LanguageServerInstallationStatus::Failed(msg),
        ),
    }
}

struct CoverageExtension {}

impl CoverageExtension {
    fn language_server_binary_path(
        &mut self,
        id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<String> {
        let bin_name = match zed::current_platform() {
            (zed_extension_api::Os::Windows, _) => format!("{id}.exe"),
            _ => id.to_string(),
        };

        if let Some(path) = worktree.which(bin_name.as_str()) {
            return Ok(path);
        }

        if let Some(binary_path) = Self::check_installed(&bin_name) {
            // silent to check for update.
            let _ = Self::check_to_update(id, &bin_name);
            return Ok(binary_path);
        }

        let binary_path = Self::check_to_update(id, &bin_name)?;
        Ok(binary_path)
    }

    fn check_installed<S>(bin_name: S) -> Option<String>
    where
        S: AsRef<str>,
    {
        let entries = fs::read_dir(".").ok()?;
        for entry in entries.flatten().filter(|entry| entry.path().is_dir()) {
            let binary_path = entry.path().join(bin_name.as_ref());
            if fs::metadata(&binary_path).is_ok_and(|stat| stat.is_file()) {
                return binary_path.to_str().map(|s| s.to_string());
            }
        }
        None
    }

    fn check_to_update<S>(id: &zed::LanguageServerId, bin_name: S) -> Result<String>
    where
        S: AsRef<str>,
    {
        let (platform, arch) = zed::current_platform();
        let release = zed::latest_github_release(
            GITHUB_REPO,
            zed::GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        )?;

        let asset_name = format!(
            "{id}-{os}-{arch}.{ext}",
            id = id,
            arch = match arch {
                zed::Architecture::Aarch64 => "arm64",
                zed::Architecture::X86 => "amd64",
                zed::Architecture::X8664 => "amd64",
            },
            os = match platform {
                zed::Os::Mac => "darwin",
                zed::Os::Linux => "linux",
                zed::Os::Windows => "windows",
            },
            ext = match platform {
                zed::Os::Windows => "zip",
                _ => "tar.gz",
            }
        );

        let file_type = match platform {
            zed::Os::Windows => zed::DownloadedFileType::Zip,
            _ => zed::DownloadedFileType::GzipTar,
        };

        let version_dir = format!("{id}-{version}", id = id, version = release.version);
        let version_binary_path = format!("{version_dir}/{}", bin_name.as_ref());

        if !fs::metadata(&version_binary_path).is_ok_and(|stat| stat.is_file()) {
            update_status(id, Status::Downloading);

            let asset = release
                .assets
                .iter()
                .find(|asset| asset.name == asset_name)
                .ok_or_else(|| format!("no asset found matching {:?}", asset_name))?;
            zed::download_file(&asset.download_url, &version_dir, file_type)
                .map_err(|e| format!("failed to download file: {e}"))?;

            let entries =
                fs::read_dir(".").map_err(|e| format!("failed to list working directory {e}"))?;
            for entry in entries {
                let entry = entry.map_err(|e| format!("failed to load directory entry {e}"))?;
                if entry.file_name().to_str() != Some(&version_dir) {
                    fs::remove_dir_all(entry.path()).ok();
                }
            }

            update_status(id, Status::None);
        }

        Ok(version_binary_path)
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
        let command = self
            .language_server_binary_path(id, worktree)
            .inspect_err(|err| {
                update_status(id, Status::Failed(err.to_string()));
            })?;

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
