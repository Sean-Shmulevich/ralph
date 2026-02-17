use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::state::SharedLoopStatus;

/// Ralph — Orchestrates AI coding agents in isolated loops to implement PRD features
#[derive(Parser)]
#[command(name = "ralph", version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Run an agent loop for a single PRD
    Run(RunArgs),
    /// Parse a PRD and print the task list (no execution)
    Parse(ParseArgs),
    /// Show status of running ralph loops
    Status(StatusArgs),
    /// Run multiple PRDs in parallel with a live TUI dashboard
    Watch(WatchArgs),
    /// Stream logs for a named loop
    Logs(LogsArgs),
    /// Gracefully stop a running loop (or all loops)
    Stop(StopArgs),
}

#[derive(Args, Debug)]
pub struct RunArgs {
    /// Path to the PRD markdown file
    pub prd: PathBuf,

    /// Agent to use (claude, gemini, codex)
    #[arg(long, default_value = "claude")]
    pub agent: String,

    /// Model override passed to the agent binary
    #[arg(long)]
    pub model: Option<String>,

    /// Maximum number of iterations before stopping
    #[arg(long, default_value = "20")]
    pub max_iterations: u32,

    /// Per-iteration timeout in seconds (hard kill)
    #[arg(long, default_value = "600")]
    pub timeout: u64,

    /// Kill agent if it produces no output for this many seconds (default: 120 = 2 min)
    #[arg(long, default_value = "120")]
    pub stall_timeout: u64,

    /// Maximum consecutive failures before circuit-breaking
    #[arg(long, default_value = "3")]
    pub max_failures: u32,

    /// Project directory (defaults to current directory)
    #[arg(long)]
    pub workdir: Option<PathBuf>,

    /// Git branch name for this loop (auto-generated from PRD name if omitted)
    #[arg(long)]
    pub branch: Option<String>,

    /// Skip git branch creation and auto-commit
    #[arg(long)]
    pub no_branch: bool,

    /// Stream agent output to the terminal in real time
    #[arg(long, short)]
    pub verbose: bool,

    /// Parse PRD and show tasks without running any agent
    #[arg(long)]
    pub dry_run: bool,

    // ── Internal fields set programmatically by `ralph watch` ─────────────────

    /// Name override for the state directory.
    /// If set, state lives in `.ralph-<state_name>/` instead of `.ralph/`.
    #[arg(skip)]
    pub state_name: Option<String>,

    /// Shared live status updated by the orchestrator for the TUI.
    #[arg(skip)]
    pub loop_status: Option<SharedLoopStatus>,

    /// Cancellation flag — set to `true` to request a graceful stop.
    #[arg(skip)]
    pub cancel_flag: Option<Arc<AtomicBool>>,
}

#[derive(Args, Debug)]
pub struct ParseArgs {
    /// Path to the PRD markdown file
    pub prd: PathBuf,

    /// Agent to use for parsing
    #[arg(long, default_value = "claude")]
    pub agent: String,

    /// Model override passed to the agent binary
    #[arg(long)]
    pub model: Option<String>,

    /// Write tasks.json to this path instead of printing
    #[arg(long, short)]
    pub output: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct StatusArgs {
    /// Path to the project directory to check (defaults to current directory)
    #[arg(long)]
    pub workdir: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct WatchArgs {
    /// PRD files to run in parallel
    #[arg(required = true)]
    pub prds: Vec<PathBuf>,

    /// Maximum number of concurrent loops (default: min(CPU count, 4))
    #[arg(long)]
    pub parallel: Option<usize>,

    /// Agent to use for all loops
    #[arg(long, default_value = "claude")]
    pub agent: String,

    /// Model override passed to the agent binary
    #[arg(long)]
    pub model: Option<String>,

    /// Maximum iterations per loop
    #[arg(long, default_value = "20")]
    pub max_iterations: u32,

    /// Per-iteration timeout in seconds
    #[arg(long, default_value = "600")]
    pub timeout: u64,

    /// Stall timeout in seconds (no output → kill)
    #[arg(long, default_value = "120")]
    pub stall_timeout: u64,

    /// Maximum consecutive failures per loop before circuit-breaking
    #[arg(long, default_value = "3")]
    pub max_failures: u32,

    /// Shared working directory for all loops (defaults to current directory)
    #[arg(long)]
    pub workdir: Option<PathBuf>,

    /// Disable the TUI dashboard (plain progress output)
    #[arg(long)]
    pub no_tui: bool,

    /// Stream agent output to terminal (only useful with --no-tui)
    #[arg(long, short)]
    pub verbose: bool,
}

#[derive(Args, Debug)]
pub struct LogsArgs {
    /// Loop name (PRD filename stem, e.g. "auth-system").
    /// Omit to read from the default .ralph/ directory.
    pub name: Option<String>,

    /// Follow (tail) the active log in real time
    #[arg(long, short)]
    pub follow: bool,

    /// Project directory (defaults to current directory)
    #[arg(long)]
    pub workdir: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct StopArgs {
    /// Name of the loop to stop (PRD filename stem).
    /// Omit to stop the default .ralph/ loop.
    pub name: Option<String>,

    /// Stop all running loops found in the workdir
    #[arg(long)]
    pub all: bool,

    /// Project directory to search for lock files (defaults to current directory)
    #[arg(long)]
    pub workdir: Option<PathBuf>,
}
