use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tokio::process::Command;

/// Thin async wrapper around the `git` binary for branch and commit management.
pub struct GitManager {
    workdir: PathBuf,
}

impl GitManager {
    pub fn new(workdir: &Path) -> Self {
        Self {
            workdir: workdir.to_path_buf(),
        }
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    async fn run(&self, args: &[&str]) -> Result<String> {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.workdir)
            .output()
            .await
            .with_context(|| format!("Failed to run: git {}", args.join(" ")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("git {} failed: {}", args[0], stderr.trim());
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    // ── Public API ────────────────────────────────────────────────────────────

    /// Return `true` if the workdir is inside a git repository.
    pub async fn is_git_repo(&self) -> bool {
        self.run(&["rev-parse", "--is-inside-work-tree"])
            .await
            .is_ok()
    }

    /// Return the name of the currently checked-out branch.
    #[allow(dead_code)]
    pub async fn current_branch(&self) -> Result<String> {
        self.run(&["rev-parse", "--abbrev-ref", "HEAD"]).await
    }

    /// Create `branch` (if it doesn't exist) and check it out.
    pub async fn create_or_checkout_branch(&self, branch: &str) -> Result<()> {
        // Check if the branch already exists locally.
        let list = self
            .run(&["branch", "--list", branch])
            .await
            .unwrap_or_default();

        if list.trim().is_empty() {
            self.run(&["checkout", "-b", branch]).await?;
        } else {
            self.run(&["checkout", branch]).await?;
        }

        Ok(())
    }

    /// Return `true` if the working tree has any uncommitted changes.
    pub async fn has_changes(&self) -> Result<bool> {
        let status = self.run(&["status", "--porcelain"]).await?;
        Ok(!status.trim().is_empty())
    }

    /// Stage all changes and create a commit with `message`.
    pub async fn commit_all(&self, message: &str) -> Result<String> {
        self.run(&["add", "-A"]).await?;
        self.run(&["commit", "-m", message]).await
    }
}
