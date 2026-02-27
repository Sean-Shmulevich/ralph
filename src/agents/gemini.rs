use anyhow::{Context, Result};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

use super::{Agent, AgentProcess};

/// Gemini CLI agent backend.
///
/// Invokes: `gemini -p "<prompt>" --yolo`
/// `--yolo` auto-approves all tool calls (non-interactive mode).
/// An optional `--model MODEL` override is supported.
pub struct GeminiAgent {
    model: Option<String>,
}

impl GeminiAgent {
    pub fn new(model: Option<String>) -> Self {
        Self { model }
    }
}

impl Agent for GeminiAgent {
    fn is_available(&self) -> bool {
        super::check_binary_available("gemini")
    }

    fn spawn(&self, prompt: &str, workdir: &Path) -> Result<AgentProcess> {
        let mut cmd = Command::new("gemini");

        cmd.arg("-p")
            .arg(prompt)
            // auto-approve all tool calls — equivalent to YOLO mode
            .arg("--yolo");

        if let Some(ref model) = self.model {
            cmd.arg("--model").arg(model);
        }

        cmd.current_dir(workdir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let child = cmd
            .spawn()
            .context("Failed to spawn gemini — is it installed and on PATH?")?;

        Ok(AgentProcess { child })
    }
}
