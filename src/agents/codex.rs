use anyhow::{Context, Result};
use std::path::Path;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use super::{Agent, AgentProcess};

/// Codex (OpenAI) CLI agent backend.
///
/// Pipes the prompt via stdin to avoid hitting the OS ARG_MAX limit.
/// Codex appends stdin to the `-p` flag content.
pub struct CodexAgent {
    model: Option<String>,
}

impl CodexAgent {
    pub fn new(model: Option<String>) -> Self {
        Self { model }
    }
}

impl Agent for CodexAgent {
    fn is_available(&self) -> bool {
        super::check_binary_available("codex")
    }

    fn spawn(&self, prompt: &str, workdir: &Path) -> Result<AgentProcess> {
        let mut cmd = Command::new("codex");

        cmd.arg("exec").arg("--full-auto");

        if let Some(ref model) = self.model {
            cmd.arg("--model").arg(model);
        }

        cmd.current_dir(workdir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .context("Failed to spawn codex — is it installed and on PATH?")?;

        let prompt_bytes = prompt.as_bytes().to_vec();
        let mut stdin = child.stdin.take().expect("stdin was piped");
        tokio::spawn(async move {
            let _ = stdin.write_all(&prompt_bytes).await;
            let _ = stdin.shutdown().await;
        });

        Ok(AgentProcess { child })
    }
}
