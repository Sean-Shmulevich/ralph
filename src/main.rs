mod agents;
mod cli;
mod git;
mod logs;
mod orchestrator;
mod parser;
mod state;
mod stop;
mod tui;
mod watch;

use std::path::PathBuf;
mod watcher;

use anyhow::{Context, Result};
use chrono::Utc;
use clap::Parser;
use cli::{Cli, Commands};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run(args) => {
            orchestrator::run(args).await?;
        }
        Commands::Parse(args) => {
            parser::parse_and_print(args).await?;
        }
        Commands::Status(args) => {
            show_status(args).await?;
        }
        Commands::Watch(args) => {
            watch::watch(args).await?;
        }
        Commands::Logs(args) => {
            logs::show_logs(args).await?;
        }
        Commands::Stop(args) => {
            stop::stop_loops(args).await?;
        }
    }

    Ok(())
}

async fn show_status(args: cli::StatusArgs) -> Result<()> {
    use std::path::PathBuf;

    let workdir: PathBuf = args
        .workdir
        .unwrap_or_else(|| PathBuf::from("."))
        .canonicalize()
        .context("Cannot resolve workdir — does it exist?")?;

    // In status command, we check for both .ralph/ and .ralph-*/ locks
    // to give a complete picture.

    let locks = find_active_locks(&workdir).await?;

    if locks.is_empty() {
        println!("💤  No ralph loops running in {}", workdir.display());
        return Ok(());
    }

    println!("🟢  {} active loop(s) in {}\n", locks.len(), workdir.display());

    for (path, lock) in locks {
        let elapsed = Utc::now()
            .signed_duration_since(lock.started_at)
            .to_std()
            .unwrap_or_default();

        let elapsed_str = format_duration(elapsed);
        let name = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy())
            .unwrap_or_default();
        let loop_Name = if name == ".ralph" {
            "default".to_string()
        } else {
            name.trim_start_matches(".ralph-").to_string()
        };

        // Check if alive
        let alive = is_pid_alive(lock.pid);
        let status_icon = if alive { "🟢" } else { "💀" };

        println!("    {status_icon} [{}] PID {}", loop_Name, lock.pid);
        println!("       PRD:      {}", lock.prd_path);
        println!("       Agent:    {}", lock.agent);
        println!("       Task:     {}", lock.current_task);
        println!("       Progress: {}", lock.progress);
        println!("       Time:     {}", elapsed_str);
        if !alive {
            println!("       (process appears dead — stale lock)");
        }
        println!();
    }

    Ok(())
}

/// Check if a process with the given PID is alive.
fn is_pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        use nix::sys::signal::{kill, Signal};
        use nix::unistd::Pid;
        // signal 0 checks for existence
        kill(Pid::from_raw(pid as i32), Option::<Signal>::None).is_ok()
    }

    #[cfg(not(unix))]
    {
        // Fallback for non-unix (though likely running on Linux per prompt)
        std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {}", pid)])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
            .unwrap_or(false)
    }
}

/// Format a duration as h:m:s
fn format_duration(d: std::time::Duration) -> String {
    let total_secs = d.as_secs();
    let h = total_secs / 3600;
    let m = (total_secs % 3600) / 60;
    let s = total_secs % 60;

    if h > 0 {
        format!("{}h {}m {}s", h, m, s)
    } else if m > 0 {
        format!("{}m {}s", m, s)
    } else {
        format!("{}s", s)
    }
}

/// Find all lock files in .ralph/ and .ralph-*/ directories.
async fn find_active_locks(workdir: &std::path::Path) -> Result<Vec<(PathBuf, state::LockFile)>> {
    let mut results = Vec::new();
    let mut read_dir = tokio::fs::read_dir(workdir)
        .await
        .context("Cannot read workdir")?;

    while let Some(entry) = read_dir.next_entry().await? {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();

        if name == ".ralph" || name.starts_with(".ralph-") {
            let lock_path = path.join("lock");
            if lock_path.exists() {
                // Try parse
                if let Ok(content) = tokio::fs::read_to_string(&lock_path).await {
                    if let Ok(lock) = serde_json::from_str::<state::LockFile>(&content) {
                        results.push((lock_path, lock));
                    }
                }
            }
        }
    }
    Ok(results)
}
