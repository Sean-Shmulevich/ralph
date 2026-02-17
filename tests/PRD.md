# Tests for Ralph CLI

## Overview

Add integration and unit tests for the Ralph CLI — a Rust binary that orchestrates AI coding agents in loops. Tests should verify core logic without requiring actual AI agent calls (mock the agent spawning).

## Tech Stack
- Rust, tokio, clap, serde_json
- Test framework: built-in `#[cfg(test)]` + `cargo test`
- Use `tempdir` / `tempfile` for isolated test directories
- Mock agent processes with simple bash scripts that echo known output

## Test Suites

### T1: State Management (unit)
- `StateManager::new()` creates `.ralph/` directory
- `write_tasks()` + `read_tasks()` roundtrip preserves all fields
- Atomic write: tasks.json is never partially written (simulate crash)
- `pick_next_task()` returns highest priority pending task
- `pick_next_task()` respects `depends_on` (skips tasks with incomplete deps)
- `mark_complete()` sets status to complete and persists
- `append_progress()` appends to progress.md without overwriting

### T2: Task Parsing & Validation (unit)
- Valid tasks.json deserializes correctly
- Missing required fields produce clear errors
- Empty task list is handled gracefully
- Duplicate task IDs are detected
- Circular dependencies in `depends_on` are detected or handled

### T3: CLI Argument Parsing (unit)
- `ralph run prd.md` parses correctly
- `ralph run prd.md --agent gemini --max-iterations 5 --timeout 300` sets all fields
- `ralph parse prd.md` works
- `ralph status` works with no args
- `ralph stop --all` parses the --all flag
- `ralph watch a.md b.md --parallel 2` parses multiple PRDs
- Unknown flags produce helpful errors

### T4: Git Manager (integration)
- Creates branch from current HEAD
- Switches to existing branch if it exists
- `commit_all()` stages and commits all changes
- Commit message matches expected format
- Works in a fresh git repo with no prior commits

### T5: Agent Trait & Mock Agent (integration)
- Create a `MockAgent` that spawns `echo "hello"` as its process
- Verify `AgentProcess` captures stdout correctly
- Verify stderr is captured separately
- Agent timeout kills process after deadline
- Completion signal `<promise>COMPLETE</promise>` is detected in output

### T6: Orchestrator Loop (integration)
- Single iteration with mock agent completes one task
- Task marked complete after successful iteration
- Failed iteration increments failure counter
- 3 consecutive failures triggers circuit breaker (loop stops)
- All-tasks-complete triggers early exit
- Progress.md is updated after each iteration

### T7: Watcher (integration)
- Stall detection fires after configured timeout with no output
- Disk space warning triggers when below threshold (mock df)
- Git conflict detection finds UU lines in status
- Watcher shutdown via handle drop

### T8: Lock File (integration)
- Lock file written on start with correct PID
- Lock file cleaned up on normal exit
- Stale lock detected when PID is dead
- Multiple lock files for watch mode (.ralph-name/)

## Acceptance Criteria
- All tests pass with `cargo test`
- No tests require network access or real AI agents
- Tests complete in <30 seconds
- Each module has its own test submodule
