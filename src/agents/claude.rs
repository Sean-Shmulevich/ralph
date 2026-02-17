use anyhow::{Context, Result};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

use super::{Agent, AgentProcess};

/// Claude Code agent backend.
///
/// Invokes: `claude --dangerously-skip-permissions --print -p "<prompt>"`
/// with an optional `--model MODEL` flag.
pub struct ClaudeAgent {
    model: Option<String>,
}

impl ClaudeAgent {
    pub fn new(model: Option<String>) -> Self {
        Self { model }
    }
}

impl Agent for ClaudeAgent {
    fn is_available(&self) -> bool {
        std::process::Command::new("which")
            .arg("claude")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn spawn(&self, prompt: &str, workdir: &Path) -> Result<AgentProcess> {
        let mut cmd = Command::new("claude");

        cmd.arg("--dangerously-skip-permissions")
            .arg("--print")
            .arg("-p")
            .arg(prompt);

        if let Some(ref model) = self.model {
            cmd.arg("--model").arg(model);
        }

        cmd.current_dir(workdir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let child = cmd
            .spawn()
            .context("Failed to spawn claude — is it installed and on PATH?")?;

        Ok(AgentProcess { child })
    }
}
