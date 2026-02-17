//! `ralph stop [<name>|--all]` — gracefully stop running loops via SIGTERM.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::cli::StopArgs;
use crate::state::LockFile;

pub async fn stop_loops(args: StopArgs) -> Result<()> {
    if args.all && args.workdir.is_none() {
        // Stop all loops system-wide via global registry
        return stop_all_global().await;
    }

    let workdir = resolve_workdir(args.workdir.as_deref())?;

    if args.all {
        stop_all(&workdir).await
    } else {
        stop_named(&workdir, args.name.as_deref()).await
    }
}

// ── Implementations ───────────────────────────────────────────────────────────

/// Stop the loop identified by `name`, or the default `.ralph/` loop if name is None.
async fn stop_named(workdir: &Path, name: Option<&str>) -> Result<()> {
    let lock_path = match name {
        Some(n) => workdir.join(format!(".ralph-{}", n)).join("lock"),
        None => workdir.join(".ralph").join("lock"),
    };

    if !lock_path.exists() {
        let label = name.unwrap_or("default");
        println!("💤  No lock file found for '{}' (already stopped?)", label);
        println!("    Checked: {}", lock_path.display());
        return Ok(());
    }

    let lock = read_lock(&lock_path)?;
    send_sigterm_to_lock(&lock, &lock_path)?;
    Ok(())
}

/// Stop all loops system-wide using the global registry.
async fn stop_all_global() -> Result<()> {
    let locks = crate::state::StateManager::find_all_global_locks();
    if locks.is_empty() {
        println!("💤  No running ralph loops found system-wide");
        return Ok(());
    }
    println!("🛑  Stopping {} loop(s) system-wide…", locks.len());
    for (lock_path, lock) in &locks {
        let _ = send_sigterm_to_lock(lock, lock_path);
    }
    Ok(())
}

/// Find all `.ralph*/lock` files in workdir and stop every running loop.
async fn stop_all(workdir: &Path) -> Result<()> {
    let lock_files = find_all_lock_files(workdir).await?;

    if lock_files.is_empty() {
        println!("💤  No running ralph loops found in {}", workdir.display());
        return Ok(());
    }

    println!("🛑  Stopping {} loop(s)…", lock_files.len());
    for lock_path in &lock_files {
        match read_lock(lock_path) {
            Ok(lock) => {
                let _ = send_sigterm_to_lock(&lock, lock_path);
            }
            Err(e) => {
                eprintln!("    ⚠️  Could not read {}: {e}", lock_path.display());
            }
        }
    }
    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn resolve_workdir(workdir: Option<&Path>) -> Result<PathBuf> {
    workdir
        .unwrap_or_else(|| Path::new("."))
        .canonicalize()
        .context("Cannot resolve workdir — does it exist?")
}

fn read_lock(lock_path: &Path) -> Result<LockFile> {
    let content = std::fs::read_to_string(lock_path)
        .with_context(|| format!("Cannot read lock file: {}", lock_path.display()))?;
    serde_json::from_str::<LockFile>(&content)
        .with_context(|| format!("Cannot parse lock file: {}", lock_path.display()))
}

/// Send SIGTERM to the PID in the lock file, reporting the result.
fn send_sigterm_to_lock(lock: &LockFile, lock_path: &Path) -> Result<()> {
    let pid = lock.pid;
    let task = &lock.current_task;
    let prd = &lock.prd_path;

    // Check if the process is still alive first
    if !is_pid_alive(pid) {
        println!(
            "💀  PID {} is not running (stale lock: {})",
            pid,
            lock_path.display()
        );
        // Clean up stale lock
        let _ = std::fs::remove_file(lock_path);
        return Ok(());
    }

    println!(
        "🔴  Sending SIGTERM to PID {} ({}, task: {})",
        pid, prd, task
    );

    #[cfg(unix)]
    {
        use nix::sys::signal::{kill, Signal};
        use nix::unistd::Pid;
        kill(Pid::from_raw(pid as i32), Signal::SIGTERM)
            .with_context(|| format!("Failed to send SIGTERM to PID {}", pid))?;
        println!("    ✅  SIGTERM sent to PID {}", pid);
    }

    #[cfg(not(unix))]
    {
        // On non-Unix (Windows), use taskkill
        let output = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output()
            .context("Failed to run taskkill")?;
        if output.status.success() {
            println!("    ✅  Process {} terminated", pid);
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("taskkill failed for PID {}: {}", pid, stderr.trim());
        }
    }

    Ok(())
}

/// Return `true` if the process with the given PID is still running.
fn is_pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        use nix::sys::signal::{kill, Signal};
        use nix::unistd::Pid;
        // kill(pid, 0) succeeds if the process exists and we have permission
        kill(
            Pid::from_raw(pid as i32),
            Signal::SIGWINCH, /* harmless */
        )
        .map(|_| true)
        .unwrap_or_else(|_| {
            // Try with signal 0 (existence check)
            matches!(
                nix::errno::Errno::last(),
                nix::errno::Errno::EPERM // exists but no permission
            )
        })
    }

    #[cfg(not(unix))]
    {
        std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {}", pid)])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
            .unwrap_or(false)
    }
}

/// Find all `lock` files inside `.ralph*/` directories in the workdir.
async fn find_all_lock_files(workdir: &Path) -> Result<Vec<PathBuf>> {
    let mut result = Vec::new();
    let mut read_dir = tokio::fs::read_dir(workdir)
        .await
        .with_context(|| format!("Cannot read workdir: {}", workdir.display()))?;

    while let Some(entry) = read_dir.next_entry().await? {
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if (name == ".ralph" || name.starts_with(".ralph-")) && path.is_dir() {
                let lock_path = path.join("lock");
                if lock_path.exists() {
                    result.push(lock_path);
                }
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::StateManager;
    use chrono::Utc;
    use tempfile::tempdir;

    fn sample_lock(pid: u32) -> LockFile {
        LockFile {
            pid,
            current_task: "T8 — Lock tests".to_string(),
            progress: "0/1 done".to_string(),
            started_at: Utc::now(),
            prd_path: "tests/PRD.md".to_string(),
            agent: "codex".to_string(),
        }
    }

    #[test]
    fn stale_lock_is_detected_and_removed() {
        let dir = tempdir().expect("create tempdir");
        let state = StateManager::new(dir.path()).expect("create state manager");
        let stale_pid = (50_000u32..55_000u32)
            .find(|pid| !is_pid_alive(*pid))
            .expect("find an unused pid");
        let lock = sample_lock(stale_pid);
        state.write_lock(&lock).expect("write stale lock");
        assert!(
            state.lock_file.exists(),
            "lock file should exist before cleanup"
        );

        send_sigterm_to_lock(&lock, &state.lock_file).expect("handle stale lock");

        assert!(
            !state.lock_file.exists(),
            "stale lock should be removed when PID is dead"
        );
    }

    #[tokio::test]
    async fn find_all_lock_files_includes_named_watch_state_dirs() {
        let dir = tempdir().expect("create tempdir");
        let default_state = StateManager::new(dir.path()).expect("create default state");
        let alpha_state = StateManager::new_named(dir.path(), "alpha").expect("create alpha state");
        let beta_state = StateManager::new_named(dir.path(), "beta").expect("create beta state");
        let ignored_dir = dir.path().join(".ralphx");
        std::fs::create_dir_all(&ignored_dir).expect("create ignored dir");
        std::fs::write(ignored_dir.join("lock"), "not a ralph state").expect("write ignored lock");

        default_state
            .write_lock(&sample_lock(123))
            .expect("write default lock");
        alpha_state
            .write_lock(&sample_lock(456))
            .expect("write alpha lock");
        beta_state
            .write_lock(&sample_lock(789))
            .expect("write beta lock");

        let mut locks = find_all_lock_files(dir.path())
            .await
            .expect("find all lock files");
        locks.sort();

        assert_eq!(locks.len(), 3);
        assert!(locks.contains(&default_state.lock_file));
        assert!(locks.contains(&alpha_state.lock_file));
        assert!(locks.contains(&beta_state.lock_file));
    }
}
