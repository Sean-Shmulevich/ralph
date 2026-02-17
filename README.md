# Ralph — Autonomous AI Agent Loop Orchestrator

> **Phase 1 MVP** — Core loop for a single PRD, Claude Code agent, git integration.

Ralph reads a PRD (Product Requirements Document), extracts a prioritised task list with an AI agent, then runs an isolated agent loop — one task at a time, each in a fresh process — until the whole PRD is implemented or a circuit-breaker fires.

---

## Quick Start

```bash
# Build
cargo build --release
cp target/release/ralph ~/.local/bin/

# Parse a PRD and preview tasks (no code changes)
ralph parse my-feature.md

# Run the full loop
ralph run my-feature.md

# Run with explicit agent/model, verbose output, 5-minute timeout
ralph run my-feature.md --agent claude --model claude-opus-4-5 --timeout 300 --verbose
```

---

## Commands

### `ralph run <prd.md>`

Run an agent loop for a single PRD.

| Flag | Default | Description |
|------|---------|-------------|
| `--agent` | `claude` | Agent backend to use |
| `--model MODEL` | agent default | Model override |
| `--max-iterations N` | `20` | Hard cap on iterations |
| `--timeout SECS` | `600` | Per-iteration wall-clock timeout |
| `--max-failures N` | `3` | Consecutive failures before stopping |
| `--workdir DIR` | `.` | Project root |
| `--branch NAME` | auto | Git branch name |
| `--no-branch` | — | Skip branch creation and auto-commit |
| `--verbose` | — | Stream agent output to terminal |
| `--dry-run` | — | Parse PRD, print tasks, exit |

### `ralph parse <prd.md>`

Parse a PRD and print the task list without running any agent iterations.

| Flag | Default | Description |
|------|---------|-------------|
| `--output FILE` | stdout | Write tasks.json to this path |

---

## How It Works

```
ralph run prd.md
  │
  ├─ Parse PRD → .ralph/tasks.json   (via claude)
  │
  └─ Loop:
       ├─ Pick highest-priority pending task (deps satisfied)
       ├─ Build prompt (task + PRD + progress log)
       ├─ Spawn claude --dangerously-skip-permissions --print -p "…"
       ├─ Wait with hard timeout (kill on expiry)
       ├─ Capture stdout+stderr → .ralph/logs/iteration-N-TID.log
       ├─ Detect completion: <promise>COMPLETE</promise> in stdout
       ├─ Update .ralph/tasks.json (atomic write)
       ├─ Append .ralph/progress.md
       ├─ Git commit if changes exist
       └─ Repeat until all done or circuit-breaker fires
```

### Completion Detection

Each iteration's stdout is scanned for:

```
<promise>COMPLETE</promise>
```

The agent is instructed to output this token **only** when the task is genuinely done.  
As a secondary signal, Ralph also checks whether `.ralph/tasks.json` was modified by the agent itself.

### Circuit Breaker

If the agent fails to complete a task `--max-failures` times in a row, Ralph stops and records the state. Re-run `ralph run prd.md` to resume — existing `tasks.json` is loaded automatically.

---

## State Directory: `.ralph/`

```
.ralph/
├── tasks.json          # Task list (authoritative source of truth)
├── progress.md         # Append-only log of each iteration's outcome
└── logs/
    ├── iteration-1-T1.log
    ├── iteration-2-T2.log
    └── …
```

`tasks.json` is written atomically (temp file → rename) to survive crashes.

---

## Agents

| Agent | Status | Command |
|-------|--------|---------|
| `claude` | ✅ Phase 1 | `claude --dangerously-skip-permissions --print -p "…"` |
| `gemini` | 🔜 Phase 2 | `gemini -p "…"` |
| `codex`  | 🔜 Phase 2 | `codex --quiet --approval-mode full-auto -p "…"` |

---

## Git Integration

Ralph automatically:
1. Creates branch `ralph/<prd-stem>` (or `--branch NAME`) before the first iteration
2. `git add -A && git commit -m "feat: TN — <title> (ralph)"` after each completed task

Pass `--no-branch` to skip all git operations.

---

## Phases

- **Phase 1** (this) — single PRD loop, Claude Code, git, state
- **Phase 2** — Gemini + Codex agents, stall detection, `ralph status`
- **Phase 3** — parallel PRDs (`ralph watch`), ratatui TUI, `ralph logs --follow`
- **Phase 4** — config file, heuristic parser, crates.io publish

---

## Known Limitations (Phase 1)

- Only Claude Code agent is supported
- No stall detection (output-silence timeout) — comes in Phase 2
- No `ralph status`/`ralph stop` — subprocess is blocking; Ctrl-C will terminate it
- On timeout, the child process is SIGKILL'd but its grandchildren (sub-shells, compilers) may linger briefly

---

## License

MIT
