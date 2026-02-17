mod agents;
mod cli;
mod config;
mod git;
mod hooks;
mod notify;
mod logs;
mod orchestrator;
mod parser;
mod state;
mod stop;
mod tui;
mod watch;

use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
mod watcher;

use anyhow::{Context, Result};
use chrono::Utc;
use clap::parser::ValueSource;
use clap::{CommandFactory, Parser};
use cli::{Cli, Commands};
use tokio::process::Command;
use tokio::time::{timeout, Duration};

#[tokio::main]
async fn main() -> Result<()> {
    let argv: Vec<std::ffi::OsString> = std::env::args_os().collect();
    let cli = Cli::parse_from(argv.clone());
    let matches = Cli::command().get_matches_from(argv);
    let config = config::load_config()?;

    match cli.command {
        Commands::Init(args) => {
            init_prd(args).await?;
        }
        Commands::Doctor(args) => {
            run_doctor(args).await?;
        }
        Commands::Run(mut args) => {
            if let Some(run_matches) = matches.subcommand_matches("run") {
                apply_run_config(&mut args, config.as_ref(), run_matches);
            }
            orchestrator::run(args).await?;
        }
        Commands::Parse(mut args) => {
            if let Some(parse_matches) = matches.subcommand_matches("parse") {
                apply_parse_config(&mut args, config.as_ref(), parse_matches);
            }
            parser::parse_and_print(args).await?;
        }
        Commands::Status(args) => {
            show_status(args).await?;
        }
        Commands::Watch(mut args) => {
            if let Some(watch_matches) = matches.subcommand_matches("watch") {
                apply_watch_config(&mut args, config.as_ref(), watch_matches);
            }
            watch::watch(args).await?;
        }
        Commands::Logs(args) => {
            logs::show_logs(args).await?;
        }
        Commands::Stop(args) => {
            stop::stop_loops(args).await?;
        }
    }

    Ok(())
}

fn apply_run_config(
    args: &mut cli::RunArgs,
    config: Option<&config::RalphConfig>,
    matches: &clap::ArgMatches,
) {
    let Some(config) = config else {
        return;
    };

    if let Some(defaults) = &config.defaults {
        if !was_provided_by_cli(matches, "agent") {
            if let Some(agent) = &defaults.agent {
                args.agent = agent.clone();
            }
        }
        if !was_provided_by_cli(matches, "max_iterations") {
            if let Some(value) = defaults.max_iterations {
                args.max_iterations = value;
            }
        }
        if !was_provided_by_cli(matches, "timeout") {
            if let Some(value) = defaults.timeout {
                args.timeout = value;
            }
        }
        if !was_provided_by_cli(matches, "stall_timeout") {
            if let Some(value) = defaults.stall_timeout {
                args.stall_timeout = value;
            }
        }
        if !was_provided_by_cli(matches, "max_failures") {
            if let Some(value) = defaults.max_failures {
                args.max_failures = value;
            }
        }
    }

    if let Some(hooks) = &config.hooks {
        if !was_provided_by_cli(matches, "hook_url") {
            if let Some(url) = &hooks.url {
                args.hook_url = Some(url.clone());
            }
        }
        if !was_provided_by_cli(matches, "hook_token") {
            if let Some(token) = &hooks.token {
                args.hook_token = Some(token.clone());
            }
        }
    }
}

fn apply_parse_config(
    args: &mut cli::ParseArgs,
    config: Option<&config::RalphConfig>,
    matches: &clap::ArgMatches,
) {
    let Some(config) = config else {
        return;
    };
    let Some(defaults) = &config.defaults else {
        return;
    };

    if !was_provided_by_cli(matches, "agent") {
        if let Some(agent) = &defaults.agent {
            args.agent = agent.clone();
        }
    }
}

fn apply_watch_config(
    args: &mut cli::WatchArgs,
    config: Option<&config::RalphConfig>,
    matches: &clap::ArgMatches,
) {
    let Some(config) = config else {
        return;
    };

    if let Some(defaults) = &config.defaults {
        if !was_provided_by_cli(matches, "agent") {
            if let Some(agent) = &defaults.agent {
                args.agent = agent.clone();
            }
        }
        if !was_provided_by_cli(matches, "max_iterations") {
            if let Some(value) = defaults.max_iterations {
                args.max_iterations = value;
            }
        }
        if !was_provided_by_cli(matches, "timeout") {
            if let Some(value) = defaults.timeout {
                args.timeout = value;
            }
        }
        if !was_provided_by_cli(matches, "stall_timeout") {
            if let Some(value) = defaults.stall_timeout {
                args.stall_timeout = value;
            }
        }
        if !was_provided_by_cli(matches, "max_failures") {
            if let Some(value) = defaults.max_failures {
                args.max_failures = value;
            }
        }
    }

    if let Some(hooks) = &config.hooks {
        if !was_provided_by_cli(matches, "hook_url") {
            if let Some(url) = &hooks.url {
                args.hook_url = Some(url.clone());
            }
        }
        if !was_provided_by_cli(matches, "hook_token") {
            if let Some(token) = &hooks.token {
                args.hook_token = Some(token.clone());
            }
        }
    }
}

fn was_provided_by_cli(matches: &clap::ArgMatches, arg_id: &str) -> bool {
    matches.value_source(arg_id) == Some(ValueSource::CommandLine)
}

#[derive(Debug)]
struct DoctorRow {
    check: String,
    status: String,
    details: String,
}

#[derive(Debug, PartialEq, Eq)]
enum AgentAuthStatus {
    Authenticated,
    NotAuthenticated(String),
    TimedOut,
    ProbeFailed(String),
}

async fn init_prd(_args: cli::InitArgs) -> Result<()> {
    let workdir = std::env::current_dir().context("Cannot resolve current directory")?;
    create_prd_template(&workdir)?;
    println!("Created {}", workdir.join("prd.md").display());
    Ok(())
}

fn create_prd_template(workdir: &std::path::Path) -> Result<()> {
    let prd_path = workdir.join("prd.md");
    if prd_path.exists() {
        anyhow::bail!(
            "{} already exists. Aborting to avoid overwriting it.",
            prd_path.display()
        );
    }

    let template = r#"# Overview

TODO: Describe the problem, target users, and desired outcomes.

# Requirements

TODO: List functional and non-functional requirements.

# Acceptance Criteria

TODO: Define testable completion criteria.
"#;

    std::fs::write(&prd_path, template)
        .with_context(|| format!("Failed to write {}", prd_path.display()))?;

    Ok(())
}

async fn run_doctor(args: cli::DoctorArgs) -> Result<()> {
    let workdir = args
        .workdir
        .unwrap_or_else(|| PathBuf::from("."))
        .canonicalize()
        .context("Cannot resolve workdir — does it exist?")?;

    let mut rows = Vec::new();

    for agent in ["claude", "codex", "gemini", "opencode"] {
        if !command_on_path(agent) {
            rows.push(DoctorRow {
                check: format!("agent:{agent}"),
                status: "MISSING".to_string(),
                details: "not found on PATH".to_string(),
            });
            continue;
        }

        let (status, details) = match probe_agent_auth(agent).await {
            AgentAuthStatus::Authenticated => ("OK", "installed + authenticated".to_string()),
            AgentAuthStatus::NotAuthenticated(msg) => ("WARN", msg),
            AgentAuthStatus::TimedOut => ("WARN", "probe timed out after 10s".to_string()),
            AgentAuthStatus::ProbeFailed(msg) => ("WARN", msg),
        };

        rows.push(DoctorRow {
            check: format!("agent:{agent}"),
            status: status.to_string(),
            details,
        });
    }

    if !command_on_path("git") {
        rows.push(DoctorRow {
            check: "git".to_string(),
            status: "FAIL".to_string(),
            details: "git not found on PATH".to_string(),
        });
        rows.push(DoctorRow {
            check: "git-repo".to_string(),
            status: "N/A".to_string(),
            details: "git is not installed".to_string(),
        });
    } else {
        let git_version = detect_git_version().await;
        rows.push(DoctorRow {
            check: "git".to_string(),
            status: "OK".to_string(),
            details: git_version
                .unwrap_or_else(|e| format!("installed (version probe failed: {e})")),
        });

        let (status, details) = match is_git_repo(&workdir).await {
            Ok(true) => ("OK", format!("{} is a git repository", workdir.display())),
            Ok(false) => (
                "WARN",
                format!("{} is not a git repository", workdir.display()),
            ),
            Err(e) => ("WARN", format!("failed to check repo status: {e}")),
        };
        rows.push(DoctorRow {
            check: "git-repo".to_string(),
            status: status.to_string(),
            details,
        });
    }

    let disk_row = match check_disk_space(&workdir).await {
        Ok((total_kib, avail_kib, used_percent)) => DoctorRow {
            check: "disk".to_string(),
            status: if used_percent >= 95 { "WARN" } else { "OK" }.to_string(),
            details: format!(
                "{} free / {} total ({}% used)",
                kib_to_human(avail_kib),
                kib_to_human(total_kib),
                used_percent
            ),
        },
        Err(e) => DoctorRow {
            check: "disk".to_string(),
            status: "WARN".to_string(),
            details: format!("failed to check disk space: {e}"),
        },
    };
    rows.push(disk_row);

    println!("Ralph doctor report for {}", workdir.display());
    println!();
    print_doctor_table(&rows);

    Ok(())
}

fn command_on_path(bin: &str) -> bool {
    StdCommand::new("which")
        .arg(bin)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

async fn detect_git_version() -> Result<String> {
    let output = Command::new("git")
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("failed to execute `git --version`")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("{}", first_non_empty_line(&stderr));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

async fn is_git_repo(workdir: &Path) -> Result<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workdir)
        .args(["rev-parse", "--is-inside-work-tree"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("failed to execute git repo probe")?;

    if !output.status.success() {
        return Ok(false);
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim() == "true")
}

async fn probe_agent_auth(agent: &str) -> AgentAuthStatus {
    let mut cmd = build_auth_probe(agent);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let result = match timeout(Duration::from_secs(10), cmd.output()).await {
        Ok(res) => res,
        Err(_) => return AgentAuthStatus::TimedOut,
    };

    let output = match result {
        Ok(output) => output,
        Err(e) => return AgentAuthStatus::ProbeFailed(e.to_string()),
    };

    if output.status.success() {
        return AgentAuthStatus::Authenticated;
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = if stderr.trim().is_empty() {
        stdout.as_ref()
    } else {
        stderr.as_ref()
    };
    let message = first_non_empty_line(combined);

    if agent == "claude" && is_claude_api_key_error(combined) {
        return AgentAuthStatus::NotAuthenticated(
            "API key required for `claude --print` (set ANTHROPIC_API_KEY)".to_string(),
        );
    }

    AgentAuthStatus::NotAuthenticated(if message.is_empty() {
        "probe failed with non-zero exit".to_string()
    } else {
        message
    })
}

fn build_auth_probe(agent: &str) -> Command {
    match agent {
        "claude" => {
            let mut c = Command::new("claude");
            c.arg("--dangerously-skip-permissions")
                .arg("--print")
                .arg("-p")
                .arg("hi");
            c
        }
        "codex" => {
            let mut c = Command::new("codex");
            c.arg("exec").arg("--full-auto").arg("hi");
            c
        }
        "gemini" => {
            let mut c = Command::new("gemini");
            c.arg("-p").arg("hi").arg("--yolo");
            c
        }
        "opencode" => {
            let mut c = Command::new("opencode");
            c.arg("run").arg("hi");
            c
        }
        _ => Command::new(agent),
    }
}

fn is_claude_api_key_error(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("invalid api key") || lower.contains("api key")
}

async fn check_disk_space(workdir: &Path) -> Result<(u64, u64, u8)> {
    let output = Command::new("df")
        .arg("-k")
        .arg(workdir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("failed to execute `df -k`")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("{}", first_non_empty_line(&stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_df_k_output(&stdout).ok_or_else(|| anyhow::anyhow!("unexpected `df -k` output format"))
}

fn parse_df_k_output(output: &str) -> Option<(u64, u64, u8)> {
    let line = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .nth(1)?;

    let fields: Vec<&str> = line.split_whitespace().collect();
    if fields.len() < 6 {
        return None;
    }

    let total_kib = fields.get(fields.len() - 5)?.parse::<u64>().ok()?;
    let avail_kib = fields.get(fields.len() - 3)?.parse::<u64>().ok()?;
    let used_percent = fields
        .get(fields.len() - 2)?
        .trim_end_matches('%')
        .parse::<u8>()
        .ok()?;

    Some((total_kib, avail_kib, used_percent))
}

fn kib_to_human(kib: u64) -> String {
    let gib = kib as f64 / (1024.0 * 1024.0);
    if gib >= 1.0 {
        return format!("{gib:.1} GiB");
    }

    let mib = kib as f64 / 1024.0;
    if mib >= 1.0 {
        return format!("{mib:.1} MiB");
    }

    format!("{kib} KiB")
}

fn first_non_empty_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .to_string()
}

fn print_doctor_table(rows: &[DoctorRow]) {
    let mut check_w = "CHECK".len();
    let mut status_w = "STATUS".len();
    for row in rows {
        check_w = check_w.max(row.check.len());
        status_w = status_w.max(row.status.len());
    }

    println!(
        "{:<check_w$}  {:<status_w$}  DETAILS",
        "CHECK",
        "STATUS",
        check_w = check_w,
        status_w = status_w
    );
    println!(
        "{}  {}  {}",
        "-".repeat(check_w),
        "-".repeat(status_w),
        "-".repeat(48)
    );

    for row in rows {
        println!(
            "{:<check_w$}  {:<status_w$}  {}",
            row.check,
            row.status,
            row.details,
            check_w = check_w,
            status_w = status_w
        );
    }
}

async fn show_status(args: cli::StatusArgs) -> Result<()> {
    use std::path::PathBuf;

    let workdir: PathBuf = args
        .workdir
        .unwrap_or_else(|| PathBuf::from("."))
        .canonicalize()
        .context("Cannot resolve workdir — does it exist?")?;

    // In status command, we check for both .ralph/ and .ralph-*/ locks
    // to give a complete picture.

    let locks = find_active_locks(&workdir).await?;

    if locks.is_empty() {
        println!("💤  No ralph loops running in {}", workdir.display());
        return Ok(());
    }

    println!(
        "🟢  {} active loop(s) in {}\n",
        locks.len(),
        workdir.display()
    );

    for (path, lock) in locks {
        let elapsed = Utc::now()
            .signed_duration_since(lock.started_at)
            .to_std()
            .unwrap_or_default();

        let elapsed_str = format_duration(elapsed);
        let name = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy())
            .unwrap_or_default();
        let loop_name = if name == ".ralph" {
            "default".to_string()
        } else {
            name.trim_start_matches(".ralph-").to_string()
        };

        // Check if alive
        let alive = is_pid_alive(lock.pid);
        let status_icon = if alive { "🟢" } else { "💀" };

        println!("    {status_icon} [{}] PID {}", loop_name, lock.pid);
        println!("       PRD:      {}", lock.prd_path);
        println!("       Agent:    {}", lock.agent);
        println!("       Task:     {}", lock.current_task);
        println!("       Progress: {}", lock.progress);
        println!("       Time:     {}", elapsed_str);
        if !alive {
            println!("       (process appears dead — stale lock)");
        }
        println!();
    }

    Ok(())
}

/// Check if a process with the given PID is alive.
fn is_pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        use nix::sys::signal::{kill, Signal};
        use nix::unistd::Pid;
        // signal 0 checks for existence
        kill(Pid::from_raw(pid as i32), Option::<Signal>::None).is_ok()
    }

    #[cfg(not(unix))]
    {
        // Fallback for non-unix (though likely running on Linux per prompt)
        std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {}", pid)])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
            .unwrap_or(false)
    }
}

/// Format a duration as h:m:s
fn format_duration(d: std::time::Duration) -> String {
    let total_secs = d.as_secs();
    let h = total_secs / 3600;
    let m = (total_secs % 3600) / 60;
    let s = total_secs % 60;

    if h > 0 {
        format!("{}h {}m {}s", h, m, s)
    } else if m > 0 {
        format!("{}m {}s", m, s)
    } else {
        format!("{}s", s)
    }
}

/// Find all lock files in .ralph/ and .ralph-*/ directories.
async fn find_active_locks(workdir: &std::path::Path) -> Result<Vec<(PathBuf, state::LockFile)>> {
    let mut results = Vec::new();
    let mut read_dir = tokio::fs::read_dir(workdir)
        .await
        .context("Cannot read workdir")?;

    while let Some(entry) = read_dir.next_entry().await? {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();

        if name == ".ralph" || name.starts_with(".ralph-") {
            let lock_path = path.join("lock");
            if lock_path.exists() {
                // Try parse
                if let Ok(content) = tokio::fs::read_to_string(&lock_path).await {
                    if let Ok(lock) = serde_json::from_str::<state::LockFile>(&content) {
                        results.push((lock_path, lock));
                    }
                }
            }
        }
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli;
    use crate::config::{DefaultsConfig, HooksConfig, RalphConfig};
    use chrono::Utc;
    use clap::{CommandFactory, Parser};
    use tempfile::tempdir;

    fn sample_lock(pid: u32) -> state::LockFile {
        state::LockFile {
            pid,
            current_task: "T8 — Lock tests".to_string(),
            progress: "0/2 done".to_string(),
            started_at: Utc::now(),
            prd_path: "tests/PRD.md".to_string(),
            agent: "codex".to_string(),
        }
    }

    #[tokio::test]
    async fn find_active_locks_reads_default_and_named_state_directories() {
        let dir = tempdir().expect("create tempdir");
        let default_state = state::StateManager::new(dir.path()).expect("create default state");
        let watch_a_state =
            state::StateManager::new_named(dir.path(), "watch-a").expect("create watch-a state");
        let watch_b_state =
            state::StateManager::new_named(dir.path(), "watch-b").expect("create watch-b state");

        default_state
            .write_lock(&sample_lock(1001))
            .expect("write default lock");
        watch_a_state
            .write_lock(&sample_lock(1002))
            .expect("write watch-a lock");
        watch_b_state
            .write_lock(&sample_lock(1003))
            .expect("write watch-b lock");

        let locks = find_active_locks(dir.path())
            .await
            .expect("find active locks");
        let lock_paths: Vec<_> = locks.into_iter().map(|(path, _)| path).collect();

        assert_eq!(lock_paths.len(), 3);
        assert!(lock_paths.contains(&default_state.lock_file));
        assert!(lock_paths.contains(&watch_a_state.lock_file));
        assert!(lock_paths.contains(&watch_b_state.lock_file));
    }

    #[test]
    fn create_prd_template_writes_expected_sections() {
        let dir = tempdir().expect("create tempdir");

        create_prd_template(dir.path()).expect("create template");

        let content =
            std::fs::read_to_string(dir.path().join("prd.md")).expect("read generated template");
        assert!(content.contains("# Overview"));
        assert!(content.contains("# Requirements"));
        assert!(content.contains("# Acceptance Criteria"));
        assert!(content.contains("TODO:"));
    }

    #[test]
    fn create_prd_template_fails_when_prd_exists() {
        let dir = tempdir().expect("create tempdir");
        let prd_path = dir.path().join("prd.md");
        std::fs::write(&prd_path, "existing content").expect("seed existing prd");

        let err = create_prd_template(dir.path()).expect_err("should reject existing prd");
        let rendered = err.to_string();
        assert!(rendered.contains("already exists"));
        assert!(rendered.contains("prd.md"));
    }

    #[test]
    fn parse_df_k_output_parses_expected_columns() {
        let output = "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/disk3s1s1 488245288 110354576 361727632 24% /\n";
        let parsed = parse_df_k_output(output).expect("parse should succeed");

        assert_eq!(parsed.0, 488245288);
        assert_eq!(parsed.1, 361727632);
        assert_eq!(parsed.2, 24);
    }

    #[test]
    fn parse_df_k_output_rejects_invalid_shape() {
        let output = "Filesystem 1024-blocks Used Available Capacity Mounted on\ninvalid\n";
        assert!(parse_df_k_output(output).is_none());
    }

    #[test]
    fn run_uses_config_defaults_when_flags_not_set() {
        let argv = ["ralph", "run", "prd.md"];
        let cli = cli::Cli::parse_from(argv);
        let matches = cli::Cli::command()
            .try_get_matches_from(argv)
            .expect("matches should parse");
        let run_matches = matches
            .subcommand_matches("run")
            .expect("run subcommand matches");

        let mut args = match cli.command {
            cli::Commands::Run(args) => args,
            _ => panic!("expected run command"),
        };

        let config = RalphConfig {
            defaults: Some(DefaultsConfig {
                agent: Some("codex".to_string()),
                max_iterations: Some(33),
                timeout: Some(700),
                stall_timeout: Some(99),
                max_failures: Some(4),
            }),
            hooks: Some(HooksConfig {
                url: Some("https://hooks.example/ralph".to_string()),
                token: Some("token-abc".to_string()),
            }),
        };

        apply_run_config(&mut args, Some(&config), run_matches);

        assert_eq!(args.agent, "codex");
        assert_eq!(args.max_iterations, 33);
        assert_eq!(args.timeout, 700);
        assert_eq!(args.stall_timeout, 99);
        assert_eq!(args.max_failures, 4);
        assert_eq!(
            args.hook_url.as_deref(),
            Some("https://hooks.example/ralph")
        );
        assert_eq!(args.hook_token.as_deref(), Some("token-abc"));
    }

    #[test]
    fn run_cli_flags_override_config_values() {
        let argv = [
            "ralph",
            "run",
            "prd.md",
            "--agent",
            "gemini",
            "--max-iterations",
            "5",
            "--hook-url",
            "https://cli.example/hook",
        ];
        let cli = cli::Cli::parse_from(argv);
        let matches = cli::Cli::command()
            .try_get_matches_from(argv)
            .expect("matches should parse");
        let run_matches = matches
            .subcommand_matches("run")
            .expect("run subcommand matches");

        let mut args = match cli.command {
            cli::Commands::Run(args) => args,
            _ => panic!("expected run command"),
        };

        let config = RalphConfig {
            defaults: Some(DefaultsConfig {
                agent: Some("codex".to_string()),
                max_iterations: Some(33),
                timeout: Some(700),
                stall_timeout: Some(99),
                max_failures: Some(4),
            }),
            hooks: Some(HooksConfig {
                url: Some("https://config.example/hook".to_string()),
                token: Some("token-from-config".to_string()),
            }),
        };

        apply_run_config(&mut args, Some(&config), run_matches);

        assert_eq!(args.agent, "gemini");
        assert_eq!(args.max_iterations, 5);
        // Not specified on CLI, so config still applies.
        assert_eq!(args.timeout, 700);
        assert_eq!(args.hook_url.as_deref(), Some("https://cli.example/hook"));
        assert_eq!(args.hook_token.as_deref(), Some("token-from-config"));
    }
}

/// Shared test lock for tests that mutate process-global state (PATH, env vars).
/// Import from both `orchestrator::tests` and `parser::tests` to serialize them.
#[cfg(test)]
pub(crate) fn global_env_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}
