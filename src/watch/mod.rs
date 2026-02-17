//! `ralph watch` — run multiple PRDs in parallel, each in its own orchestrator loop.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::cli::{RunArgs, WatchArgs};
use crate::state::{LoopState, LoopStatus, SharedLoopStatus};

// ── Public entry point ────────────────────────────────────────────────────────

pub async fn watch(args: WatchArgs) -> Result<()> {
    if args.prds.is_empty() {
        anyhow::bail!("No PRD files specified for ralph watch");
    }

    let workdir = resolve_workdir(args.workdir.as_deref())?;

    let parallel = args.parallel.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .min(4)
    });

    // Validate all PRD paths exist before starting
    let prds: Vec<PathBuf> = args
        .prds
        .iter()
        .map(|p| {
            p.canonicalize()
                .with_context(|| format!("PRD not found: {}", p.display()))
        })
        .collect::<Result<Vec<_>>>()?;

    // Derive unique slugs (deduplicate if two PRDs have the same stem)
    let slugs = make_unique_slugs(&prds);

    println!("🚀  Ralph Watch — {} PRDs, parallel={}", prds.len(), parallel);
    for (prd, slug) in prds.iter().zip(slugs.iter()) {
        println!("    • {} → .ralph-{}/", prd.display(), slug);
    }

    // Create shared LoopStatus for each loop
    let statuses: Vec<SharedLoopStatus> = prds
        .iter()
        .zip(slugs.iter())
        .map(|(prd, slug)| {
            Arc::new(std::sync::Mutex::new(LoopStatus::new(
                slug.clone(),
                prd.to_string_lossy().to_string(),
                args.agent.clone(),
            )))
        })
        .collect();

    // Shared cancellation flag — set to true on SIGINT/SIGTERM or when TUI quits
    let cancel_flag = Arc::new(AtomicBool::new(false));

    // ── Signal handlers ───────────────────────────────────────────────────────
    {
        let cf = cancel_flag.clone();
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            eprintln!("\n🛑  Interrupt received — stopping all loops…");
            cf.store(true, Ordering::Relaxed);
        });
    }

    #[cfg(unix)]
    {
        let cf = cancel_flag.clone();
        tokio::spawn(async move {
            use tokio::signal::unix::{signal, SignalKind};
            if let Ok(mut stream) = signal(SignalKind::terminate()) {
                stream.recv().await;
                eprintln!("\n🛑  SIGTERM received — stopping all loops…");
                cf.store(true, Ordering::Relaxed);
            }
        });
    }

    // ── TUI (unless --no-tui or not a terminal) ───────────────────────────────
    let tui_handle = if !args.no_tui && is_tty() {
        let statuses_clone = statuses.clone();
        let cf = cancel_flag.clone();
        Some(std::thread::spawn(move || {
            crate::tui::run_tui(statuses_clone, cf)
        }))
    } else {
        if !args.no_tui {
            println!("   (TUI disabled — not a TTY; using plain output)");
        }
        None
    };

    // ── Spawn orchestrator loops ──────────────────────────────────────────────
    let semaphore = Arc::new(Semaphore::new(parallel));
    let mut join_set = JoinSet::new();

    for (prd, (slug, status)) in prds.iter().zip(slugs.iter().zip(statuses.iter())) {
        // Acquire a semaphore permit before spawning (blocks if at capacity)
        let permit = semaphore.clone().acquire_owned().await?;

        let run_args = build_run_args(&args, prd, slug, &workdir, status.clone(), &cancel_flag);
        let status_clone = status.clone();

        join_set.spawn(async move {
            let result = crate::orchestrator::run(run_args).await;
            drop(permit); // Release slot back to semaphore

            if let Err(ref e) = result {
                if let Ok(mut s) = status_clone.lock() {
                    s.state = LoopState::Failed(e.to_string());
                    s.push_log(format!("❌ Loop failed: {e}"));
                }
            }

            result
        });
    }

    // ── Wait for all loops to finish ──────────────────────────────────────────
    let mut errors: Vec<String> = Vec::new();
    while let Some(outcome) = join_set.join_next().await {
        match outcome {
            Ok(Ok(())) => {}
            Ok(Err(e)) => errors.push(e.to_string()),
            Err(e) => errors.push(format!("task panic: {e}")),
        }
    }

    // Signal TUI to exit and wait for it
    cancel_flag.store(true, Ordering::Relaxed);
    if let Some(handle) = tui_handle {
        let _ = handle.join();
    }

    // ── Final summary ─────────────────────────────────────────────────────────
    println!("\n📋  Watch complete — summary:");
    for status in &statuses {
        if let Ok(s) = status.lock() {
            let icon = match &s.state {
                LoopState::Complete => "✅",
                LoopState::Failed(_) => "❌",
                LoopState::Stopped => "🛑",
                _ => "⚠️ ",
            };
            println!(
                "    {} {}  {}/{} tasks  ({})",
                icon, s.name, s.tasks_done, s.tasks_total, s.elapsed_str()
            );
        }
    }

    if !errors.is_empty() {
        eprintln!("\n⚠️  Loop errors:");
        for e in &errors {
            eprintln!("   • {e}");
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

/// Derive a URL-safe slug from a PRD path (file stem, lowercased, spaces→dashes).
pub fn prd_slug(prd: &Path) -> String {
    prd.file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

/// Generate unique slugs for a list of PRDs.
/// If two PRDs would produce the same slug, suffix them with `-2`, `-3`, etc.
fn make_unique_slugs(prds: &[PathBuf]) -> Vec<String> {
    let mut seen: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let mut result = Vec::with_capacity(prds.len());

    for prd in prds {
        let base = prd_slug(prd);
        let count = seen.entry(base.clone()).or_insert(0);
        *count += 1;
        if *count == 1 {
            result.push(base);
        } else {
            result.push(format!("{}-{}", base, count));
        }
    }
    result
}

/// Build a `RunArgs` struct for one loop in watch mode.
fn build_run_args(
    watch_args: &WatchArgs,
    prd: &Path,
    slug: &str,
    workdir: &Path,
    loop_status: SharedLoopStatus,
    cancel_flag: &Arc<AtomicBool>,
) -> RunArgs {
    RunArgs {
        prd: prd.to_path_buf(),
        agent: watch_args.agent.clone(),
        model: watch_args.model.clone(),
        max_iterations: watch_args.max_iterations,
        timeout: watch_args.timeout,
        stall_timeout: watch_args.stall_timeout,
        max_failures: watch_args.max_failures,
        workdir: Some(workdir.to_path_buf()),
        // Git branching is disabled for parallel watch mode (avoids concurrent conflicts).
        // Users who need branching should use `ralph run` per PRD.
        branch: None,
        no_branch: true,
        // Never print verbose output in watch mode — logs go to files + TUI buffer
        verbose: false,
        dry_run: false,
        hook_url: watch_args.hook_url.clone(),
        hook_token: watch_args.hook_token.clone(),
        state_name: Some(slug.to_string()),
        loop_status: Some(loop_status),
        cancel_flag: Some(cancel_flag.clone()),
    }
}

fn is_tty() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}
