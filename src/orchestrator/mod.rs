use anyhow::{Context, Result};
use chrono::Utc;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Instant;
use tokio::fs as tfs;
use tokio::io::AsyncWriteExt as _;
use tokio::time::Duration;

use crate::agents::{create_agent, Agent};
use crate::cli::RunArgs;
use crate::git::GitManager;
use crate::hooks::{self, HookConfig, HookEvent, Progress};
use crate::notify::{self, NotifyConfig};
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

    let prd_ref = args.prd.as_ref().context("No PRD file specified")?;
    let prd_path = prd_ref
        .canonicalize()
        .with_context(|| format!("PRD file not found: {}", prd_ref.display()))?;

    // Build state manager — named variant for `ralph watch`, default for `ralph run`
    let state = match &args.state_name {
        Some(name) => StateManager::new_named(&workdir, name)?,
        None => StateManager::new(&workdir)?,
    };

    let git = GitManager::new(&workdir);
    let agent = create_agent(&args.agent, args.model.clone(), args.api_url.clone(), args.api_key.clone())?;

    let is_watch_mode = args.state_name.is_some();

    // Set up webhook hook if configured
    let hook = args
        .hook_url
        .as_ref()
        .map(|url| HookConfig::new(url.clone(), args.hook_token.clone()));

    // Set up OpenClaw notify if configured
    let notify = args.notify.as_ref().and_then(|flag| {
        let prd_name = prd_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let cfg = NotifyConfig::from_env(flag, &prd_name);
        if cfg.is_none() {
            eprintln!("⚠️  --notify requires OPENCLAW_HOOKS_TOKEN env var");
        }
        cfg
    });

    if !is_watch_mode {
        // Interactive `ralph run` — print startup banner
        println!("🚀  Ralph — starting agent loop");
        println!("    PRD:             {}", prd_path.display());
        println!("    Agent:           {}", args.agent);
        println!("    Workdir:         {}", workdir.display());
        println!("    Max iterations:  {}", args.max_iterations);
        println!("    Timeout:         {}s per iteration", args.timeout);
        println!(
            "    Stall timeout:   {}s no-output kill",
            args.stall_timeout
        );
        println!("    Max failures:    {}", args.max_failures);
    }

    if !agent.is_available() {
        anyhow::bail!(
            "Agent '{}' not found on PATH. Install it and try again.",
            args.agent
        );
    }

    // ── Codex sandbox preflight warnings ──────────────────────────────────────
    if args.agent == "codex" {
        let mut warnings = Vec::new();

        // Check for uninstalled Node deps
        if workdir.join("package.json").exists() && !workdir.join("node_modules").exists() {
            warnings.push("package.json found but node_modules/ missing → run `npm install` first");
        }

        // Check for uninstalled Rust deps
        if workdir.join("Cargo.toml").exists() {
            if let Ok(home) = std::env::var("HOME") {
                let registry = PathBuf::from(home).join(".cargo/registry");
                if !registry.exists() {
                    warnings.push("Cargo.toml found but cargo registry missing → run `cargo fetch` first");
                }
            }
        }

        // Check for uninstalled Python deps
        if workdir.join("requirements.txt").exists() && !workdir.join(".venv").exists() && !workdir.join("venv").exists() {
            warnings.push("requirements.txt found but no venv/ → run `pip install -r requirements.txt` in a venv first");
        }

        if !warnings.is_empty() {
            eprintln!();
            eprintln!("⚠️  Codex runs in a sandbox with NO network access.");
            eprintln!("   Dependencies must be pre-installed or Codex will fail.");
            for w in &warnings {
                eprintln!("   • {w}");
            }
            eprintln!();
        }
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
        } else if !is_watch_mode {
            if let Ok(current_branch) = git.current_branch().await {
                println!("    Current branch: {}", current_branch);
            }
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
            // Reset any in_progress tasks back to pending (interrupted previous run)
            let mut fixed = existing;
            let mut reset_count = 0;
            for task in &mut fixed.tasks {
                if task.status == TaskStatus::InProgress {
                    task.status = TaskStatus::Pending;
                    reset_count += 1;
                }
            }
            if reset_count > 0 {
                if !is_watch_mode {
                    println!("⚠️  Reset {reset_count} interrupted task(s) back to pending");
                }
                state.save_tasks(&fixed)?;
            }
            fixed
        }
        None => {
            if !is_watch_mode {
                println!("\n🔍  No tasks.json found — parsing PRD…");
            }
            log_to_status(&args.loop_status, "Parsing PRD…".to_string());
            let tl = parse_prd(
                &prd_path,
                &args.agent,
                args.model.as_deref(),
                args.parse_timeout,
            )
            .await?;
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

    // Agent fallback: track per-task failures to try different agents on retry.
    // After the primary agent fails on a task, we try the next available fallback.
    const FALLBACK_ORDER: &[&str] = &["codex", "gemini", "claude", "opencode"];
    let mut task_fail_count: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let mut active_agent: Box<dyn Agent> = agent;
    let mut active_agent_name: String = args.agent.clone();

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
            fire_hook(
                &hook,
                &notify,
                HookEvent::MaxIterations {
                    max_iterations: args.max_iterations,
                    progress: make_progress(&task_list),
                },
                None,
            )
            .await;
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
            fire_hook(
                &hook,
                &notify,
                HookEvent::CircuitBreaker {
                    consecutive_failures,
                    last_error: "Too many consecutive failures".to_string(),
                    progress: make_progress(&task_list),
                },
                None,
            )
            .await;
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
                if !all_tasks_complete(&task_list) {
                    let msg = "No actionable pending tasks remain, but not all tasks are complete.";
                    if !is_watch_mode {
                        eprintln!("\n⚠️  {msg}");
                    }
                    state.append_progress(&format!("**STOPPED** — {msg}"))?;
                    update_loop_state(&args.loop_status, LoopState::Failed(msg.to_string()));
                    break;
                }

                if !is_watch_mode {
                    println!("\n✅  All tasks complete! PRD implementation finished.");
                }
                state.append_progress("**COMPLETE** — all tasks finished successfully.")?;
                fire_hook(
                    &hook,
                    &notify,
                    HookEvent::AllComplete {
                        total_tasks: task_list.tasks.len() as u32,
                        total_iterations: iteration - 1,
                        total_duration_secs: 0,
                        summary: format!(
                            "All {} tasks completed in {} iterations",
                            task_list.tasks.len(),
                            iteration - 1
                        ),
                        progress: make_progress(&task_list),
                    },
                    None,
                )
                .await;
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

        // Track per-iteration runtime for hooks and terminal output.
        let iteration_started_at = Instant::now();

        // Spawn agent with timeout + stall detection
        let iter_result = run_iteration(
            active_agent.as_ref(),
            &prompt,
            &workdir,
            &log_path,
            args.timeout,
            args.stall_timeout,
            args.verbose && !is_watch_mode,
            args.loop_status.clone(),
        )
        .await;
        let iteration_duration_secs = iteration_started_at.elapsed().as_secs();

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
                        println!(
                            "    ✅  Task {} — complete ({}s)",
                            task.id, iteration_duration_secs
                        );
                    }
                    log_to_status(
                        &args.loop_status,
                        format!("✅ Task {} complete: {}", task.id, task.title),
                    );
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

                    // Fire webhook
                    fire_hook(
                        &hook,
                        &notify,
                        HookEvent::TaskComplete {
                            task_id: task.id.clone(),
                            task_title: task.title.clone(),
                            iteration,
                            duration_secs: iteration_duration_secs,
                            files_changed: vec![],
                            summary: format!(
                                "Task {} — {} completed in iteration {}",
                                task.id, task.title, iteration
                            ),
                            progress: make_progress(&task_list),
                        },
                        None,
                    )
                    .await;

                    // Auto-commit if there are changes
                    if !args.no_branch && git.is_git_repo().await {
                        match git.has_changes().await {
                            Ok(true) => {
                                let msg = format!("feat: {} — {} (ralph)", task.id, task.title);
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

                    fire_hook(
                        &hook,
                        &notify,
                        HookEvent::TaskFailed {
                            task_id: task.id.clone(),
                            task_title: task.title.clone(),
                            iteration,
                            duration_secs: iteration_duration_secs,
                            error: "Task not completed this iteration".to_string(),
                            consecutive_failures,
                            progress: make_progress(&task_list),
                        },
                        None,
                    )
                    .await;
                }
            }

            Err(e) => {
                if !is_watch_mode {
                    eprintln!("    ❌  Iteration error: {e:#}");
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

                fire_hook(
                    &hook,
                    &notify,
                    HookEvent::TaskFailed {
                        task_id: task.id.clone(),
                        task_title: task.title.clone(),
                        iteration,
                        duration_secs: iteration_duration_secs,
                        error: format!("{e}"),
                        consecutive_failures,
                        progress: make_progress(&task_list),
                    },
                    None,
                )
                .await;
            }
        }

        // ── Agent fallback: swap to a different agent after a failure ──────────
        if consecutive_failures > 0 {
            task_fail_count
                .entry(task.id.clone())
                .and_modify(|c| *c += 1)
                .or_insert(1);

            // Find the next fallback agent that isn't the current one and is available
            for &candidate in FALLBACK_ORDER {
                if candidate == active_agent_name {
                    continue;
                }
                if let Ok(new_agent) = create_agent(candidate, args.model.clone(), args.api_url.clone(), args.api_key.clone()) {
                    if new_agent.is_available() {
                        let old_name = active_agent_name.clone();
                        active_agent = new_agent;
                        active_agent_name = candidate.to_string();
                        if !is_watch_mode {
                            eprintln!(
                                "    🔄  Falling back from {} → {} for task {}",
                                old_name, candidate, task.id
                            );
                        }
                        state.append_progress(&format!(
                            "Agent fallback: {} → {} for task {}",
                            old_name, candidate, task.id
                        ))?;
                        break;
                    }
                }
            }

            // If task succeeds on retry, reset back to primary agent
        } else {
            // Success — reset to primary agent if we had fallen back
            if active_agent_name != args.agent {
                if let Ok(primary) = create_agent(&args.agent, args.model.clone(), args.api_url.clone(), args.api_key.clone()) {
                    if !is_watch_mode {
                        eprintln!(
                            "    🔄  Task succeeded — switching back to primary agent ({})",
                            args.agent
                        );
                    }
                    active_agent = primary;
                    active_agent_name = args.agent.clone();
                }
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

// ── Hook helpers ──────────────────────────────────────────────────────────────

fn make_progress(task_list: &TaskList) -> Progress {
    let completed = task_list
        .tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Complete)
        .count() as u32;
    let failed = task_list
        .tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Failed)
        .count() as u32;
    let total = task_list.tasks.len() as u32;
    Progress {
        completed,
        failed,
        remaining: total - completed - failed,
        total,
    }
}

async fn fire_hook(
    hook: &Option<HookConfig>,
    notify_cfg: &Option<NotifyConfig>,
    event: HookEvent,
    log_path: Option<&Path>,
) {
    if let Some(ref config) = hook {
        hooks::send_hook(config, &event).await;
    }
    if let Some(ref config) = notify_cfg {
        notify::send_notify(config, &event, log_path).await;
    }
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
#[allow(clippy::too_many_arguments)]
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
    let (watcher_handle, mut event_rx, last_output_ts) = start_watcher(watcher_config);

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
    watcher_handle.shutdown();

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
        .filter(|t| {
            t.depends_on
                .iter()
                .all(|dep| complete_ids.contains(dep.as_str()))
        })
        .min_by_key(|t| t.priority)
}

fn all_tasks_complete(task_list: &TaskList) -> bool {
    task_list
        .tasks
        .iter()
        .all(|task| task.status == TaskStatus::Complete)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::AgentProcess;
    use crate::cli::RunArgs;
    use crate::state::StateManager;
    use chrono::Utc;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::process::Stdio;
    use tempfile::tempdir;
    use tokio::process::Command;
    use tokio::time::Instant;

    struct MockAgent {
        program: String,
        args: Vec<String>,
    }

    impl MockAgent {
        fn new(program: &str, args: &[&str]) -> Self {
            Self {
                program: program.to_string(),
                args: args.iter().map(|a| a.to_string()).collect(),
            }
        }
    }

    impl Agent for MockAgent {
        fn is_available(&self) -> bool {
            true
        }

        fn spawn(&self, _prompt: &str, workdir: &Path) -> Result<AgentProcess> {
            let mut cmd = Command::new(&self.program);
            cmd.args(&self.args)
                .current_dir(workdir)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            let child = cmd.spawn().context("Failed to spawn mock agent process")?;
            Ok(AgentProcess { child })
        }
    }

    #[tokio::test]
    async fn mock_agent_echo_stdout_is_captured() {
        let dir = tempdir().expect("create tempdir");
        let log_path = dir.path().join("iteration.log");
        let agent = MockAgent::new("echo", &["hello"]);

        let stdout = run_iteration(&agent, "prompt", dir.path(), &log_path, 5, 5, false, None)
            .await
            .expect("run iteration");

        assert_eq!(stdout.trim(), "hello");
    }

    #[tokio::test]
    async fn captures_stderr_separately_from_stdout() {
        let dir = tempdir().expect("create tempdir");
        let log_path = dir.path().join("iteration.log");
        let agent = MockAgent::new("sh", &["-c", "echo out; echo err >&2"]);

        let stdout = run_iteration(&agent, "prompt", dir.path(), &log_path, 5, 5, false, None)
            .await
            .expect("run iteration");

        assert!(stdout.contains("out"));
        assert!(!stdout.contains("err"));

        let log = tokio::fs::read_to_string(&log_path)
            .await
            .expect("read iteration log");
        assert!(log.contains("=== STDOUT ===\nout"));
        assert!(log.contains("=== STDERR ===\nerr"));
    }

    #[tokio::test]
    async fn hard_timeout_kills_agent_process() {
        let dir = tempdir().expect("create tempdir");
        let log_path = dir.path().join("iteration.log");
        let agent = MockAgent::new("sh", &["-c", "sleep 10"]);
        let started = Instant::now();

        let err = run_iteration(&agent, "prompt", dir.path(), &log_path, 1, 60, false, None)
            .await
            .expect_err("iteration should time out");

        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(15),
            "timeout should kill within reasonable time, elapsed={elapsed:?}"
        );
        assert!(
            err.to_string().contains("Agent timed out after 1s"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn detects_completion_signal_in_captured_output() {
        let dir = tempdir().expect("create tempdir");
        std::fs::write(
            dir.path().join("response.txt"),
            "work done\n<promise>COMPLETE</promise>\n",
        )
        .expect("write response file");

        let log_path = dir.path().join("iteration.log");
        let agent = MockAgent::new("cat", &["response.txt"]);

        let stdout = run_iteration(&agent, "prompt", dir.path(), &log_path, 5, 5, false, None)
            .await
            .expect("run iteration");

        assert!(stdout.contains("<promise>COMPLETE</promise>"));
    }

    fn write_fake_codex(workdir: &Path) -> std::path::PathBuf {
        let bin_dir = workdir.join("bin");
        fs::create_dir_all(&bin_dir).expect("create bin dir");

        let codex_path = bin_dir.join("codex");
        fs::write(
            &codex_path,
            r#"#!/bin/sh
mode="${MOCK_CODEX_MODE:-complete}"
if [ "$mode" = "complete" ]; then
  printf 'done\n<promise>COMPLETE</promise>\n'
elif [ "$mode" = "slow_complete" ]; then
  sleep 2
  printf 'done\n<promise>COMPLETE</promise>\n'
elif [ "$mode" = "incomplete" ]; then
  printf 'still working\n'
else
  printf 'agent error\n' 1>&2
  exit 1
fi
"#,
        )
        .expect("write fake codex");

        let mut perms = fs::metadata(&codex_path)
            .expect("stat fake codex")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&codex_path, perms).expect("chmod fake codex");

        // Create identical stubs for all fallback agents so tests don't hit real binaries
        for name in &["gemini", "claude", "opencode"] {
            let p = bin_dir.join(name);
            fs::copy(&codex_path, &p).expect(&format!("copy fake {name}"));
            let mut pm = fs::metadata(&p).expect("stat").permissions();
            pm.set_mode(0o755);
            fs::set_permissions(&p, pm).expect("chmod");
        }

        bin_dir
    }

    fn seed_tasks(workdir: &Path, task_status: TaskStatus) {
        let state = StateManager::new(workdir).expect("create state manager");
        let now = Utc::now();
        let task_list = TaskList {
            version: 1,
            prd_path: workdir.join("prd.md").to_string_lossy().to_string(),
            created_at: now,
            updated_at: now,
            tasks: vec![Task {
                id: "T6".to_string(),
                title: "Orchestrator loop integration tests".to_string(),
                description: "T6 body".to_string(),
                priority: 1,
                status: task_status,
                depends_on: vec![],
                completed_at: None,
                notes: None,
            }],
        };
        state.save_tasks(&task_list).expect("save seeded tasks");
    }

    fn seed_custom_tasks(workdir: &Path, tasks: Vec<Task>) {
        let state = StateManager::new(workdir).expect("create state manager");
        let now = Utc::now();
        let task_list = TaskList {
            version: 1,
            prd_path: workdir.join("prd.md").to_string_lossy().to_string(),
            created_at: now,
            updated_at: now,
            tasks,
        };
        state.save_tasks(&task_list).expect("save seeded tasks");
    }

    #[test]
    fn all_tasks_complete_requires_every_task_to_be_complete() {
        let now = Utc::now();
        let task_list = TaskList {
            version: 1,
            prd_path: "prd.md".to_string(),
            created_at: now,
            updated_at: now,
            tasks: vec![
                Task {
                    id: "T1".to_string(),
                    title: "done".to_string(),
                    description: "done".to_string(),
                    priority: 1,
                    status: TaskStatus::Complete,
                    depends_on: vec![],
                    completed_at: None,
                    notes: None,
                },
                Task {
                    id: "T2".to_string(),
                    title: "in progress".to_string(),
                    description: "wip".to_string(),
                    priority: 2,
                    status: TaskStatus::InProgress,
                    depends_on: vec![],
                    completed_at: None,
                    notes: None,
                },
            ],
        };

        assert!(
            !all_tasks_complete(&task_list),
            "complete + in_progress must not be treated as all complete"
        );
    }

    fn run_args(
        prd_path: &Path,
        workdir: &Path,
        max_iterations: u32,
        max_failures: u32,
    ) -> RunArgs {
        RunArgs {
            prd: Some(prd_path.to_path_buf()),
            template: None,
            agent: "codex".to_string(),
            model: None,
            max_iterations,
            timeout: 5,
            stall_timeout: 5,
            parse_timeout: 5,
            max_failures,
            workdir: Some(workdir.to_path_buf()),
            branch: None,
            no_branch: true,
            verbose: false,
            dry_run: false,
            hook_url: None,
            hook_token: None,
            notify: None,
            api_url: None,
            api_key: None,
            state_name: None,
            loop_status: None,
            cancel_flag: None,
        }
    }

    #[tokio::test]
    async fn single_iteration_marks_task_complete_and_updates_progress() {
        let _guard = crate::global_env_lock().lock().expect("lock env mutation");
        let dir = tempdir().expect("create tempdir");
        let prd_path = dir.path().join("prd.md");
        fs::write(&prd_path, "# PRD").expect("write prd");
        seed_tasks(dir.path(), TaskStatus::Pending);
        let bin_dir = write_fake_codex(dir.path());

        let old_path = std::env::var("PATH").ok();
        let new_path = match old_path.as_deref() {
            Some(path) if !path.is_empty() => format!("{}:{}", bin_dir.display(), path),
            _ => bin_dir.display().to_string(),
        };
        std::env::set_var("PATH", new_path);
        std::env::set_var("MOCK_CODEX_MODE", "complete");

        run(run_args(&prd_path, dir.path(), 5, 3))
            .await
            .expect("run orchestrator");

        if let Some(path) = old_path {
            std::env::set_var("PATH", path);
        } else {
            std::env::remove_var("PATH");
        }
        std::env::remove_var("MOCK_CODEX_MODE");

        let state = StateManager::new(dir.path()).expect("create state manager");
        let tasks = state
            .load_tasks()
            .expect("load tasks")
            .expect("tasks should exist");
        assert_eq!(tasks.tasks[0].status, TaskStatus::Complete);

        let progress = fs::read_to_string(&state.progress_file).expect("read progress");
        assert!(progress.contains("**Task T6 complete**"));
        assert!(progress.contains("**COMPLETE** — all tasks finished successfully."));

        let logs: Vec<_> = fs::read_dir(&state.logs_dir)
            .expect("read logs dir")
            .collect::<Result<_, _>>()
            .expect("collect logs");
        assert_eq!(logs.len(), 1, "one loop iteration should run");
    }

    #[tokio::test]
    async fn three_consecutive_incomplete_iterations_trigger_circuit_breaker() {
        let _guard = crate::global_env_lock().lock().expect("lock env mutation");
        let dir = tempdir().expect("create tempdir");
        let prd_path = dir.path().join("prd.md");
        fs::write(&prd_path, "# PRD").expect("write prd");
        seed_tasks(dir.path(), TaskStatus::Pending);
        let bin_dir = write_fake_codex(dir.path());

        let old_path = std::env::var("PATH").ok();
        let new_path = match old_path.as_deref() {
            Some(path) if !path.is_empty() => format!("{}:{}", bin_dir.display(), path),
            _ => bin_dir.display().to_string(),
        };
        std::env::set_var("PATH", new_path);
        std::env::set_var("MOCK_CODEX_MODE", "incomplete");

        run(run_args(&prd_path, dir.path(), 10, 3))
            .await
            .expect("run orchestrator");

        if let Some(path) = old_path {
            std::env::set_var("PATH", path);
        } else {
            std::env::remove_var("PATH");
        }
        std::env::remove_var("MOCK_CODEX_MODE");

        let state = StateManager::new(dir.path()).expect("create state manager");
        let tasks = state
            .load_tasks()
            .expect("load tasks")
            .expect("tasks should exist");
        assert_eq!(
            tasks.tasks[0].status,
            TaskStatus::Pending,
            "incomplete iterations should reset task to pending"
        );

        let progress = fs::read_to_string(&state.progress_file).expect("read progress");
        assert!(progress.contains("Consecutive failures: 1/3"));
        assert!(progress.contains("Consecutive failures: 2/3"));
        assert!(progress.contains("Consecutive failures: 3/3"));
        assert!(
            progress.contains(
                "**STOPPED** — circuit breaker after 3 consecutive failures (iteration 4)."
            ),
            "circuit breaker entry missing from progress.md"
        );

        let logs: Vec<_> = fs::read_dir(&state.logs_dir)
            .expect("read logs dir")
            .collect::<Result<_, _>>()
            .expect("collect logs");
        assert_eq!(
            logs.len(),
            3,
            "circuit breaker should stop after 3 failures"
        );
    }

    #[tokio::test]
    async fn all_tasks_complete_exits_early_without_iteration() {
        let _guard = crate::global_env_lock().lock().expect("lock env mutation");
        let dir = tempdir().expect("create tempdir");
        let prd_path = dir.path().join("prd.md");
        fs::write(&prd_path, "# PRD").expect("write prd");
        seed_tasks(dir.path(), TaskStatus::Complete);
        let bin_dir = write_fake_codex(dir.path());

        let old_path = std::env::var("PATH").ok();
        let new_path = match old_path.as_deref() {
            Some(path) if !path.is_empty() => format!("{}:{}", bin_dir.display(), path),
            _ => bin_dir.display().to_string(),
        };
        std::env::set_var("PATH", new_path);
        std::env::set_var("MOCK_CODEX_MODE", "complete");

        run(run_args(&prd_path, dir.path(), 5, 3))
            .await
            .expect("run orchestrator");

        if let Some(path) = old_path {
            std::env::set_var("PATH", path);
        } else {
            std::env::remove_var("PATH");
        }
        std::env::remove_var("MOCK_CODEX_MODE");

        let state = StateManager::new(dir.path()).expect("create state manager");
        let progress = fs::read_to_string(&state.progress_file).expect("read progress");
        assert!(progress.contains("**COMPLETE** — all tasks finished successfully."));

        let logs: Vec<_> = fs::read_dir(&state.logs_dir)
            .expect("read logs dir")
            .collect::<Result<_, _>>()
            .expect("collect logs");
        assert!(
            logs.is_empty(),
            "no iteration should run when all tasks are complete"
        );
    }

    #[tokio::test]
    async fn complete_and_in_progress_tasks_do_not_exit_early() {
        let _guard = crate::global_env_lock().lock().expect("lock env mutation");
        let dir = tempdir().expect("create tempdir");
        let prd_path = dir.path().join("prd.md");
        fs::write(&prd_path, "# PRD").expect("write prd");
        seed_custom_tasks(
            dir.path(),
            vec![
                Task {
                    id: "T1".to_string(),
                    title: "already done".to_string(),
                    description: "done".to_string(),
                    priority: 1,
                    status: TaskStatus::Complete,
                    depends_on: vec![],
                    completed_at: None,
                    notes: None,
                },
                Task {
                    id: "T2".to_string(),
                    title: "in progress task".to_string(),
                    description: "wip".to_string(),
                    priority: 2,
                    status: TaskStatus::InProgress,
                    depends_on: vec![],
                    completed_at: None,
                    notes: None,
                },
            ],
        );
        let bin_dir = write_fake_codex(dir.path());

        let old_path = std::env::var("PATH").ok();
        let new_path = match old_path.as_deref() {
            Some(path) if !path.is_empty() => format!("{}:{}", bin_dir.display(), path),
            _ => bin_dir.display().to_string(),
        };
        std::env::set_var("PATH", new_path);
        std::env::set_var("MOCK_CODEX_MODE", "complete");

        run(run_args(&prd_path, dir.path(), 5, 3))
            .await
            .expect("run orchestrator");

        if let Some(path) = old_path {
            std::env::set_var("PATH", path);
        } else {
            std::env::remove_var("PATH");
        }
        std::env::remove_var("MOCK_CODEX_MODE");

        let state = StateManager::new(dir.path()).expect("create state manager");
        let progress = fs::read_to_string(&state.progress_file).expect("read progress");
        assert!(
            progress.contains("**Task T2 complete**"),
            "ralph should run an iteration for the in_progress task rather than exiting early"
        );

        let logs: Vec<_> = fs::read_dir(&state.logs_dir)
            .expect("read logs dir")
            .collect::<Result<_, _>>()
            .expect("collect logs");
        assert_eq!(logs.len(), 1, "one iteration should run before completion");
    }

    #[tokio::test]
    async fn lock_file_is_written_with_pid_and_removed_on_clean_exit() {
        let _guard = crate::global_env_lock().lock().expect("lock env mutation");
        let dir = tempdir().expect("create tempdir");
        let prd_path = dir.path().join("prd.md");
        fs::write(&prd_path, "# PRD").expect("write prd");
        seed_tasks(dir.path(), TaskStatus::Pending);
        let bin_dir = write_fake_codex(dir.path());

        let old_path = std::env::var("PATH").ok();
        let new_path = match old_path.as_deref() {
            Some(path) if !path.is_empty() => format!("{}:{}", bin_dir.display(), path),
            _ => bin_dir.display().to_string(),
        };
        std::env::set_var("PATH", new_path);
        std::env::set_var("MOCK_CODEX_MODE", "slow_complete");

        let args = run_args(&prd_path, dir.path(), 5, 3);
        let handle = tokio::spawn(async move { run(args).await });

        let state = StateManager::new(dir.path()).expect("create state manager");
        let expected_pid = std::process::id();

        let started = Instant::now();
        let mut seen_pid = None;
        while started.elapsed() < Duration::from_secs(2) {
            if let Some(lock) = state.read_lock().expect("read lock during run") {
                seen_pid = Some(lock.pid);
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        let observed_pid = seen_pid.expect("lock file should be visible while run is active");
        assert_eq!(
            observed_pid, expected_pid,
            "lock file should contain current PID"
        );

        handle
            .await
            .expect("join orchestrator task")
            .expect("run orchestrator");

        assert!(
            !state.lock_file.exists(),
            "lock file should be removed after normal exit"
        );

        if let Some(path) = old_path {
            std::env::set_var("PATH", path);
        } else {
            std::env::remove_var("PATH");
        }
        std::env::remove_var("MOCK_CODEX_MODE");
    }
}
