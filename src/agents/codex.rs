use anyhow::{Context, Result};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

use super::{Agent, AgentProcess};

/// Codex (OpenAI) CLI agent backend.
///
/// Invokes: `codex exec --full-auto "<prompt>"`
///
/// `--full-auto` is codex's convenience flag for sandboxed automatic execution
/// with low-friction approvals — equivalent to the PRD's `--approval-mode full-auto`.
/// An optional `--model MODEL` override is supported.
pub struct CodexAgent {
    model: Option<String>,
}

impl CodexAgent {
    pub fn new(model: Option<String>) -> Self {
        Self { model }
    }
}

impl Agent for CodexAgent {
    fn name(&self) -> &str {
        "codex"
    }

    fn is_available(&self) -> bool {
        std::process::Command::new("which")
            .arg("codex")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn spawn(&self, prompt: &str, workdir: &Path) -> Result<AgentProcess> {
        let mut cmd = Command::new("codex");

        // `codex exec` is the non-interactive subcommand
        cmd.arg("exec")
            // auto-approve all actions, sandboxed to workspace writes
            .arg("--full-auto");

        if let Some(ref model) = self.model {
            cmd.arg("--model").arg(model);
        }

        // Prompt is a positional argument (after flags)
        cmd.arg(prompt);

        cmd.current_dir(workdir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let child = cmd
            .spawn()
            .context("Failed to spawn codex — is it installed and on PATH?")?;

        Ok(AgentProcess { child })
    }
}
