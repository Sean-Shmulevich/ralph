/// Background watchdog that monitors a running agent iteration.
///
/// The watcher runs as a separate tokio task and performs periodic health checks:
///
/// 1. **Output stall** — if no stdout/stderr has been seen for `stall_timeout`, fires
///    `WatcherEvent::StallDetected`.  The orchestrator is responsible for killing the
///    child and treating the iteration as failed.
///
/// 2. **Disk space** — warns when free space on the workdir filesystem drops below
///    `disk_warn_threshold` (default 1 GiB).
///
/// 3. **Git conflicts** — detects unmerged files (`UU`, `AA`, `DD` in `git status
///    --porcelain`) which would block a later auto-commit.
///
/// Communication flows via:
/// - An `Arc<AtomicU64>` last-output timestamp (seconds since UNIX epoch), updated by
///   the orchestrator's stdout/stderr reader tasks each time a line is received.
/// - A `mpsc::Sender<WatcherEvent>` through which the watcher pushes events back to
///   the orchestrator.
/// - A `oneshot::Sender<()>` owned by the orchestrator; when it is dropped (or the
///   iteration ends), the watcher's `shutdown_rx` becomes ready and the task exits.
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio::time::interval;

// ── Public types ──────────────────────────────────────────────────────────────

/// Events the watcher sends to the orchestrator.
#[derive(Debug, Clone)]
pub enum WatcherEvent {
    /// Free disk space has dropped below the configured threshold.
    DiskSpaceWarning { free_bytes: u64 },

    /// Unmerged files detected in the working tree (merge conflict).
    GitConflictsDetected,

    /// No output received from the agent for `no_output_secs` seconds.
    /// The orchestrator should kill the child and fail the iteration.
    StallDetected { no_output_secs: u64 },
}

/// Configuration for the background watcher.
#[derive(Clone)]
pub struct WatcherConfig {
    /// How often health checks run (default: 5 s).
    pub check_interval: Duration,

    /// Time with no agent output before a `StallDetected` event fires (default: 120 s).
    pub stall_timeout: Duration,

    /// Free-space threshold in bytes below which a warning is emitted (default: 1 GiB).
    pub disk_warn_threshold: u64,

    /// Project working directory — used for git checks and disk-space queries.
    pub workdir: PathBuf,
}

impl WatcherConfig {
    /// Create a config using default intervals for the given workdir.
    pub fn new(workdir: PathBuf) -> Self {
        Self {
            check_interval: Duration::from_secs(5),
            stall_timeout: Duration::from_secs(120),
            disk_warn_threshold: 1024 * 1024 * 1024, // 1 GiB
            workdir,
        }
    }

    /// Override the stall timeout.
    pub fn with_stall_timeout(mut self, d: Duration) -> Self {
        self.stall_timeout = d;
        self
    }
}

/// Handle returned to the caller of `start_watcher`.
/// Dropping this handle (or calling `shutdown`) signals the watcher to exit.
pub struct WatcherHandle {
    _shutdown_tx: oneshot::Sender<()>,
}

impl WatcherHandle {
    /// Explicitly stop the watcher (also happens when the handle is dropped).
    pub fn shutdown(self) {
        // Dropping _shutdown_tx sends the signal via oneshot.
        drop(self);
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Start the background watcher as a detached tokio task.
///
/// Returns:
/// - A `WatcherHandle` — drop it (or call `.shutdown()`) to stop the watcher.
/// - An `mpsc::Receiver<WatcherEvent>` — poll this in your orchestrator select loop.
/// - An `Arc<AtomicU64>` last-output timestamp — update it from your stdout/stderr
///   reader tasks by calling `update_last_output(&last_output_ts)`.
pub fn start_watcher(
    config: WatcherConfig,
) -> (WatcherHandle, mpsc::Receiver<WatcherEvent>, Arc<AtomicU64>) {
    let (event_tx, event_rx) = mpsc::channel::<WatcherEvent>(16);
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    let now_secs = unix_now_secs();
    let last_output_ts = Arc::new(AtomicU64::new(now_secs));
    let last_output_ts_clone = last_output_ts.clone();

    tokio::spawn(async move {
        run_watcher(config, last_output_ts_clone, event_tx, shutdown_rx).await;
    });

    let handle = WatcherHandle {
        _shutdown_tx: shutdown_tx,
    };

    (handle, event_rx, last_output_ts)
}

/// Call this from stdout/stderr reader tasks whenever a line of output is received.
pub fn update_last_output(ts: &Arc<AtomicU64>) {
    ts.store(unix_now_secs(), Ordering::Relaxed);
}

// ── Watcher task ──────────────────────────────────────────────────────────────

async fn run_watcher(
    config: WatcherConfig,
    last_output_ts: Arc<AtomicU64>,
    event_tx: mpsc::Sender<WatcherEvent>,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    let mut ticker = interval(config.check_interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // Track whether we already fired a stall event for the current stall window.
    let mut stall_fired = false;

    loop {
        tokio::select! {
            biased;

            // Shutdown signal — orchestrator dropped the handle
            _ = &mut shutdown_rx => {
                break;
            }

            _ = ticker.tick() => {
                // ── Stall check ───────────────────────────────────────────────
                let last_ts = last_output_ts.load(Ordering::Relaxed);
                let now = unix_now_secs();
                let silent_secs = now.saturating_sub(last_ts);

                if silent_secs >= config.stall_timeout.as_secs() {
                    if !stall_fired {
                        stall_fired = true;
                        let _ = event_tx
                            .send(WatcherEvent::StallDetected {
                                no_output_secs: silent_secs,
                            })
                            .await;
                    }
                } else {
                    // Reset flag if output resumed (e.g. after we warned but didn't kill)
                    stall_fired = false;
                }

                // ── Disk space check ──────────────────────────────────────────
                match free_disk_bytes(&config.workdir).await {
                    Ok(free) if free < config.disk_warn_threshold => {
                        let _ = event_tx
                            .send(WatcherEvent::DiskSpaceWarning { free_bytes: free })
                            .await;
                    }
                    _ => {}
                }

                // ── Git conflict check ────────────────────────────────────────
                if has_git_conflicts(&config.workdir).await {
                    let _ = event_tx.send(WatcherEvent::GitConflictsDetected).await;
                }
            }
        }
    }
}

// ── OS helpers ────────────────────────────────────────────────────────────────

/// Return free disk space in bytes for the filesystem containing `path`.
///
/// Cross-platform: uses `df -k` (POSIX) and parses the "Available" column.
pub async fn free_disk_bytes(path: &Path) -> Result<u64> {
    let output = tokio::process::Command::new("df")
        .arg("-k") // 1K blocks, works on Linux + macOS + BSD
        .arg(path)
        .output()
        .await?;

    if !output.status.success() {
        anyhow::bail!("df failed");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Output looks like (header + data line):
    // Filesystem  1K-blocks  Used  Available  Use%  Mounted on
    // /dev/sda1   500000000  ...   123456789  ...   /
    //
    // Available is typically column index 3 (0-indexed).
    let avail_kb = stdout
        .lines()
        .nth(1)
        .and_then(|line| {
            line.split_whitespace()
                .nth(3)
                .and_then(|s| s.parse::<u64>().ok())
        })
        .ok_or_else(|| anyhow::anyhow!("Failed to parse df output: {}", stdout))?;

    Ok(avail_kb * 1024)
}

/// Return `true` if the git working tree contains unmerged files.
pub async fn has_git_conflicts(workdir: &Path) -> bool {
    let output = match tokio::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(workdir)
        .output()
        .await
    {
        Ok(o) => o,
        Err(_) => return false,
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().any(|line| {
        // UU = unmerged both modified
        // AA = unmerged both added
        // DD = unmerged both deleted
        line.starts_with("UU") || line.starts_with("AA") || line.starts_with("DD")
    })
}

/// Current time as seconds since the UNIX epoch.
fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::process::Command as StdCommand;
    use tempfile::tempdir;
    use tokio::time::timeout;

    fn unix_now_secs_for_test() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    fn run_git(workdir: &Path, args: &[&str]) {
        let output = StdCommand::new("git")
            .args(args)
            .current_dir(workdir)
            .output()
            .expect("git command should run");

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            panic!("git {} failed: {}", args.join(" "), stderr.trim());
        }
    }

    fn init_repo(workdir: &Path) {
        run_git(workdir, &["init"]);
        run_git(workdir, &["config", "user.name", "Watcher Test"]);
        run_git(
            workdir,
            &["config", "user.email", "watcher-test@example.com"],
        );
    }

    #[tokio::test]
    async fn stall_detection_fires_after_timeout_with_no_output() {
        let dir = tempdir().expect("create tempdir");
        let config = WatcherConfig {
            check_interval: Duration::from_millis(25),
            stall_timeout: Duration::from_secs(1),
            disk_warn_threshold: 0,
            workdir: dir.path().to_path_buf(),
        };

        let (_handle, mut event_rx, last_output_ts) = start_watcher(config);
        last_output_ts.store(
            unix_now_secs_for_test().saturating_sub(5),
            Ordering::Relaxed,
        );

        let event = timeout(Duration::from_secs(2), event_rx.recv())
            .await
            .expect("stall event should arrive")
            .expect("event channel should stay open");

        match event {
            WatcherEvent::StallDetected { no_output_secs } => {
                assert!(
                    no_output_secs >= 1,
                    "unexpected no_output_secs: {no_output_secs}"
                );
            }
            other => panic!("expected StallDetected, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn disk_space_warning_triggers_when_df_reports_low_space() {
        let dir = tempdir().expect("create tempdir");

        let config = WatcherConfig {
            check_interval: Duration::from_millis(25),
            stall_timeout: Duration::from_secs(3600),
            // Any finite free-space value is less than u64::MAX, so the warning
            // should fire on the first successful `df` check without env mocking.
            disk_warn_threshold: u64::MAX,
            workdir: dir.path().to_path_buf(),
        };

        let (_handle, mut event_rx, _last_output_ts) = start_watcher(config);
        let event = timeout(Duration::from_secs(2), event_rx.recv())
            .await
            .expect("disk warning should arrive")
            .expect("event channel should stay open");

        match event {
            WatcherEvent::DiskSpaceWarning { free_bytes } => {
                assert!(free_bytes > 0, "expected positive free-space bytes");
            }
            other => panic!("expected DiskSpaceWarning, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn git_conflict_detection_emits_event_for_uu_status() {
        let dir = tempdir().expect("create tempdir");
        init_repo(dir.path());

        let file_path = dir.path().join("conflict.txt");
        fs::write(&file_path, "base\n").expect("write base file");
        run_git(dir.path(), &["add", "conflict.txt"]);
        run_git(dir.path(), &["commit", "-m", "base"]);

        let default_branch = StdCommand::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(dir.path())
            .output()
            .expect("resolve default branch");
        assert!(default_branch.status.success(), "resolve default branch");
        let default_branch = String::from_utf8_lossy(&default_branch.stdout)
            .trim()
            .to_string();

        run_git(dir.path(), &["checkout", "-b", "feature/conflict"]);
        fs::write(&file_path, "feature\n").expect("write feature variant");
        run_git(dir.path(), &["commit", "-am", "feature change"]);

        run_git(dir.path(), &["checkout", &default_branch]);
        fs::write(&file_path, "main\n").expect("write main variant");
        run_git(dir.path(), &["commit", "-am", "main change"]);

        let merge_output = StdCommand::new("git")
            .args(["merge", "feature/conflict"])
            .current_dir(dir.path())
            .output()
            .expect("run merge");
        assert!(
            !merge_output.status.success(),
            "merge should fail with conflict"
        );

        let config = WatcherConfig {
            check_interval: Duration::from_millis(25),
            stall_timeout: Duration::from_secs(3600),
            disk_warn_threshold: 0,
            workdir: dir.path().to_path_buf(),
        };

        let (_handle, mut event_rx, _last_output_ts) = start_watcher(config);
        let event = timeout(Duration::from_secs(2), event_rx.recv())
            .await
            .expect("git conflict event should arrive")
            .expect("event channel should stay open");

        match event {
            WatcherEvent::GitConflictsDetected => {}
            other => panic!("expected GitConflictsDetected, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn watcher_exits_when_handle_is_dropped() {
        let dir = tempdir().expect("create tempdir");
        let config = WatcherConfig {
            check_interval: Duration::from_millis(25),
            stall_timeout: Duration::from_secs(3600),
            disk_warn_threshold: 0,
            workdir: dir.path().to_path_buf(),
        };

        let (handle, mut event_rx, _last_output_ts) = start_watcher(config);
        drop(handle);

        let recv = timeout(Duration::from_secs(2), event_rx.recv())
            .await
            .expect("watcher should terminate and close channel");
        assert!(recv.is_none(), "event channel should close after shutdown");
    }
}
