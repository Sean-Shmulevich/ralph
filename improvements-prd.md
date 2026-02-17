# Ralph CLI Improvements — Open Source Readiness

## Overview

Harden and polish the Ralph CLI for open source release. Fix known bugs, improve resilience, add developer experience features, and prepare CI/docs.

## Codebase Context

- Rust binary at the project root
- Modules: agents/, orchestrator/, parser/, state/, git/, watcher/, watch/, tui/, hooks, logs, stop, cli
- 4 agents supported: claude, codex, gemini, opencode
- Tests are inline `#[cfg(test)]` modules in each source file
- Currently 39 passing tests

## Bug Fixes

### Fix `in_progress` completion check
The orchestrator's "all tasks complete" check counts `in_progress` tasks as done. The `pick_next_task()` only picks `Pending` tasks, so an `in_progress` task gets skipped and Ralph falsely reports completion. Fix: the completion check in the main loop (`pick_next_task` returning `None`) should explicitly verify that ALL tasks have status `Complete`, not just that no `Pending` tasks remain. Add a test that seeds one Complete + one InProgress task and verifies Ralph does NOT exit early.

### Add parse timeout
The `parse_prd()` function has no timeout — if Claude hangs, Ralph hangs forever. Wrap the agent call in `tokio::time::timeout` with a 120-second default. If it times out, fall through to the next fallback agent. Add `--parse-timeout` CLI flag to override.

## Resilience

### Detect Claude Code OAuth-only installs
Before spawning `claude --print`, do a quick probe: run `claude --print -p "test" 2>&1` with a 10-second timeout. If stderr contains "Invalid API key" or "API key", print a helpful message explaining that `--print` mode requires an API key and suggest setting `ANTHROPIC_API_KEY` or using a different agent. Then fall through to fallback agents. This avoids the cryptic "Agent failed during PRD parsing" error.

### Per-iteration duration tracking
Currently `duration_secs` in hook events is always 0. Track `Instant::now()` before spawning the agent in each iteration and compute elapsed after. Pass the real duration to hook events and print it in the terminal output (e.g. "Task T3 — complete (42s)").

## Developer Experience

### `ralph init` command
Add a new CLI subcommand `ralph init` that creates a `prd.md` template in the current directory. The template should have the standard sections: Overview, Requirements, Acceptance Criteria, with TODO placeholders. If `prd.md` already exists, abort with a message. Add `InitArgs` to cli.rs.

### `ralph doctor` command
Add a subcommand that checks the environment:
- Which agents are installed (claude, codex, gemini, opencode) — check PATH
- Which agents are authenticated (try a quick probe like `claude --print -p "hi"` with 10s timeout)
- Git installed and version
- Current directory is a git repo
- Disk space
- Print a summary table. Add `DoctorArgs` to cli.rs.

### Config file support (`ralph.toml`)
Look for `ralph.toml` in the current directory, then `~/.config/ralph/config.toml`. Parse with the `toml` crate. Support these fields:
```toml
[defaults]
agent = "codex"
max_iterations = 20
timeout = 600
stall_timeout = 120
max_failures = 3

[hooks]
url = "https://example.com/webhook"
token = "secret"
```
CLI flags override config file values. Config file values override built-in defaults. Add `toml` to Cargo.toml dependencies.

## Open Source Prep

### Clean compiler warnings
Run `cargo fix --bin ralph` and manually fix any remaining warnings. The current 7 warnings include unused imports, dead code, and non_snake_case. All warnings should be resolved.

### README with usage examples
Rewrite README.md with:
- One-line description and badges placeholder
- Install section (`cargo install ralph-loop` or build from source)
- Quick start (3 commands to go from PRD to implemented code)
- Full CLI reference for all subcommands
- Agent setup guides (claude, codex, gemini, opencode)
- Webhook hooks documentation
- Config file format
- Contributing section pointing to CONTRIBUTING.md
- License (MIT)

### Add CONTRIBUTING.md
Standard open source contributing guide: how to build, run tests, add a new agent, submit PRs. Keep it concise.

### Add LICENSE file
MIT license with copyright "2026 Sean Shmulevich"

### GitHub Actions CI
Create `.github/workflows/ci.yml`:
- Trigger on push to main and PRs
- Matrix: ubuntu-latest + macos-latest
- Steps: checkout, install Rust (stable), `cargo test`, `cargo clippy -- -D warnings`
- Cache cargo registry and target directory

## Acceptance Criteria
- All existing 39 tests still pass
- New tests added for bug fixes (in_progress check, parse timeout)
- `cargo clippy -- -D warnings` passes with no warnings
- `ralph init`, `ralph doctor` work
- `ralph.toml` config loading works
- README is comprehensive and accurate
- CI workflow file is valid YAML
