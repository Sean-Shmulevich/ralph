use anyhow::{Context, Result};
use std::path::Path;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use super::{Agent, AgentProcess};

/// Claude Code agent backend.
///
/// Pipes the prompt via stdin (`-p -`) to avoid hitting the OS ARG_MAX limit
/// on large prompts. Equivalent to: `echo "<prompt>" | claude --print -p -`
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
        super::check_binary_available("claude")
    }

    fn spawn(&self, prompt: &str, workdir: &Path) -> Result<AgentProcess> {
        let mut cmd = Command::new("claude");

        cmd.arg("--dangerously-skip-permissions")
            .arg("--print")
            .arg("-p")
            .arg("-"); // read prompt from stdin

        if let Some(ref model) = self.model {
            cmd.arg("--model").arg(model);
        }

        cmd.current_dir(workdir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .context("Failed to spawn claude — is it installed and on PATH?")?;

        // Write the prompt to stdin, then close it
        let prompt_bytes = prompt.as_bytes().to_vec();
        let mut stdin = child.stdin.take().expect("stdin was piped");
        tokio::spawn(async move {
            let _ = stdin.write_all(&prompt_bytes).await;
            let _ = stdin.shutdown().await;
        });

        Ok(AgentProcess { child })
    }
}
