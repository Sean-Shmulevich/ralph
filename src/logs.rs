//! `ralph logs [<name>] [--follow]` — stream logs for a named loop.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tokio::io::AsyncReadExt;
use tokio::time::{interval, Duration};

use crate::cli::LogsArgs;

pub async fn show_logs(args: LogsArgs) -> Result<()> {
    let workdir = resolve_workdir(args.workdir.as_deref())?;

    // Find the logs directory: .ralph-<name>/logs/ or .ralph/logs/
    let logs_dir = find_logs_dir(&workdir, args.name.as_deref())?;

    if args.follow {
        follow_logs(&logs_dir).await
    } else {
        dump_logs(&logs_dir).await
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn resolve_workdir(workdir: Option<&Path>) -> Result<PathBuf> {
    workdir
        .unwrap_or_else(|| Path::new("."))
        .canonicalize()
        .context("Cannot resolve workdir — does it exist?")
}

/// Locate the logs directory for a given loop name.
fn find_logs_dir(workdir: &Path, name: Option<&str>) -> Result<PathBuf> {
    match name {
        Some(n) => {
            // Try .ralph-<name>/logs/ first
            let named = workdir.join(format!(".ralph-{}", n)).join("logs");
            if named.exists() {
                return Ok(named);
            }
            // Fall back to .ralph/logs/ with a warning
            let default_logs = workdir.join(".ralph").join("logs");
            if default_logs.exists() {
                eprintln!(
                    "⚠️  .ralph-{}/logs/ not found, falling back to .ralph/logs/",
                    n
                );
                return Ok(default_logs);
            }
            anyhow::bail!(
                "No logs directory found for '{}'. Tried:\n  {}\n  {}",
                n,
                named.display(),
                default_logs.display()
            );
        }
        None => {
            // Default .ralph/logs/
            let default_logs = workdir.join(".ralph").join("logs");
            if default_logs.exists() {
                return Ok(default_logs);
            }
            anyhow::bail!(
                "No .ralph/logs/ directory found in {}. \
                Is there a running or completed ralph loop here?",
                workdir.display()
            );
        }
    }
}

/// Collect all iteration log files sorted by iteration number and print them.
async fn dump_logs(logs_dir: &Path) -> Result<()> {
    let mut entries = collect_log_files(logs_dir).await?;
    if entries.is_empty() {
        println!("(no log files found in {})", logs_dir.display());
        return Ok(());
    }
    entries.sort_by_key(|(n, _)| *n);

    for (_, path) in &entries {
        let content = tokio::fs::read_to_string(path)
            .await
            .with_context(|| format!("Cannot read log {}", path.display()))?;
        println!(
            "\n─── {} ───",
            path.file_name().unwrap_or_default().to_string_lossy()
        );
        print!("{}", content);
    }
    Ok(())
}

/// Follow (tail) the most recently modified log file, switching to newer files
/// as they appear.
async fn follow_logs(logs_dir: &Path) -> Result<()> {
    println!("Following logs in {} (Ctrl-C to stop)", logs_dir.display());

    let mut current_path: Option<PathBuf> = None;
    let mut file: Option<tokio::fs::File> = None;
    let mut buf = Vec::new();
    let mut ticker = interval(Duration::from_millis(200));

    loop {
        ticker.tick().await;

        // Find the newest log file
        let newest = newest_log_file(logs_dir).await;

        match (&current_path, &newest) {
            (_, None) => {
                // No logs yet
            }
            (None, Some(new_path)) | (Some(_), Some(new_path))
                if current_path.as_deref() != Some(new_path.as_path()) =>
            {
                // Switched to a new file — print a header and start from beginning
                println!(
                    "\n─── {} ───",
                    new_path.file_name().unwrap_or_default().to_string_lossy()
                );
                let f = tokio::fs::File::open(new_path)
                    .await
                    .with_context(|| format!("Cannot open {}", new_path.display()))?;
                current_path = Some(new_path.clone());
                file = Some(f);
            }
            _ => {}
        }

        // Read any new content from the current file
        if let Some(ref mut f) = file {
            buf.clear();
            let n = f.read_to_end(&mut buf).await.unwrap_or(0);
            if n > 0 {
                let chunk = String::from_utf8_lossy(&buf[..n]);
                print!("{}", chunk);
                // Flush stdout so output appears immediately
                use std::io::Write;
                let _ = std::io::stdout().flush();
            }
        }
    }
}

/// Return all `iteration-*.log` files in the directory with their iteration number.
async fn collect_log_files(logs_dir: &Path) -> Result<Vec<(u32, PathBuf)>> {
    let mut result = Vec::new();
    let mut read_dir = tokio::fs::read_dir(logs_dir)
        .await
        .with_context(|| format!("Cannot read logs dir: {}", logs_dir.display()))?;

    while let Some(entry) = read_dir.next_entry().await? {
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with("iteration-") && name.ends_with(".log") {
                // Extract the iteration number: "iteration-<N>-<task>.log"
                let n: u32 = name
                    .trim_start_matches("iteration-")
                    .split('-')
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                result.push((n, path));
            }
        }
    }
    Ok(result)
}

/// Find the newest (highest iteration number) log file in the directory.
async fn newest_log_file(logs_dir: &Path) -> Option<PathBuf> {
    let mut entries = collect_log_files(logs_dir).await.ok()?;
    if entries.is_empty() {
        return None;
    }
    entries.sort_by_key(|(n, _)| *n);
    entries.pop().map(|(_, p)| p)
}
