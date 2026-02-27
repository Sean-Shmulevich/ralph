use anyhow::{Context, Result};
use std::path::Path;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
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

        // Use `-p ""` so the actual prompt comes via stdin (avoids E2BIG)
        cmd.arg("-p").arg("").arg("--yolo");

        if let Some(ref model) = self.model {
            cmd.arg("--model").arg(model);
        }

        cmd.current_dir(workdir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .context("Failed to spawn gemini — is it installed and on PATH?")?;

        let prompt_bytes = prompt.as_bytes().to_vec();
        let mut stdin = child.stdin.take().expect("stdin was piped");
        tokio::spawn(async move {
            let _ = stdin.write_all(&prompt_bytes).await;
            let _ = stdin.shutdown().await;
        });

        Ok(AgentProcess { child })
    }
}
