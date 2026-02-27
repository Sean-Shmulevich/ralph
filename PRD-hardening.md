# PRD: Ralph Orchestrator Hardening

Harden the Ralph orchestrator's retry, fallback, and error handling in `src/orchestrator/mod.rs` and related files. All changes must preserve existing tests and add new ones where appropriate.

## Tasks

### T1: Same-agent retry with exponential backoff before fallback
Currently, any single failure immediately triggers agent fallback. Instead, retry the same agent up to N times (default 2, configurable via `--retries-before-fallback`) with exponential backoff (2s, 4s, 8s) before falling back to a different agent. Add the `retries_before_fallback` field to `RunArgs` and `cli.rs`. The retry counter should reset when switching to a new task.

### T2: Append failure context to retry prompts
When retrying a failed task (same agent or fallback), append a "Previous Attempt" section to the prompt that includes: (1) which attempt number this is, (2) the error message or reason for failure, (3) the last 50 lines of stdout from the failed attempt. This gives the agent context about what went wrong. Add a `previous_attempt_context` parameter to the prompt template.

### T3: Configurable fallback chain via CLI flag
Replace the hardcoded `FALLBACK_ORDER` constant with a `--fallback` CLI flag that accepts a comma-separated list of agent names (e.g. `--fallback gemini,codex`). If not provided, default to the current order minus the primary agent. Store as `fallback_agents: Vec<String>` in `RunArgs`. Validate that all specified agents are known agent names.

### T4: Unify Failed and Pending task handling
Currently, tasks set to `TaskStatus::Failed` (from iteration errors) are never retried, while incomplete tasks (reset to `Pending`) are. Change the behavior so that `Failed` tasks are also eligible for retry in `pick_next_task()` — but only after all `Pending` tasks have been attempted. Add a `failure_count` field to the `Task` struct (in `state/mod.rs`) to track how many times a task has failed, and skip tasks that exceed `--max-task-failures` (default 5).

### T5: Rate limit detection with automatic wait-and-retry
Detect HTTP 429 / "rate limit" / "usage limit" errors in agent stdout/stderr. When detected: (1) do NOT count it as a failure, (2) parse `Retry-After` or `resets_in_seconds` if available, (3) sleep for the indicated duration (or 60s default), (4) retry the same task with the same agent. Log the wait to the user. Add a helper function `detect_rate_limit(output: &str) -> Option<Duration>` in a new file `src/rate_limit.rs`.

### T6: Smarter stall detection — distinguish thinking from truly stalled
Currently, stall detection only checks stdout/stderr timestamps. Improve it: (1) also monitor the agent's CPU usage via `/proc/<pid>/stat` — if CPU is active, extend the stall grace period by 2x, (2) check if the working directory has file modifications (via `fs::metadata` mtime checks on the workdir) — active file writes mean the agent is working. Update `src/watcher/mod.rs` with these additional signals.

### T7: Better exit code handling and error categorization
Add an `ErrorKind` enum in `src/orchestrator/mod.rs` with variants: `Timeout`, `Stall`, `RateLimit`, `AgentCrash`, `AgentIncomplete`, `Unknown`. Categorize iteration failures into these kinds. Use the kind to decide behavior: `RateLimit` → wait and retry (T5), `Timeout`/`Stall` → increase timeout for next attempt by 50%, `AgentCrash` → count toward fallback, `AgentIncomplete` → retry same agent first. Log the error kind in progress.md and hook events.
