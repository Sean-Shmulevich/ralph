use anyhow::{Context, Result};
use std::path::Path;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use super::{Agent, AgentProcess};

/// OpenCode CLI agent backend.
///
/// Pipes the prompt via stdin to avoid hitting the OS ARG_MAX limit.
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
        super::check_binary_available("opencode")
    }

    fn spawn(&self, prompt: &str, workdir: &Path) -> Result<AgentProcess> {
        let mut cmd = Command::new("opencode");

        cmd.arg("run");

        if let Some(ref model) = self.model {
            cmd.arg("--model").arg(model);
        }

        cmd.current_dir(workdir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .context("Failed to spawn opencode — is it installed and on PATH?")?;

        let prompt_bytes = prompt.as_bytes().to_vec();
        let mut stdin = child.stdin.take().expect("stdin was piped");
        tokio::spawn(async move {
            let _ = stdin.write_all(&prompt_bytes).await;
            let _ = stdin.shutdown().await;
        });

        Ok(AgentProcess { child })
    }
}
