use anyhow::{Context, Result};
use chrono::Utc;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use tokio::fs as tfs;
use tokio::io::AsyncWriteExt as _;
use tokio::time::Duration;

use crate::agents::{create_agent, Agent};
use crate::cli::RunArgs;
use crate::git::GitManager;
use crate::parser::parse_prd;
use crate::state::{
    LockFile, LoopState, SharedLoopStatus, StateManager, Task, TaskList, TaskStatus,
};
use crate::watcher::{start_watcher, update_last_output, WatcherConfig, WatcherEvent};

// ── Prompt template ───────────────────────────────────────────────────────────

const ITERATION_PROMPT: &str = r#"You are an expert software engineer. Your mission is to implement a specific task from a PRD inside the current repository.

## Current Task

**Task ID**: {task_id}
**Title**: {task_title}
**Description**: {task_description}

## All Tasks (for context)

{all_tasks}

## PRD

{prd_content}

## Progress Log

{progress}

## Instructions

1. Implement **"{task_title}"** as described above.
2. Write clean, production-quality code — handle errors, add comments where helpful.
3. If a test suite exists (cargo test, npm test, pytest, etc.) run it and fix any failures.
4. When the task is **fully and completely done**, output this token on its own line:

   <promise>COMPLETE</promise>

5. If you cannot finish in this iteration, do as much as possible and explain what still remains — do NOT output the completion token.

Only output `<promise>COMPLETE</promise>` when you are genuinely confident the task is done.
"#;

// ── Entry point ───────────────────────────────────────────────────────────────

pub async fn run(args: RunArgs) -> Result<()> {
    // Resolve paths
    let workdir: PathBuf = args
        .workdir
        .clone()
        .unwrap_or_else(|| PathBuf::from("."))
        .canonicalize()
        .context("Cannot resolve workdir — does it exist?")?;

    let prd_path = args
        .prd
        .canonicalize()
        .with_context(|| format!("PRD file not found: {}", args.prd.display()))?;

    // Build state manager — named variant for `ralph watch`, default for `ralph run`
    let state = match &args.state_name {
        Some(name) => StateManager::new_named(&workdir, name)?,
        None => StateManager::new(&workdir)?,
    };

    let git = GitManager::new(&workdir);
    let agent = create_agent(&args.agent, args.model.clone())?;

    let is_watch_mode = args.state_name.is_some();

    if !is_watch_mode {
        // Interactive `ralph run` — print startup banner
        println!("🚀  Ralph — starting agent loop");
        println!("    PRD:             {}", prd_path.display());
        println!("    Agent:           {}", args.agent);
        println!("    Workdir:         {}", workdir.display());
        println!("    Max iterations:  {}", args.max_iterations);
        println!("    Timeout:         {}s per iteration", args.timeout);
        println!("    Stall timeout:   {}s no-output kill", args.stall_timeout);
        println!("    Max failures:    {}", args.max_failures);
    }

    if !agent.is_available() {
        anyhow::bail!(
            "Agent '{}' not found on PATH. Install it and try again.",
            args.agent
        );
    }

    // ── Write lock file ───────────────────────────────────────────────────────
    let run_started_at = Utc::now();
    let lock = LockFile {
        pid: std::process::id(),
        current_task: "starting…".to_string(),
        progress: "0/? done".to_string(),
        started_at: run_started_at,
        prd_path: prd_path.to_string_lossy().to_string(),
        agent: args.agent.clone(),
    };
    state.write_lock(&lock)?;

    struct LockGuard<'a>(&'a StateManager);
    impl Drop for LockGuard<'_> {
        fn drop(&mut self) {
            self.0.remove_lock();
        }
    }
    let _lock_guard = LockGuard(&state);

    // ── Update shared loop status ─────────────────────────────────────────────
    update_loop_state(&args.loop_status, LoopState::Parsing);

    // ── Git branch management ─────────────────────────────────────────────────
    if !args.no_branch && git.is_git_repo().await {
        let branch_name = args.branch.clone().unwrap_or_else(|| {
            let stem = prd_path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase()
                .replace(' ', "-");
            format!("ralph/{}", stem)
        });

        if !is_watch_mode {
            println!("\n🌿  Branch: {}", branch_name);
        }
        if let Err(e) = git.create_or_checkout_branch(&branch_name).await {
            if !is_watch_mode {
                eprintln!("⚠️   Git branch warning: {e}");
            }
            log_to_status(&args.loop_status, format!("⚠️  Git branch warning: {e}"));
        }
    }

    // ── Load or parse tasks ───────────────────────────────────────────────────
    let mut task_list = match state.load_tasks()? {
        Some(existing) => {
            if !is_watch_mode {
                println!(
                    "\n📂  Loaded existing tasks ({} total)",
                    existing.tasks.len()
                );
            }
            existing
        }
        None => {
            if !is_watch_mode {
                println!("\n🔍  No tasks.json found — parsing PRD…");
            }
            log_to_status(&args.loop_status, "Parsing PRD…".to_string());
            let tl = parse_prd(&prd_path, &args.agent, args.model.as_deref()).await?;
            state.save_tasks(&tl)?;
            if !is_watch_mode {
                println!("✅  Parsed {} tasks → tasks.json", tl.tasks.len());
            }
            tl
        }
    };

    // Update total task count in shared status
    if let Some(ref ls) = args.loop_status {
        if let Ok(mut s) = ls.lock() {
            s.tasks_total = task_list.tasks.len() as u32;
            s.state = LoopState::Running;
        }
    }

    // Dry-run: just show tasks and exit
    if args.dry_run {
        print_task_table(&task_list);
        return Ok(());
    }

    let prd_content = std::fs::read_to_string(&prd_path)
        .with_context(|| format!("Cannot read PRD: {}", prd_path.display()))?;

    let mut iteration: u32 = 1;
    let mut consecutive_failures: u32 = 0;

    // ── Main loop ─────────────────────────────────────────────────────────────
    loop {
        // Check cancellation flag (set by SIGINT/SIGTERM or `ralph stop`)
        if let Some(ref flag) = args.cancel_flag {
            if flag.load(Ordering::Relaxed) {
                if !is_watch_mode {
                    println!("\n🛑  Cancellation requested — saving state and stopping.");
                }
                update_loop_state(&args.loop_status, LoopState::Stopped);
                break;
            }
        }

        // Termination guards
        if iteration > args.max_iterations {
            if !is_watch_mode {
                println!(
                    "\n⚠️   Max iterations ({}) reached. Stopping.",
                    args.max_iterations
                );
            }
            update_loop_state(&args.loop_status, LoopState::Stopped);
            break;
        }

        if consecutive_failures >= args.max_failures {
            if !is_watch_mode {
                println!(
                    "\n❌  Circuit breaker: {} consecutive failures. Stopping.",
                    args.max_failures
                );
            }
            state.append_progress(&format!(
                "**STOPPED** — circuit breaker after {} consecutive failures (iteration {}).",
                args.max_failures, iteration
            ))?;
            update_loop_state(
                &args.loop_status,
                LoopState::Failed(format!("{} consecutive failures", args.max_failures)),
            );
            break;
        }

        // Pick the next actionable pending task (dependencies satisfied)
        let task = match pick_next_task(&task_list) {
            Some(t) => t.clone(),
            None => {
                if !is_watch_mode {
                    println!("\n✅  All tasks complete! PRD implementation finished.");
                }
                state.append_progress("**COMPLETE** — all tasks finished successfully.")?;
                update_loop_state(&args.loop_status, LoopState::Complete);
                break;
            }
        };

        let total_tasks = task_list.tasks.len();
        let done_tasks = task_list
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Complete)
            .count();

        if !is_watch_mode {
            println!(
                "\n━━━ Iteration {} ━━━  Task {} — {}",
                iteration, task.id, task.title
            );
        }

        // Update shared loop status
        if let Some(ref ls) = args.loop_status {
            if let Ok(mut s) = ls.lock() {
                s.current_task = format!("{} — {}", task.id, task.title);
                s.tasks_done = done_tasks as u32;
                s.iteration = iteration;
                s.state = LoopState::Running;
            }
        }

        // Update lock file with current progress
        let lock = LockFile {
            pid: std::process::id(),
            current_task: format!("{} — {}", task.id, task.title),
            progress: format!("{}/{} done", done_tasks, total_tasks),
            started_at: run_started_at,
            prd_path: prd_path.to_string_lossy().to_string(),
            agent: args.agent.clone(),
        };
        if let Err(e) = state.write_lock(&lock) {
            eprintln!("⚠️   Lock file update failed: {e}");
        }

        // Build prompt context
        let progress = std::fs::read_to_string(&state.progress_file).unwrap_or_default();
        let all_tasks = format_task_table(&task_list);

        let prompt = ITERATION_PROMPT
            .replace("{task_id}", &task.id)
            .replace("{task_title}", &task.title)
            .replace("{task_description}", &task.description)
            .replace("{all_tasks}", &all_tasks)
            .replace("{prd_content}", &prd_content)
            .replace("{progress}", &progress);

        // Mark in-progress and persist
        set_task_status(&mut task_list, &task.id, TaskStatus::InProgress);
        task_list.updated_at = Utc::now();
        state.save_tasks(&task_list)?;

        let log_path = state.log_path(iteration, &task.id);
        if !is_watch_mode {
            println!("    Log: {}", log_path.display());
        }

        // Snapshot tasks.json before the agent runs (detect agent-side changes)
        let tasks_snapshot_before = serde_json::to_string(&task_list.tasks).unwrap_or_default();

        // Spawn agent with timeout + stall detection
        let iter_result = run_iteration(
            agent.as_ref(),
            &prompt,
            &workdir,
            &log_path,
            args.timeout,
            args.stall_timeout,
            args.verbose && !is_watch_mode,
            args.loop_status.clone(),
        )
        .await;

        match iter_result {
            Ok(stdout) => {
                let promised_complete = stdout.contains("<promise>COMPLETE</promise>");

                // Check if the agent directly edited tasks.json
                let tasks_snapshot_after = state
                    .load_tasks()
                    .ok()
                    .flatten()
                    .map(|tl| serde_json::to_string(&tl.tasks).unwrap_or_default())
                    .unwrap_or_else(|| tasks_snapshot_before.clone());

                let agent_edited_tasks = tasks_snapshot_before != tasks_snapshot_after;

                let task_done = promised_complete || agent_edited_tasks;

                if task_done {
                    if !is_watch_mode {
                        println!("    ✅  Task {} — complete", task.id);
                    }
                    log_to_status(&args.loop_status, format!("✅ Task {} complete: {}", task.id, task.title));
                    consecutive_failures = 0;

                    set_task_status(&mut task_list, &task.id, TaskStatus::Complete);
                    if let Some(t) = task_list.tasks.iter_mut().find(|t| t.id == task.id) {
                        t.completed_at = Some(Utc::now());
                    }
                    task_list.updated_at = Utc::now();
                    state.save_tasks(&task_list)?;

                    // Update tasks_done count
                    if let Some(ref ls) = args.loop_status {
                        if let Ok(mut s) = ls.lock() {
                            s.tasks_done = task_list
                                .tasks
                                .iter()
                                .filter(|t| t.status == TaskStatus::Complete)
                                .count() as u32;
                        }
                    }

                    state.append_progress(&format!(
                        "**Task {} complete** — {}\n\n(iteration {})",
                        task.id, task.title, iteration
                    ))?;

                    // Auto-commit if there are changes
                    if !args.no_branch && git.is_git_repo().await {
                        match git.has_changes().await {
                            Ok(true) => {
                                let msg =
                                    format!("feat: {} — {} (ralph)", task.id, task.title);
                                match git.commit_all(&msg).await {
                                    Ok(_) => {
                                        if !is_watch_mode {
                                            println!("    📦  Git commit: {}", msg);
                                        }
                                    }
                                    Err(e) => {
                                        if !is_watch_mode {
                                            eprintln!("    ⚠️   Git commit failed: {e}");
                                        }
                                    }
                                }
                            }
                            Ok(false) => {}
                            Err(e) => {
                                if !is_watch_mode {
                                    eprintln!("    ⚠️   Git status check failed: {e}");
                                }
                            }
                        }
                    }
                } else {
                    if !is_watch_mode {
                        println!(
                            "    ⚠️   Task {} not completed this iteration (failure #{}/{})",
                            task.id,
                            consecutive_failures + 1,
                            args.max_failures
                        );
                    }
                    consecutive_failures += 1;

                    // Reset to pending so it will be retried
                    set_task_status(&mut task_list, &task.id, TaskStatus::Pending);
                    task_list.updated_at = Utc::now();
                    state.save_tasks(&task_list)?;

                    state.append_progress(&format!(
                        "**Iteration {} — Task {} incomplete**\n\nConsecutive failures: {}/{}",
                        iteration, task.id, consecutive_failures, args.max_failures
                    ))?;
                }
            }

            Err(e) => {
                if !is_watch_mode {
                    eprintln!("    ❌  Iteration error: {e}");
                }
                log_to_status(&args.loop_status, format!("❌ Iteration error: {e}"));
                consecutive_failures += 1;

                set_task_status(&mut task_list, &task.id, TaskStatus::Failed);
                task_list.updated_at = Utc::now();
                state.save_tasks(&task_list)?;

                state.append_progress(&format!(
                    "**Iteration {} FAILED** — Task {} error: {e}\n\nConsecutive failures: {}/{}",
                    iteration, task.id, consecutive_failures, args.max_failures
                ))?;
            }
        }

        iteration += 1;
    }

    if !is_watch_mode {
        println!();
        print_task_table(&task_list);
    }
    Ok(())
}

// ── Helpers for shared status ─────────────────────────────────────────────────

fn update_loop_state(ls: &Option<SharedLoopStatus>, state: LoopState) {
    if let Some(ref ls) = ls {
        if let Ok(mut s) = ls.lock() {
            s.state = state;
        }
    }
}

fn log_to_status(ls: &Option<SharedLoopStatus>, line: String) {
    if let Some(ref ls) = ls {
        if let Ok(mut s) = ls.lock() {
            s.push_log(line);
        }
    }
}

// ── Iteration execution ───────────────────────────────────────────────────────

/// Spawn the agent for one iteration, capture all output, and enforce:
///   - Hard timeout (kills after `timeout_secs`)
///   - Stall detection (kills if no stdout/stderr for `stall_timeout_secs`)
///
/// Stdout and stderr are read concurrently on separate tokio tasks so neither
/// pipe fills its kernel buffer and deadlocks the process.
async fn run_iteration(
    agent: &dyn Agent,
    prompt: &str,
    workdir: &Path,
    log_path: &Path,
    timeout_secs: u64,
    stall_timeout_secs: u64,
    verbose: bool,
    loop_status: Option<SharedLoopStatus>,
) -> Result<String> {
    let mut proc = agent.spawn(prompt, workdir)?;

    // Take the piped handles before moving `proc` anywhere.
    let stdout_pipe = proc
        .child
        .stdout
        .take()
        .context("Agent stdout pipe missing")?;
    let stderr_pipe = proc
        .child
        .stderr
        .take()
        .context("Agent stderr pipe missing")?;

    // ── Start background watcher ──────────────────────────────────────────────
    let watcher_config = WatcherConfig::new(workdir.to_path_buf())
        .with_stall_timeout(Duration::from_secs(stall_timeout_secs));
    let (_watcher_handle, mut event_rx, last_output_ts) = start_watcher(watcher_config);

    // ── Read stdout and stderr concurrently, updating stall timestamp ─────────
    let ts_stdout = last_output_ts.clone();
    let ls_stdout = loop_status.clone();
    let stdout_task = tokio::spawn(async move {
        use tokio::io::AsyncBufReadExt as _;
        let reader = tokio::io::BufReader::new(stdout_pipe);
        let mut lines = reader.lines();
        let mut collected = String::new();
        while let Ok(Some(line)) = lines.next_line().await {
            update_last_output(&ts_stdout);
            if verbose {
                println!("{}", line);
            }
            // Feed into TUI log buffer
            if let Some(ref ls) = ls_stdout {
                if let Ok(mut s) = ls.lock() {
                    s.push_log(line.clone());
                }
            }
            collected.push_str(&line);
            collected.push('\n');
        }
        collected
    });

    let ts_stderr = last_output_ts.clone();
    let ls_stderr = loop_status.clone();
    let stderr_task = tokio::spawn(async move {
        use tokio::io::AsyncBufReadExt as _;
        let reader = tokio::io::BufReader::new(stderr_pipe);
        let mut lines = reader.lines();
        let mut collected = String::new();
        while let Ok(Some(line)) = lines.next_line().await {
            update_last_output(&ts_stderr);
            if verbose {
                eprint!("{}", line);
            }
            // Feed into TUI log buffer (mark as stderr with prefix)
            if let Some(ref ls) = ls_stderr {
                if let Ok(mut s) = ls.lock() {
                    s.push_log(format!("[err] {}", line));
                }
            }
            collected.push_str(&line);
            collected.push('\n');
        }
        collected
    });

    // ── Main select: child exit | hard timeout | watcher events ──────────────
    let hard_timeout = Duration::from_secs(timeout_secs);

    let outcome: Result<Option<std::process::ExitStatus>> = tokio::select! {
        // Child exited normally
        result = proc.child.wait() => {
            match result {
                Ok(status) => Ok(Some(status)),
                Err(e) => Err(anyhow::anyhow!("Error waiting for agent process: {e}")),
            }
        }

        // Hard wall-clock timeout
        _ = tokio::time::sleep(hard_timeout) => {
            let _ = proc.child.kill().await;
            Err(anyhow::anyhow!("Agent timed out after {}s", timeout_secs))
        }

        // Watcher events (stall, disk, git)
        event = event_rx.recv() => {
            match event {
                Some(WatcherEvent::StallDetected { no_output_secs }) => {
                    let _ = proc.child.kill().await;
                    Err(anyhow::anyhow!(
                        "Agent stalled — no output for {}s (stall timeout: {}s)",
                        no_output_secs,
                        stall_timeout_secs
                    ))
                }
                Some(WatcherEvent::DiskSpaceWarning { free_bytes }) => {
                    eprintln!(
                        "    ⚠️   Low disk space: {:.1} MB free",
                        free_bytes as f64 / 1024.0 / 1024.0
                    );
                    // Continue — non-fatal warning, wait for child
                    Ok(proc.child.wait().await.ok())
                }
                Some(WatcherEvent::GitConflictsDetected) => {
                    eprintln!("    ⚠️   Git merge conflicts detected in working tree");
                    Ok(proc.child.wait().await.ok())
                }
                None => {
                    // Channel closed (watcher task exited); just wait for child
                    Ok(proc.child.wait().await.ok())
                }
            }
        }
    };

    // Collect output (pipes are now closed / tasks will drain quickly)
    let stdout_str = stdout_task.await.unwrap_or_default();
    let stderr_str = stderr_task.await.unwrap_or_default();

    // Write combined log
    let exit_status = outcome?; // propagate any kill/timeout errors
    let exit_code = exit_status.and_then(|s| s.code());

    let log_content = format!(
        "=== EXIT CODE: {:?} ===\n\n=== STDOUT ===\n{}\n\n=== STDERR ===\n{}\n",
        exit_code, stdout_str, stderr_str
    );

    if let Ok(mut log_file) = tfs::File::create(log_path).await {
        let _ = log_file.write_all(log_content.as_bytes()).await;
    }

    // Treat non-zero exit with no stdout as a hard failure
    let success = exit_status.map(|s| s.success()).unwrap_or(false);
    if !success && stdout_str.trim().is_empty() {
        anyhow::bail!(
            "Agent exited with code {:?}: {}",
            exit_code,
            stderr_str.trim()
        );
    }

    Ok(stdout_str)
}

// ── Task scheduling ───────────────────────────────────────────────────────────

/// Return the highest-priority pending task whose dependencies are all complete.
fn pick_next_task(task_list: &TaskList) -> Option<&Task> {
    let complete_ids: HashSet<&str> = task_list
        .tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Complete)
        .map(|t| t.id.as_str())
        .collect();

    task_list
        .tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Pending)
        .filter(|t| t.depends_on.iter().all(|dep| complete_ids.contains(dep.as_str())))
        .min_by_key(|t| t.priority)
}

fn set_task_status(task_list: &mut TaskList, task_id: &str, status: TaskStatus) {
    if let Some(t) = task_list.tasks.iter_mut().find(|t| t.id == task_id) {
        t.status = status;
    }
}

// ── Display helpers ───────────────────────────────────────────────────────────

fn status_icon(s: &TaskStatus) -> &'static str {
    match s {
        TaskStatus::Pending => "⏳",
        TaskStatus::InProgress => "🔄",
        TaskStatus::Complete => "✅",
        TaskStatus::Failed => "❌",
    }
}

fn format_task_table(task_list: &TaskList) -> String {
    let mut out = String::new();
    for t in &task_list.tasks {
        let deps = if t.depends_on.is_empty() {
            "—".to_string()
        } else {
            t.depends_on.join(", ")
        };
        out.push_str(&format!(
            "- [{}/{}] {} — {} (deps: {})\n",
            t.id,
            t.status,
            status_icon(&t.status),
            t.title,
            deps
        ));
    }
    out
}

fn print_task_table(task_list: &TaskList) {
    let total = task_list.tasks.len();
    let complete = task_list
        .tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Complete)
        .count();
    let failed = task_list
        .tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Failed)
        .count();
    let pending = total - complete - failed;

    println!("📋  Task summary:");
    println!("    ✅ Complete : {}/{}", complete, total);
    println!("    ❌ Failed   : {}", failed);
    println!("    ⏳ Remaining: {}", pending);
    println!();

    for t in &task_list.tasks {
        let deps = if t.depends_on.is_empty() {
            "none".to_string()
        } else {
            t.depends_on.join(", ")
        };
        println!(
            "  {} {}  {}  (priority {} | deps: {})",
            status_icon(&t.status),
            t.id,
            t.title,
            t.priority,
            deps
        );
        println!("     {}", t.description);
    }
}
