use std::fs;
use std::path::PathBuf;
use zed_extension_api::{self as zed, LanguageServerId, Result};

struct AxamlExtension {
    cached_binary_path: Option<String>,
}

impl AxamlExtension {
    fn language_server_binary(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<(String, Option<String>)> {
        let local_meta = worktree
            .shell_env()
            .into_iter()
            .find(|(k, _)| k == "HOME")
            .map(|(_, h)| format!("{h}/.local/share/axaml-ls/meta.json"))
            .filter(|p| fs::metadata(p).is_ok());

        // 1. Locally installed binary in PATH (useful for development)
        if let Some(path) = worktree.which("axaml-ls") {
            return Ok((path, local_meta));
        }

        // 2. Already downloaded binary in extension working directory
        if let Some(path) = &self.cached_binary_path {
            if fs::metadata(path).map_or(false, |m| m.is_file()) {
                let meta = fs::metadata("meta.json")
                    .ok()
                    .and_then(|_| std::env::current_dir().ok())
                    .map(|d| d.join("meta.json").to_string_lossy().to_string())
                    .or(local_meta);
                return Ok((path.clone(), meta));
            }
        }

        // 3. Download from GitHub Releases
        zed::set_language_server_installation_status(
            language_server_id,
            &zed::LanguageServerInstallationStatus::CheckingForUpdate,
        );

        let release = zed::latest_github_release(
            "YaeKitsune13/zed-axaml",
            zed::GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        )?;

        let (platform, arch) = zed::current_platform();
        let target = match (platform, arch) {
            (zed::Os::Linux, zed::Architecture::X8664) => "x86_64-unknown-linux-gnu",
            (zed::Os::Linux, zed::Architecture::Aarch64) => "aarch64-unknown-linux-gnu",
            (zed::Os::Mac, zed::Architecture::X8664) => "x86_64-apple-darwin",
            (zed::Os::Mac, zed::Architecture::Aarch64) => "aarch64-apple-darwin",
            (zed::Os::Windows, zed::Architecture::X8664) => "x86_64-pc-windows-msvc",
            _ => return Err(format!("Unsupported platform: {platform:?} {arch:?}")),
        };

        let archive_ext = match platform {
            zed::Os::Windows => "zip",
            _ => "tar.gz",
        };
        let asset_name = format!("axaml-ls-{target}.{archive_ext}");

        let asset = release
            .assets
            .iter()
            .find(|a| a.name == asset_name)
            .ok_or_else(|| format!("Release {} does not contain archive {asset_name}", release.version))?;

        let version_dir = format!("axaml-ls-{}", release.version);
        let binary_name = match platform {
            zed::Os::Windows => "axaml-ls.exe",
            _ => "axaml-ls",
        };
        let binary_path = format!("{version_dir}/{binary_name}");

        if !fs::metadata(&binary_path).map_or(false, |m| m.is_file()) {
            zed::set_language_server_installation_status(
                language_server_id,
                &zed::LanguageServerInstallationStatus::Downloading,
            );

            let file_type = match platform {
                zed::Os::Windows => zed::DownloadedFileType::Zip,
                _ => zed::DownloadedFileType::GzipTar,
            };

            zed::download_file(&asset.download_url, &version_dir, file_type)
                .map_err(|e| format!("Failed to download axaml-ls: {e}"))?;
            zed::make_file_executable(&binary_path)?;

            // Clean up old versions
            if let Ok(entries) = fs::read_dir(".") {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    if let Some(s) = name.to_str() {
                        if s.starts_with("axaml-ls-") && s != version_dir {
                            let _ = fs::remove_dir_all(entry.path());
                        }
                    }
                }
            }
        }

        // Download meta.json from release if not present locally
        let mut resolved_meta = local_meta;
        if resolved_meta.is_none() {
            let meta_file = "meta.json";
            if !fs::metadata(meta_file).map_or(false, |m| m.is_file()) {
                if let Some(meta_asset) = release.assets.iter().find(|a| a.name == "meta.json") {
                    let _ = zed::download_file(
                        &meta_asset.download_url,
                        meta_file,
                        zed::DownloadedFileType::Uncompressed,
                    );
                }
            }
            if fs::metadata(meta_file).map_or(false, |m| m.is_file()) {
                if let Ok(cwd) = std::env::current_dir() {
                    resolved_meta = Some(PathBuf::from(cwd).join(meta_file).to_string_lossy().to_string());
                }
            }
        }

        self.cached_binary_path = Some(binary_path.clone());
        Ok((binary_path, resolved_meta))
    }
}

impl zed::Extension for AxamlExtension {
    fn new() -> Self {
        Self {
            cached_binary_path: None,
        }
    }

    fn language_server_command(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        let (command, meta_path) = self.language_server_binary(language_server_id, worktree)?;
        let mut env = worktree.shell_env();
        if let Some(meta) = meta_path {
            env.push(("AXAML_META".to_string(), meta));
        }

        Ok(zed::Command {
            command,
            args: vec![],
            env,
        })
    }
}

zed::register_extension!(AxamlExtension);
