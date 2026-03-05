use std::fs;
use zed_extension_api::{self as zed, Result};

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
}

zed::register_extension!(CoverageExtension);
