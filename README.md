# Ralph — Autonomous AI Agent for Software Development

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

Ralph is an autonomous AI agent that reads a PRD, extracts tasks, and implements them iteratively until completion. Each iteration spawns a fresh agent with clean context, avoiding context rot.

Supports multiple agents (Codex, Gemini, Claude, OpenCode) with automatic fallback on failure, and optional real-time notifications via [OpenClaw](https://openclaw.ai).

---

## Installation

### Build from source

Requires Rust 1.70+ and `git`.

1.  Clone the repository:
    ```bash
    git clone https://github.com/openclaw/ralph.git
    cd ralph
    ```
2.  Build the project:
    ```bash
    cargo build --release
    ```
3.  Add the executable to your PATH:
    ```bash
    cp target/release/ralph ~/.local/bin/
    ```
    (Ensure `~/.local/bin` is in your system's PATH environment variable).

---

## Quick Start

Go from a PRD to implemented code in three easy steps:

1.  **Create your PRD:** Start with a `prd.md` file defining your project.
    ```bash
    ralph init
    # Edit prd.md with your project details
    ```

2.  **Parse and preview tasks:** See what tasks Ralph extracts without making any changes.
    ```bash
    ralph parse prd.md
    ```

3.  **Run the autonomous loop:** Let Ralph implement the tasks.
    ```bash
    ralph run prd.md
    ```

---

## CLI Reference

### `ralph init`

Initializes a new `prd.md` template in the current directory. If `prd.md` already exists, it will abort.

### `ralph parse <PRD_FILE>`

Parses a Product Requirements Document (PRD) and prints the extracted task list. This command does not modify any code.

| Flag | Default | Description |
|------|---------|-------------|
| `--output FILE` | `stdout` | Write tasks.json to this path |

### `ralph run <PRD_FILE>`

Runs the autonomous agent loop to implement tasks from the specified PRD.

| Flag | Default | Description |
|------|---------|-------------|
| `--agent` | `codex` | Agent backend to use (`codex`, `gemini`, `claude`, `opencode`, `api`) |
| `--model MODEL` | agent default | Specific model override for the agent |
| `--max-iterations N` | `20` | Hard cap on the number of agent iterations |
| `--timeout SECS` | `600` | Per-iteration wall-clock timeout in seconds |
| `--stall-timeout SECS` | `120` | Timeout for agent output silence in seconds (before next fallback) |
| `--max-failures N` | `3` | Consecutive failures before stopping the loop |
| `--workdir DIR` | `.` | Project root directory for Ralph to operate in |
| `--branch NAME` | auto | Git branch name to create for the changes |
| `--no-branch` | — | Skip branch creation and auto-commit operations |
| `--verbose` | — | Stream agent output to the terminal for debugging |
| `--dry-run` | — | Parse PRD, print tasks, and exit without running the loop |
| `--parse-timeout` | `120` | Timeout for the PRD parsing phase in seconds |
| `--notify` | — | OpenClaw notifications (`discord:CHANNEL_ID` or `telegram:CHAT_ID`) |
| `--hook-url` | — | Generic webhook URL for event POSTs |
| `--hook-token` | — | Bearer token for webhook auth |
| `--api-url` | anthropic | Base URL for API agent (e.g. `http://localhost:3456` for Max proxy) |
| `--api-key` | env | API key for API agent (default: `ANTHROPIC_API_KEY` env var) |

### `ralph doctor`

Checks the local environment for common issues and dependencies required by Ralph.

Outputs a summary table including:
-   **Agents installed**: Checks for `claude`, `codex`, `gemini`, `opencode` in PATH.
-   **Agents authenticated**: Attempts a quick probe to verify authentication for found agents.
-   **Git status**: Verifies Git installation and if the current directory is a Git repository.
-   **Disk space**: Reports available disk space.

---

## Agent Setup Guides

Ralph supports multiple AI agents. Here's how to set them up:

### Claude

Ralph interfaces with Claude via the Anthropic `claude` CLI.

1.  **Install the Claude CLI:**
    ```bash
    pip install anthropic-cli
    ```
2.  **Set your API key:**
    Ensure your Anthropic API key is set as an environment variable:
    ```bash
    export ANTHROPIC_API_KEY="sk-..."
    ```
    Alternatively, configure it using the Anthropic CLI directly.

### Gemini

Ralph interfaces with Gemini via the Google `gemini` CLI.

1.  **Install the Gemini CLI:**
    ```bash
    npm install -g @google/gemini-cli
    ```
    (Requires Node.js and npm)
2.  **Authenticate:**
    Run the authentication command:
    ```bash
    gemini auth login
    ```
    Follow the prompts to authenticate with your Google account.

### Codex (OpenAI compatible)

Ralph can use any OpenAI-compatible API, often referred to as "Codex" in some contexts. This typically involves setting an API key.

1.  **Set your API key:**
    ```bash
    export OPENAI_API_KEY="sk-..."
    ```
    Ensure the `openai` CLI is available in your PATH if you intend to use it directly, though Ralph only requires the environment variable.

### OpenCode (Local/Open-source models)

For local or open-source models, Ralph expects an executable named `opencode` in your PATH that accepts similar `--print -p "..."` arguments.

Set up your preferred local model (e.g., Llama, Code Llama, GPT4All) to be accessible via an `opencode` wrapper script that conforms to this interface.

---

## Webhook Hooks

Ralph can dispatch events to a configurable webhook URL for real-time monitoring or integration with other systems.

To enable webhooks, add a `[hooks]` section to your `ralph.toml` configuration file:

```toml
[hooks]
url = "https://your-webhook-url.com/endpoint"
token = "your-optional-secret-token" # Used in an 'X-Webhook-Token' header
```

Ralph will send `POST` requests to the specified `url` with a JSON payload containing details about each significant event (e.g., task started, task completed, iteration failed). If a `token` is provided, it will be included in the `X-Webhook-Token` header for verification by your webhook receiver.

---

## Configuration File (`ralph.toml`)

Ralph can be configured using a `ralph.toml` file. It looks for this file in the current directory first, then in `~/.config/ralph/config.toml`. CLI flags always override configuration file values, which in turn override built-in defaults.

Example `ralph.toml`:

```toml
# ralph.toml
[defaults]
agent = "gemini"            # Default agent to use if not specified by CLI
max_iterations = 20         # Maximum number of agent iterations
timeout = 600               # Per-iteration timeout in seconds
stall_timeout = 120         # Agent output silence timeout in seconds
max_failures = 3            # Consecutive failures before stopping

[hooks]
url = "https://your-webhook-url.com/endpoint"
token = "your-optional-secret-token"
```

A full list of configurable options matches the `ralph run` command-line arguments where applicable.

---

## Contributing

Ralph is an open-source project. We welcome contributions! Please see our `CONTRIBUTING.md` file for details on how to set up your development environment, run tests, and submit pull requests.

---

## License

Ralph is licensed under the MIT License.

Copyright (c) 2026 Sean Shmulevich