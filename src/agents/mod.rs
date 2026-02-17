mod claude;
mod codex;
mod gemini;

pub use claude::ClaudeAgent;
pub use codex::CodexAgent;
pub use gemini::GeminiAgent;

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
    /// Human-readable agent name (used in logs and messages).
    #[allow(dead_code)]
    fn name(&self) -> &str;

    /// Return `true` if the agent binary is on PATH and appears runnable.
    fn is_available(&self) -> bool;

    /// Spawn the agent with the given prompt, returning the live process handle.
    fn spawn(&self, prompt: &str, workdir: &Path) -> Result<AgentProcess>;
}

/// Build the concrete agent implementation for the given name.
pub fn create_agent(name: &str, model: Option<String>) -> Result<Box<dyn Agent>> {
    match name {
        "claude" => Ok(Box::new(ClaudeAgent::new(model))),
        "gemini" => Ok(Box::new(GeminiAgent::new(model))),
        "codex" => Ok(Box::new(CodexAgent::new(model))),
        other => anyhow::bail!(
            "Unknown agent '{}'. Supported agents: claude, gemini, codex",
            other
        ),
    }
}
