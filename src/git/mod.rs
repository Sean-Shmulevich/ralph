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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::process::Command as StdCommand;
    use tempfile::{tempdir, TempDir};

    fn run_git(workdir: &Path, args: &[&str]) -> String {
        let output = StdCommand::new("git")
            .args(args)
            .current_dir(workdir)
            .output()
            .expect("git command should run");

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            panic!("git {} failed: {}", args.join(" "), stderr.trim());
        }

        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn init_repo() -> TempDir {
        let dir = tempdir().expect("create tempdir");
        run_git(dir.path(), &["init"]);
        run_git(dir.path(), &["config", "user.name", "Ralph Test"]);
        run_git(
            dir.path(),
            &["config", "user.email", "ralph-test@example.com"],
        );
        dir
    }

    fn create_initial_commit(workdir: &Path) {
        fs::write(workdir.join("README.md"), "initial\n").expect("write seed file");
        run_git(workdir, &["add", "README.md"]);
        run_git(workdir, &["commit", "-m", "chore: initial"]);
    }

    #[tokio::test]
    async fn creates_branch_from_current_head() {
        let dir = init_repo();
        create_initial_commit(dir.path());
        let manager = GitManager::new(dir.path());
        let before_head = run_git(dir.path(), &["rev-parse", "HEAD"]);

        manager
            .create_or_checkout_branch("feature/test-branch")
            .await
            .expect("create branch");

        let current_branch = manager.current_branch().await.expect("current branch");
        let after_head = run_git(dir.path(), &["rev-parse", "HEAD"]);

        assert_eq!(current_branch, "feature/test-branch");
        assert_eq!(before_head, after_head);
    }

    #[tokio::test]
    async fn switches_to_existing_branch_if_it_exists() {
        let dir = init_repo();
        create_initial_commit(dir.path());
        let default_branch = run_git(dir.path(), &["rev-parse", "--abbrev-ref", "HEAD"]);
        run_git(dir.path(), &["checkout", "-b", "existing-branch"]);
        run_git(dir.path(), &["checkout", &default_branch]);

        let manager = GitManager::new(dir.path());
        manager
            .create_or_checkout_branch("existing-branch")
            .await
            .expect("checkout existing branch");

        let current_branch = manager.current_branch().await.expect("current branch");
        assert_eq!(current_branch, "existing-branch");
    }

    #[tokio::test]
    async fn commit_all_stages_and_commits_all_changes() {
        let dir = init_repo();
        create_initial_commit(dir.path());

        fs::write(dir.path().join("README.md"), "updated\n").expect("update tracked file");
        fs::write(dir.path().join("NEW.txt"), "new file\n").expect("create new file");

        let manager = GitManager::new(dir.path());
        assert!(manager.has_changes().await.expect("status before commit"));

        manager
            .commit_all("feat: T4 - commit all files")
            .await
            .expect("commit all");

        assert!(!manager.has_changes().await.expect("status after commit"));

        let changed_files = run_git(
            dir.path(),
            &["show", "--pretty=format:", "--name-only", "HEAD"],
        );
        assert!(changed_files.lines().any(|line| line.trim() == "README.md"));
        assert!(changed_files.lines().any(|line| line.trim() == "NEW.txt"));
    }

    #[tokio::test]
    async fn commit_all_preserves_expected_commit_message_format() {
        let dir = init_repo();
        create_initial_commit(dir.path());
        fs::write(dir.path().join("feature.rs"), "// impl\n").expect("write file");
        let manager = GitManager::new(dir.path());
        let message = "feat: T4 — Git manager integration tests (ralph)";

        manager.commit_all(message).await.expect("commit");

        let subject = run_git(dir.path(), &["log", "-1", "--pretty=%s"]);
        assert_eq!(subject, message);
    }

    #[tokio::test]
    async fn works_in_fresh_git_repo_with_no_prior_commits() {
        let dir = init_repo();
        let manager = GitManager::new(dir.path());

        manager
            .create_or_checkout_branch("fresh-start")
            .await
            .expect("create branch in fresh repo");
        fs::write(dir.path().join("first.txt"), "first commit\n").expect("write first file");

        manager
            .commit_all("feat: T4 - first commit in fresh repo")
            .await
            .expect("first commit should succeed");

        let commit_count = run_git(dir.path(), &["rev-list", "--count", "HEAD"]);
        let current_branch = manager.current_branch().await.expect("current branch");

        assert_eq!(commit_count, "1");
        assert_eq!(current_branch, "fresh-start");
    }
}
