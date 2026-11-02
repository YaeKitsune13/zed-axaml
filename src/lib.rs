use zed_extension_api::{self as zed, LanguageServerId, Result};

struct AxamlExtension;

impl zed::Extension for AxamlExtension {
    fn new() -> Self {
        Self
    }

    fn language_server_command(
        &mut self,
        _language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        let path = worktree
            .which("axaml-ls")
            .ok_or_else(|| "axaml-ls не найден в PATH".to_string())?;

        Ok(zed::Command {
            command: path,
            args: vec![],
            env: worktree.shell_env(),
        })
    }
}

zed::register_extension!(AxamlExtension);
