use anyhow::{Context, Result};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

use super::{Agent, AgentProcess};

/// OpenCode CLI agent backend.
///
/// Invokes: `opencode run "<prompt>"`
///
/// OpenCode is an open-source terminal AI coding agent supporting 75+ providers.
/// In non-interactive `run` mode it executes the prompt and exits.
/// An optional `--model provider/model` override is supported.
pub struct OpenCodeAgent {
    model: Option<String>,
}

impl OpenCodeAgent {
    pub fn new(model: Option<String>) -> Self {
        Self { model }
    }
}

impl Agent for OpenCodeAgent {
    fn is_available(&self) -> bool {
        std::process::Command::new("which")
            .arg("opencode")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn spawn(&self, prompt: &str, workdir: &Path) -> Result<AgentProcess> {
        let mut cmd = Command::new("opencode");

        // `opencode run` is the non-interactive mode
        cmd.arg("run");

        if let Some(ref model) = self.model {
            cmd.arg("--model").arg(model);
        }

        // Prompt is a positional argument
        cmd.arg(prompt);

        cmd.current_dir(workdir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let child = cmd
            .spawn()
            .context("Failed to spawn opencode — is it installed and on PATH?")?;

        Ok(AgentProcess { child })
    }
}
