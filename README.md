# Ralph

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust CI](https://github.com/Sean-Shmulevich/ralph/actions/workflows/ci.yml/badge.svg)](https://github.com/Sean-Shmulevich/ralph/actions)

Autonomous AI coding agent orchestrator. Give it a PRD, get working code.

Ralph reads a Product Requirements Document, breaks it into tasks, and runs AI coding agents in isolated loops until everything's implemented. Each iteration gets a fresh context window — no context rot, no token bloat.

```
$ ralph run prd.md --agent codex --notify discord:CHANNEL_ID

🚀  Ralph v0.1.0 — 7 tasks parsed from prd.md
━━━ Iteration 1 ━━━  Task T1 — Create EasyPost server client
    ✅  Task T1 complete (42s)
━━━ Iteration 2 ━━━  Task T2 — Add from-address config
    ✅  Task T2 complete (38s)
...
🎉  All 7 tasks complete! 7 iterations, 4m12s total
```

## Why Ralph?

**Context rot kills long coding sessions.** After enough back-and-forth, AI agents lose track of what they've done, hallucinate file contents, and repeat mistakes. Ralph solves this by:

1. **Fresh context every iteration** — each task gets a clean agent with full attention
2. **Automatic fallback** — if Codex fails, Ralph tries Gemini, then Claude, then OpenCode
3. **Real-time notifications** — get Discord/Telegram updates as tasks complete
4. **Circuit breaker** — stops wasting tokens after consecutive failures
5. **Resume anywhere** — state is persisted; re-run the same command to pick up where you left off

## Install

### Download a release binary

Grab the latest from [GitHub Releases](https://github.com/Sean-Shmulevich/ralph/releases):

```bash
# Linux (x86_64)
curl -sL https://github.com/Sean-Shmulevich/ralph/releases/latest/download/ralph-v0.1.0-x86_64-unknown-linux-gnu.tar.gz | tar xz
sudo mv ralph-*/ralph /usr/local/bin/

# macOS (Apple Silicon)
curl -sL https://github.com/Sean-Shmulevich/ralph/releases/latest/download/ralph-v0.1.0-aarch64-apple-darwin.tar.gz | tar xz
sudo mv ralph-*/ralph /usr/local/bin/
```

### Build from source

Requires Rust 1.70+:

```bash
git clone https://github.com/Sean-Shmulevich/ralph.git
cd ralph
cargo build --release
cp target/release/ralph ~/.local/bin/
```

## Quick Start

```bash
# 1. Check your setup
ralph doctor

# 2. Create a PRD
ralph init
# Edit prd.md with your requirements

# 3. Preview tasks (no execution)
ralph parse prd.md

# 4. Pre-install deps (important for Codex — its sandbox has no internet)
npm install   # or: cargo fetch, pip install -r requirements.txt

# 5. Run it
ralph run prd.md --agent codex
```

## Agents

Ralph supports multiple AI coding agents with automatic fallback:

| Agent | Best For | Setup |
|-------|----------|-------|
| **Codex** (default) | Most reliable | `npm i -g @openai/codex` + `OPENAI_API_KEY` |
| **Gemini** | Generous context window | `npm i -g @google/gemini-cli` + `gemini auth login` |
| **Claude** | Highest quality output | `npm i -g @anthropic-ai/claude-code` + API key or OAuth |
| **OpenCode** | Local/open-source models | Your own `opencode` binary in PATH |
| **API** | Direct Anthropic API (text-only) | `ANTHROPIC_API_KEY` or `--api-url` for proxies |

When an agent fails a task, Ralph automatically tries the next available one:

```
codex → gemini → claude → opencode
```

> **Note:** Claude's `--print` mode can stall on complex tasks. Use `--stall-timeout 30` to fail fast.

> **Note:** The API agent returns text only (no file editing). It's useful for PRD parsing but not implementation.

## Commands

### `ralph run <PRD>`

Run the agent loop for a single PRD:

```bash
ralph run prd.md                          # defaults: --agent codex, 20 iterations
ralph run prd.md --agent gemini           # use Gemini
ralph run prd.md --max-iterations 30      # more iterations
ralph run prd.md --stall-timeout 30       # kill stalled agents faster
ralph run prd.md --dry-run                # parse tasks, don't execute
ralph run prd.md -v                       # stream agent output to terminal
```

| Flag | Default | Description |
|------|---------|-------------|
| `--agent` | `codex` | Agent to use |
| `--max-iterations` | `20` | Max loop iterations |
| `--timeout` | `600` | Per-iteration hard kill (seconds) |
| `--stall-timeout` | `120` | Kill if no output for this long (seconds) |
| `--max-failures` | `3` | Consecutive failures before circuit breaker |
| `--workdir` | `.` | Project directory |
| `--branch` | auto | Git branch name |
| `--no-branch` | — | Skip git branching and auto-commit |
| `--notify` | — | OpenClaw notifications (see below) |
| `--hook-url` | — | Generic webhook URL |

### `ralph watch <PRD...>`

Run multiple PRDs in parallel with a live TUI dashboard:

```bash
ralph watch auth.md api.md ui.md --agent codex --parallel 3
```

### `ralph status`

Show all running Ralph loops system-wide:

```bash
$ ralph status
🟢  2 active loop(s) system-wide

    🟢 [default] PID 12345
       Dir:      /home/user/myproject
       PRD:      prd.md
       Agent:    codex
       Task:     T3 — Implement auth middleware
       Progress: 2/7 done
       Time:     4m 32s
```

### `ralph stop [name] [--all]`

Stop running loops:

```bash
ralph stop            # stop the default loop in cwd
ralph stop --all      # stop ALL loops system-wide
ralph stop myloop     # stop a named loop
```

### Other commands

```bash
ralph init            # create a starter prd.md template
ralph parse prd.md    # parse and display tasks without running
ralph doctor          # check agents, auth, git, disk space
ralph logs <name>     # stream logs for a watch loop
```

## Notifications

### OpenClaw (Discord / Telegram)

```bash
export OPENCLAW_GATEWAY_TOKEN="your-token"
ralph run prd.md --notify discord:CHANNEL_ID
ralph run prd.md --notify telegram:CHAT_ID
```

Events sent: ✅ task complete, ❌ task failed, ⚠️ circuit breaker, 🎉 all done.

### Generic Webhooks

```bash
ralph run prd.md --hook-url https://your-server.com/webhook --hook-token secret
```

Ralph POSTs JSON events to your URL with an `X-Webhook-Token` header.

## Configuration

Create `ralph.toml` in your project root (or `~/.config/ralph/config.toml` globally):

```toml
[defaults]
agent = "codex"
max_iterations = 25
stall_timeout = 60
max_failures = 3

[hooks]
url = "https://your-webhook.com/endpoint"
token = "your-secret"
```

CLI flags always override config file values.

## How It Works

```
┌─────────────┐     ┌──────────┐     ┌─────────────┐
│   PRD.md    │────▶│  Parser  │────▶│ tasks.json  │
└─────────────┘     └──────────┘     └──────┬──────┘
                                            │
                    ┌───────────────────────┐│
                    │    Orchestrator Loop  ││
                    │                       ▼│
                    │  ┌─────────────────┐  ││
                    │  │ Pick next task  │  ││
                    │  └────────┬────────┘  ││
                    │           │           ││
                    │  ┌────────▼────────┐  ││
                    │  │ Spawn agent     │  ││  ┌──────────┐
                    │  │ (fresh context) │◀─┼┼──│ Watcher  │
                    │  └────────┬────────┘  ││  │ (stall,  │
                    │           │           ││  │  disk,   │
                    │  ┌────────▼────────┐  ││  │  git)    │
                    │  │ Check output    │  ││  └──────────┘
                    │  │ for COMPLETE    │  ││
                    │  └────────┬────────┘  ││
                    │           │           ││
                    │     ┌─────▼─────┐    ││
                    │     │  ✅ / ❌   │    ││
                    │     └─────┬─────┘    ││
                    │           │          ││
                    │    ┌──────▼───────┐  ││
                    │    │ Git commit   │  ││
                    │    │ Update state │  ││
                    │    │ Notify       │  ││
                    │    └──────┬───────┘  ││
                    │           │          ││
                    │           ▼          ││
                    │    next iteration    ││
                    └───────────────────────┘│
                                            │
                    ┌───────────────────────┐│
                    │  On failure: fallback ││
                    │  codex → gemini →     ││
                    │  claude → opencode    ││
                    └───────────────────────┘│
```

1. **Parse** — AI agent extracts atomic tasks from your PRD into `.ralph/tasks.json`
2. **Loop** — each iteration spawns a fresh agent that reads the task, implements it, and signals `<promise>COMPLETE</promise>` when done
3. **Watch** — background watchdog monitors for stalls, disk space, and git conflicts
4. **Fallback** — if the agent fails, Ralph tries the next available agent
5. **Stop** — circuit breaker triggers after N consecutive failures

State lives in `.ralph/` — tasks, progress log, iteration logs. Git history + `progress.md` are the only memory between iterations.

## Writing Good PRDs

Ralph works best with well-structured PRDs:

- **Number tasks explicitly**: `## T1: Setup database schema`
- **Keep tasks atomic**: each should fit in one context window
- **Include acceptance criteria**: so the agent knows when it's done
- **Specify dependencies**: `(deps: T1, T2)` helps Ralph order tasks
- **Be specific**: "Add login endpoint with JWT" > "Add auth"

See [`examples/hello-world.md`](examples/hello-world.md) for a minimal example, or run `ralph init` for a template.

## Resuming

Ralph persists all state. Re-run the exact same command to resume:

```bash
# First run (interrupted)
ralph run prd.md --agent codex
# Ctrl+C or crash

# Resume — completed tasks are skipped
ralph run prd.md --agent codex
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup and guidelines.

## License

MIT — see [LICENSE](LICENSE).
