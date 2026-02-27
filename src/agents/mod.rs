mod api;
mod claude;
mod codex;
mod gemini;
mod opencode;

pub use api::ApiAgent;
pub use claude::ClaudeAgent;
pub use codex::CodexAgent;
pub use gemini::GeminiAgent;
pub use opencode::OpenCodeAgent;

use anyhow::Result;
use std::path::Path;
use tokio::process::Child;

/// A spawned agent process with attached stdio handles.
pub struct AgentProcess {
    pub child: Child,
}

/// Trait implemented by every agent backend (Claude Code, Gemini CLI, Codex, …).
///
/// `spawn` is intentionally synchronous — tokio's `Command::spawn()` doesn't need
/// to be awaited. Only the *waiting* for the child and reading its output are async.
pub trait Agent: Send + Sync {
    /// Return `true` if the agent binary is on PATH and appears runnable.
    fn is_available(&self) -> bool;

    /// Spawn the agent with the given prompt, returning the live process handle.
    fn spawn(&self, prompt: &str, workdir: &Path) -> Result<AgentProcess>;
}

/// Check if an agent binary is reachable by trying to run it directly.
/// This avoids shelling out to `which` (which may not be on PATH itself,
/// or may see a different PATH than the current process).
pub fn check_binary_available(name: &str) -> bool {
    std::process::Command::new(name)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Build the concrete agent implementation for the given name.
pub fn create_agent(
    name: &str,
    model: Option<String>,
    api_url: Option<String>,
    api_key: Option<String>,
) -> Result<Box<dyn Agent>> {
    match name {
        "claude" => Ok(Box::new(ClaudeAgent::new(model))),
        "gemini" => Ok(Box::new(GeminiAgent::new(model))),
        "codex" => Ok(Box::new(CodexAgent::new(model))),
        "opencode" => Ok(Box::new(OpenCodeAgent::new(model))),
        "api" => Ok(Box::new(ApiAgent::new(api_url, api_key, model)?)),
        other => anyhow::bail!(
            "Unknown agent '{}'. Supported agents: claude, gemini, codex, opencode, api",
            other
        ),
    }
}
