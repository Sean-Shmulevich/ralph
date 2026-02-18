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
    /// Create a starter PRD template in the current directory
    Init(InitArgs),
    /// Check local environment health (agents, auth, git, disk)
    Doctor(DoctorArgs),
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
    /// Manage reusable PRD templates
    Template(TemplateArgs),
}

#[derive(Args, Debug)]
pub struct TemplateArgs {
    #[command(subcommand)]
    pub command: TemplateCommands,
}

#[derive(Subcommand, Debug)]
pub enum TemplateCommands {
    /// Save a PRD file as a reusable template
    Save {
        /// Template name (e.g. "code-review")
        name: String,
        /// Path to the PRD markdown file to save
        prd: PathBuf,
    },
    /// List all saved templates
    List {
        /// Show full descriptions (not just names)
        #[arg(short, long)]
        verbose: bool,
    },
    /// Show the full content of a template
    Show {
        /// Template name
        name: String,
    },
    /// Remove a saved template
    Remove {
        /// Template name
        name: String,
    },
}

#[derive(Args, Debug)]
pub struct RunArgs {
    /// Path to the PRD markdown file (or use --template)
    #[arg(required_unless_present = "template")]
    pub prd: Option<PathBuf>,

    /// Use a saved template instead of a PRD file (see: ralph template list)
    #[arg(long, conflicts_with = "prd")]
    pub template: Option<String>,

    /// Agent to use (claude, gemini, codex)
    #[arg(long, default_value = "codex")]
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

    /// Timeout in seconds for PRD parsing (falls back to next available agent)
    #[arg(long, default_value = "120")]
    pub parse_timeout: u64,

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

    /// Webhook URL to POST events to (task complete, failures, etc.)
    #[arg(long)]
    pub hook_url: Option<String>,

    /// Bearer token for webhook authentication
    #[arg(long)]
    pub hook_token: Option<String>,

    /// Send progress notifications to OpenClaw channel (e.g. discord:CHANNEL_ID)
    /// Requires OPENCLAW_HOOKS_TOKEN env var.
    #[arg(long)]
    pub notify: Option<String>,

    /// Base URL for API agent (default: https://api.anthropic.com, or http://localhost:3456 for Max proxy)
    #[arg(long)]
    pub api_url: Option<String>,

    /// API key for API agent (default: reads ANTHROPIC_API_KEY env var)
    #[arg(long)]
    pub api_key: Option<String>,

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
    /// Path to the PRD markdown file (or use --template)
    #[arg(required_unless_present = "template")]
    pub prd: Option<PathBuf>,

    /// Use a saved template instead of a PRD file
    #[arg(long, conflicts_with = "prd")]
    pub template: Option<String>,

    /// Agent to use for parsing
    #[arg(long, default_value = "codex")]
    pub agent: String,

    /// Model override passed to the agent binary
    #[arg(long)]
    pub model: Option<String>,

    /// Timeout in seconds for PRD parsing (falls back to next available agent)
    #[arg(long, default_value = "120")]
    pub parse_timeout: u64,

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
    #[arg(long, default_value = "codex")]
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

    /// Webhook URL to POST events to
    #[arg(long)]
    pub hook_url: Option<String>,

    /// Bearer token for webhook authentication
    #[arg(long)]
    pub hook_token: Option<String>,

    /// Send progress notifications to OpenClaw channel (e.g. discord:CHANNEL_ID)
    #[arg(long)]
    pub notify: Option<String>,

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

#[derive(Args, Debug)]
pub struct InitArgs {}

#[derive(Args, Debug)]
pub struct DoctorArgs {
    /// Directory to check (defaults to current directory)
    #[arg(long)]
    pub workdir: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::{Cli, Commands};
    use clap::Parser;
    use std::path::PathBuf;

    #[test]
    fn run_subcommand_parses_prd_path() {
        let cli = Cli::try_parse_from(["ralph", "run", "prd.md"]).expect("parse should succeed");

        match cli.command {
            Commands::Run(args) => {
                assert_eq!(args.prd, Some(PathBuf::from("prd.md")));
            }
            _ => panic!("expected run command"),
        }
    }

    #[test]
    fn init_subcommand_parses_without_args() {
        let cli = Cli::try_parse_from(["ralph", "init"]).expect("parse should succeed");

        match cli.command {
            Commands::Init(_args) => {}
            _ => panic!("expected init command"),
        }
    }

    #[test]
    fn doctor_subcommand_parses_without_args() {
        let cli = Cli::try_parse_from(["ralph", "doctor"]).expect("parse should succeed");

        match cli.command {
            Commands::Doctor(args) => {
                assert!(args.workdir.is_none());
            }
            _ => panic!("expected doctor command"),
        }
    }

    #[test]
    fn run_subcommand_parses_agent_iterations_and_timeout_flags() {
        let cli = Cli::try_parse_from([
            "ralph",
            "run",
            "prd.md",
            "--agent",
            "gemini",
            "--max-iterations",
            "5",
            "--timeout",
            "300",
            "--parse-timeout",
            "45",
        ])
        .expect("parse should succeed");

        match cli.command {
            Commands::Run(args) => {
                assert_eq!(args.prd, Some(PathBuf::from("prd.md")));
                assert_eq!(args.agent, "gemini");
                assert_eq!(args.max_iterations, 5);
                assert_eq!(args.timeout, 300);
                assert_eq!(args.parse_timeout, 45);
            }
            _ => panic!("expected run command"),
        }
    }

    #[test]
    fn parse_subcommand_parses_prd_path() {
        let cli = Cli::try_parse_from(["ralph", "parse", "prd.md", "--parse-timeout", "30"])
            .expect("parse should succeed");

        match cli.command {
            Commands::Parse(args) => {
                assert_eq!(args.prd, Some(PathBuf::from("prd.md")));
                assert_eq!(args.parse_timeout, 30);
            }
            _ => panic!("expected parse command"),
        }
    }

    #[test]
    fn status_subcommand_parses_without_args() {
        let cli = Cli::try_parse_from(["ralph", "status"]).expect("parse should succeed");

        match cli.command {
            Commands::Status(args) => {
                assert!(args.workdir.is_none());
            }
            _ => panic!("expected status command"),
        }
    }

    #[test]
    fn stop_subcommand_parses_all_flag() {
        let cli = Cli::try_parse_from(["ralph", "stop", "--all"]).expect("parse should succeed");

        match cli.command {
            Commands::Stop(args) => {
                assert!(args.all);
                assert!(args.name.is_none());
            }
            _ => panic!("expected stop command"),
        }
    }

    #[test]
    fn watch_subcommand_parses_multiple_prds_and_parallel() {
        let cli = Cli::try_parse_from(["ralph", "watch", "a.md", "b.md", "--parallel", "2"])
            .expect("parse should succeed");

        match cli.command {
            Commands::Watch(args) => {
                assert_eq!(
                    args.prds,
                    vec![PathBuf::from("a.md"), PathBuf::from("b.md")]
                );
                assert_eq!(args.parallel, Some(2));
            }
            _ => panic!("expected watch command"),
        }
    }

    #[test]
    fn unknown_flags_produce_helpful_errors() {
        let err = match Cli::try_parse_from(["ralph", "run", "prd.md", "--bogus"]) {
            Ok(_) => panic!("unknown flag should fail"),
            Err(err) => err,
        };
        assert_eq!(err.kind(), clap::error::ErrorKind::UnknownArgument);

        let rendered = err.to_string();
        assert!(rendered.contains("--bogus"));
        assert!(rendered.to_ascii_lowercase().contains("usage"));
    }
}
