# Ralph Usage Guide

Practical recipes for running Ralph with different AI agents and OpenClaw notifications.

---

## Table of Contents

- [Prerequisites](#prerequisites)
- [Agent Overview](#agent-overview)
- [OpenClaw Notifications](#openclaw-notifications)
- [Recommended Workflows](#recommended-workflows)
- [Agent-Specific Setup](#agent-specific-setup)
- [Configuration File](#configuration-file)
- [Troubleshooting](#troubleshooting)

---

## Prerequisites

```bash
# Install Ralph
cargo install ralph-loop
# Or build from source
git clone https://github.com/openclaw/ralph.git && cd ralph
cargo build --release && cp target/release/ralph ~/.local/bin/

# Verify
ralph doctor
```

You also need at least one agent CLI installed. Run `ralph doctor` to check which are available.

---

## Agent Overview

| Agent | CLI | Best For | Caveats |
|-------|-----|----------|---------|
| **Codex** | `codex` | Most reliable for implementation | Sandboxed — no internet. Pre-install deps first. |
| **Gemini** | `gemini` | Good all-rounder, generous context | Requires Google auth (`gemini auth login`) |
| **Claude** | `claude` | High quality output | `--print` mode stalls on complex tasks. Use `--stall-timeout 30`. |
| **OpenCode** | `opencode` | Local/open-source models | Requires separate setup |
| **API** | *(curl-based)* | Direct Anthropic API calls | Text-only — no file editing. Good for parsing, not implementation. |

### Fallback Order

When an agent fails a task, Ralph automatically tries the next agent in fallback order:

```
codex → gemini → api → claude → opencode
```

---

## OpenClaw Notifications

Ralph can send real-time progress updates to Discord or Telegram via an OpenClaw gateway.

### Setup

1. Get your OpenClaw hooks token (from your gateway config or `openclaw status`)
2. Set it as an environment variable:

```bash
export OPENCLAW_HOOKS_TOKEN="your-hooks-token-here"
```

3. Use the `--notify` flag with `channel:target_id` format:

```bash
# Discord channel
ralph run prd.md --agent codex --notify discord:1468795968156729427

# Telegram chat
ralph run prd.md --agent codex --notify telegram:5979047659
```

### What Gets Notified

- ✅ Task completed successfully
- ❌ Task failed (with failure count)
- 🛑 Circuit breaker triggered (3+ consecutive failures)
- 🏁 All tasks completed
- ⚠️ Max iterations reached

### Combining with Webhooks

You can use both `--notify` (OpenClaw) and `--hook-url` (generic webhook) simultaneously:

```bash
ralph run prd.md \
  --notify discord:CHANNEL_ID \
  --hook-url https://your-webhook.com/endpoint \
  --hook-token your-secret
```

Notifications are fire-and-forget — failures never block Ralph.

---

## Recommended Workflows

### 1. Standard Run (Codex, recommended)

Pre-install dependencies, then let Codex handle everything in its sandbox:

```bash
cd /path/to/project
npm install          # or: cargo fetch, pip install -r requirements.txt
ralph run prd.md --agent codex --notify discord:CHANNEL_ID
```

### 2. Redundant Multi-Agent (fallback on failure)

Start with Codex; if it fails 3 times, Ralph falls back to Gemini, then API, then Claude:

```bash
ralph run prd.md --agent codex --max-failures 3 --notify discord:CHANNEL_ID
```

This is the default behavior — Ralph tries the next agent automatically.

### 3. Fast Iteration with Short Stall Timeout

Claude either starts outputting immediately or never will. Use a short stall timeout:

```bash
ralph run prd.md --agent claude --stall-timeout 30 --notify discord:CHANNEL_ID
```

### 4. Parallel PRDs with Watch Mode

Run multiple PRDs simultaneously with a live TUI dashboard:

```bash
ralph watch prd-auth.md prd-api.md prd-ui.md \
  --agent codex \
  --notify discord:CHANNEL_ID
```

### 5. Dry Run First (preview tasks)

Always preview what Ralph will do before committing:

```bash
ralph parse prd.md           # Just print extracted tasks
ralph run prd.md --dry-run   # Parse and show plan, no execution
```

### 6. API Agent with Claude Max Proxy

If you have a Claude Max subscription, use the proxy for parsing or light tasks:

```bash
# Start the Max proxy (separate terminal)
# See: https://github.com/rynfar/opencode-claude-max-proxy

ralph run prd.md \
  --agent api \
  --api-url http://localhost:3456 \
  --api-key dummy
```

> **Note:** The API agent can only return text — it can't edit files or run commands. Use it for PRD parsing or combine it as a fallback behind Codex/Gemini.

### 7. Resume a Stopped Run

Ralph stores state in `.ralph/tasks.json`. Re-run the same command and it picks up where it left off:

```bash
# First run (interrupted or failed)
ralph run prd.md --agent codex

# Resume — completed tasks are skipped
ralph run prd.md --agent codex
```

---

## Agent-Specific Setup

### Codex (OpenAI)

```bash
# Install
npm install -g @openai/codex

# Auth
export OPENAI_API_KEY="sk-..."

# Critical: pre-install project deps (Codex sandbox has no internet)
cd /path/to/project
npm install        # Node projects
cargo fetch        # Rust projects
pip install -r requirements.txt  # Python projects
```

### Gemini (Google)

```bash
# Install
npm install -g @google/gemini-cli

# Auth (interactive browser flow)
gemini auth login
```

### Claude (Anthropic)

```bash
# Install
npm install -g @anthropic-ai/claude-code

# Auth — either API key or OAuth
export ANTHROPIC_API_KEY="sk-..."
# Or: claude login (OAuth, but can cause stall issues)
```

### OpenCode

```bash
# Install your preferred local model wrapper
# Must accept: opencode --print -p "prompt"
```

---

## Configuration File

Create `ralph.toml` in your project root or `~/.config/ralph/config.toml` for global defaults:

```toml
[defaults]
agent = "codex"
max_iterations = 20
timeout = 600
stall_timeout = 120
max_failures = 3

[hooks]
url = "https://your-webhook.com/endpoint"
token = "your-secret"
```

CLI flags always override config file values.

### Recommended Config for OpenClaw Users

```toml
[defaults]
agent = "codex"
max_iterations = 25
stall_timeout = 60
max_failures = 3
```

Set `OPENCLAW_HOOKS_TOKEN` in your shell profile and use `--notify` per-run.

---

## Troubleshooting

### Agent stalls with no output

```bash
# Use shorter stall timeout — kills stuck agents faster
ralph run prd.md --stall-timeout 30

# Check which agents are healthy
ralph doctor
```

### Codex fails with missing modules

Pre-install all dependencies before running:

```bash
npm install && ralph run prd.md --agent codex
```

### Claude `--print` hangs

Known issue. Claude's `--print` mode stalls randomly on complex tasks. Workarounds:

1. Use `--stall-timeout 30` (kill early, retry)
2. Switch to Codex (most reliable)
3. Use the API agent with a Claude Max proxy

### Tasks stuck as `in_progress`

Ralph auto-resets `in_progress` tasks to `pending` on startup. Just re-run.

### Notification not arriving

1. Check `OPENCLAW_HOOKS_TOKEN` is set: `echo $OPENCLAW_HOOKS_TOKEN`
2. Verify gateway is running: `curl http://127.0.0.1:18789/health`
3. Check format: `--notify discord:CHANNEL_ID` (no spaces)

---

## PRD Tips

- Keep tasks atomic — one task per context window
- Use `ralph init` for a starter template
- Number tasks explicitly: `## T1: Setup database schema`
- Include acceptance criteria so Ralph knows when a task passes
- See [references/prd-format.md](../references/prd-format.md) if it exists

---

*For contributing, see [CONTRIBUTING.md](../CONTRIBUTING.md). Licensed under MIT.*
