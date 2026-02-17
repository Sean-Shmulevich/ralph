use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Top-level config file schema for `ralph.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RalphConfig {
    pub defaults: Option<DefaultsConfig>,
    pub hooks: Option<HooksConfig>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DefaultsConfig {
    pub agent: Option<String>,
    pub max_iterations: Option<u32>,
    pub timeout: Option<u64>,
    pub stall_timeout: Option<u64>,
    pub max_failures: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct HooksConfig {
    pub url: Option<String>,
    pub token: Option<String>,
}

pub fn load_config() -> Result<Option<RalphConfig>> {
    let cwd = std::env::current_dir().context("Cannot resolve current directory")?;
    load_config_from(&cwd, home_dir().as_deref())
}

fn load_config_from(cwd: &Path, home_dir: Option<&Path>) -> Result<Option<RalphConfig>> {
    let Some(path) = find_config_path(cwd, home_dir) else {
        return Ok(None);
    };

    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read config file {}", path.display()))?;
    let parsed = toml::from_str::<RalphConfig>(&raw)
        .with_context(|| format!("Failed to parse TOML config {}", path.display()))?;
    Ok(Some(parsed))
}

fn find_config_path(cwd: &Path, home_dir: Option<&Path>) -> Option<PathBuf> {
    let local = cwd.join("ralph.toml");
    if local.is_file() {
        return Some(local);
    }

    let home = home_dir?;
    let global = home.join(".config").join("ralph").join("config.toml");
    if global.is_file() {
        return Some(global);
    }

    None
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::{find_config_path, load_config_from};
    use tempfile::tempdir;

    #[test]
    fn prefers_local_ralph_toml_over_global_config() {
        let cwd = tempdir().expect("temp cwd");
        let home = tempdir().expect("temp home");

        let local_path = cwd.path().join("ralph.toml");
        std::fs::write(&local_path, "[defaults]\nagent = \"codex\"\n").expect("write local");

        let global_dir = home.path().join(".config").join("ralph");
        std::fs::create_dir_all(&global_dir).expect("create global dir");
        std::fs::write(
            global_dir.join("config.toml"),
            "[defaults]\nagent = \"gemini\"\n",
        )
        .expect("write global");

        let found =
            find_config_path(cwd.path(), Some(home.path())).expect("config path should exist");
        assert_eq!(found, local_path);
    }

    #[test]
    fn falls_back_to_global_config_when_local_missing() {
        let cwd = tempdir().expect("temp cwd");
        let home = tempdir().expect("temp home");

        let global_dir = home.path().join(".config").join("ralph");
        std::fs::create_dir_all(&global_dir).expect("create global dir");
        let global_path = global_dir.join("config.toml");
        std::fs::write(&global_path, "[defaults]\nmax_iterations = 42\n").expect("write global");

        let found =
            find_config_path(cwd.path(), Some(home.path())).expect("config path should exist");
        assert_eq!(found, global_path);
    }

    #[test]
    fn parses_defaults_and_hooks_from_toml() {
        let cwd = tempdir().expect("temp cwd");
        std::fs::write(
            cwd.path().join("ralph.toml"),
            r#"
[defaults]
agent = "codex"
max_iterations = 20
timeout = 600
stall_timeout = 120
max_failures = 3

[hooks]
url = "https://example.com/webhook"
token = "secret"
"#,
        )
        .expect("write config");

        let config = load_config_from(cwd.path(), None)
            .expect("load should succeed")
            .expect("config should exist");
        let defaults = config.defaults.expect("defaults should exist");
        let hooks = config.hooks.expect("hooks should exist");

        assert_eq!(defaults.agent.as_deref(), Some("codex"));
        assert_eq!(defaults.max_iterations, Some(20));
        assert_eq!(defaults.timeout, Some(600));
        assert_eq!(defaults.stall_timeout, Some(120));
        assert_eq!(defaults.max_failures, Some(3));
        assert_eq!(hooks.url.as_deref(), Some("https://example.com/webhook"));
        assert_eq!(hooks.token.as_deref(), Some("secret"));
    }
}
