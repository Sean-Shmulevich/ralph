use anyhow::{Context, Result};
use chrono::Utc;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

use crate::cli::ParseArgs;
use crate::state::{Task, TaskList};

// ── Prompts ───────────────────────────────────────────────────────────────────

const PARSE_SYSTEM_PROMPT: &str = r#"You are a task extraction assistant. Your job is to read a Product Requirements Document (PRD) and produce a structured, ordered task list.

Output ONLY a valid JSON array — no markdown fences, no explanation, no commentary before or after.

Each task object must follow this exact schema:
{
  "id": "T1",
  "title": "Short task title (5–10 words)",
  "description": "One or two sentences describing what must be implemented.",
  "priority": 1,
  "status": "pending",
  "depends_on": []
}

Rules:
- id: Sequential T1, T2, T3, …
- priority: 1 = highest priority; assign in dependency order so prerequisites come first
- depends_on: list of task ids that must be complete before this one (empty array if none)
- status: always "pending"
- Do NOT include tasks that are already described as "phase 2" or "future work" unless they are clearly needed for the MVP
- Output ONLY the JSON array

PRD content follows:
"#;

// ── Public API ────────────────────────────────────────────────────────────────

/// Use an agent to parse a PRD markdown file into a `TaskList`.
pub async fn parse_prd(
    prd_path: &Path,
    agent: &str,
    model: Option<&str>,
    parse_timeout_secs: u64,
) -> Result<TaskList> {
    let prd_content = std::fs::read_to_string(prd_path)
        .with_context(|| format!("Cannot read PRD file: {}", prd_path.display()))?;

    let prompt = format!("{}{}", PARSE_SYSTEM_PROMPT, prd_content);

    eprintln!("🔍  Parsing PRD with {} (this may take a moment)…", agent);

    let raw = run_agent(agent, model, &prompt, parse_timeout_secs).await?;

    // Extract the JSON array — the agent might wrap it in prose.
    let json_str = extract_json_array(&raw).with_context(|| {
        format!(
            "Agent did not return a JSON array. Raw output:\n---\n{}\n---",
            raw
        )
    })?;

    let tasks: Vec<Task> = serde_json::from_str(&json_str).with_context(|| {
        format!(
            "JSON array from agent is not valid Task objects. JSON:\n{}\n",
            json_str
        )
    })?;

    Ok(TaskList {
        version: 1,
        prd_path: prd_path.to_string_lossy().to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        tasks,
    })
}

/// `ralph parse <prd.md>` entry point — parse and print (or write) tasks.
pub async fn parse_and_print(args: ParseArgs) -> Result<()> {
    let task_list = parse_prd(
        &args.prd,
        &args.agent,
        args.model.as_deref(),
        args.parse_timeout,
    )
    .await?;

    println!("\n📋  Tasks extracted from PRD:\n");
    for task in &task_list.tasks {
        let icon = "⏳";
        let deps = if task.depends_on.is_empty() {
            "none".to_string()
        } else {
            task.depends_on.join(", ")
        };
        println!("  {} {} — {}", icon, task.id, task.title);
        println!("    Priority: {}  │  Depends on: {}", task.priority, deps);
        println!("    {}", task.description);
        println!();
    }
    println!("Total: {} tasks", task_list.tasks.len());

    if let Some(ref output) = args.output {
        let content =
            serde_json::to_string_pretty(&task_list).context("Failed to serialise task list")?;
        std::fs::write(output, content)
            .with_context(|| format!("Failed to write {}", output.display()))?;
        println!("\n✅  Saved to {}", output.display());
    }

    Ok(())
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Agent ordering for fallback: try the requested agent first, then others.
const FALLBACK_ORDER: &[&str] = &["claude", "codex", "gemini", "opencode"];

async fn run_agent(
    agent: &str,
    model: Option<&str>,
    prompt: &str,
    parse_timeout_secs: u64,
) -> Result<String> {
    // Try the requested agent first
    match try_agent(agent, model, prompt, parse_timeout_secs).await {
        Ok(output) => return Ok(output),
        Err(e) => {
            eprintln!("⚠️  {} failed: {}", agent, e);
            eprintln!("    Trying fallback agents…");
        }
    }

    // Try fallback agents
    for fallback in FALLBACK_ORDER {
        if *fallback == agent {
            continue; // already tried
        }
        if !agent_on_path(fallback) {
            continue; // not installed
        }
        eprintln!("🔄  Trying {} as fallback…", fallback);
        match try_agent(fallback, model, prompt, parse_timeout_secs).await {
            Ok(output) => return Ok(output),
            Err(e) => {
                eprintln!("⚠️  {} also failed: {}", fallback, e);
            }
        }
    }

    anyhow::bail!(
        "All agents failed for PRD parsing. Tried: {} + fallbacks.\n\
         Make sure at least one agent is installed and authenticated.\n\
         Tip: run your agent standalone first (e.g. `claude --print -p \"hello\"`) to verify it works.",
        agent
    )
}

fn agent_on_path(name: &str) -> bool {
    let bin = match name {
        "claude" => "claude",
        "codex" => "codex",
        "gemini" => "gemini",
        "opencode" => "opencode",
        _ => return false,
    };
    std::process::Command::new("which")
        .arg(bin)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn build_agent_command(agent: &str, model: Option<&str>, prompt: &str) -> Result<Command> {
    let mut cmd = match agent {
        "claude" => {
            let mut c = Command::new("claude");
            c.arg("--dangerously-skip-permissions")
                .arg("--print")
                .arg("-p")
                .arg(prompt);
            if let Some(m) = model {
                c.arg("--model").arg(m);
            }
            c
        }
        "gemini" => {
            let mut c = Command::new("gemini");
            c.arg("-p").arg(prompt).arg("--yolo");
            if let Some(m) = model {
                c.arg("--model").arg(m);
            }
            c
        }
        "codex" => {
            let mut c = Command::new("codex");
            c.arg("exec").arg("--full-auto");
            if let Some(m) = model {
                c.arg("--model").arg(m);
            }
            c.arg(prompt);
            c
        }
        "opencode" => {
            let mut c = Command::new("opencode");
            c.arg("run");
            if let Some(m) = model {
                c.arg("--model").arg(m);
            }
            c.arg(prompt);
            c
        }
        other => anyhow::bail!("Unknown agent for parsing: {}", other),
    };
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    Ok(cmd)
}

async fn probe_claude_print_auth() -> Result<()> {
    let mut probe = Command::new("claude");
    probe
        .arg("--dangerously-skip-permissions")
        .arg("--print")
        .arg("-p")
        .arg("test")
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    let output = match timeout(Duration::from_secs(10), probe.output()).await {
        Ok(result) => result.context("Failed to spawn claude for auth probe")?,
        Err(_) => return Ok(()),
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if is_claude_api_key_error(&stderr) {
            eprintln!(
                "⚠️  Claude --print mode requires ANTHROPIC_API_KEY.\n    Your Claude install appears OAuth-only.\n    Set ANTHROPIC_API_KEY or use another agent (codex/gemini/opencode)."
            );
            anyhow::bail!("Claude --print auth probe failed: missing API key");
        }
    }

    Ok(())
}

fn is_claude_api_key_error(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("invalid api key") || lower.contains("api key")
}

async fn try_agent(
    agent: &str,
    model: Option<&str>,
    prompt: &str,
    parse_timeout_secs: u64,
) -> Result<String> {
    if agent == "claude" {
        probe_claude_print_auth().await?;
    }

    let mut cmd = build_agent_command(agent, model, prompt)?;

    let output = match timeout(Duration::from_secs(parse_timeout_secs), cmd.output()).await {
        Ok(result) => {
            result.with_context(|| format!("Failed to spawn {} — is it installed?", agent))?
        }
        Err(_) => anyhow::bail!("timed out after {}s", parse_timeout_secs),
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("{}", stderr.trim());
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Pull the first `[…]` JSON array out of arbitrary text.
fn extract_json_array(text: &str) -> Option<String> {
    let start = text.find('[')?;
    let end = text.rfind(']')?;
    if start <= end {
        Some(text[start..=end].to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{is_claude_api_key_error, parse_prd};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};
    use tempfile::tempdir;

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn write_fake_agent(bin_dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = bin_dir.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write fake agent");
        let mut perms = fs::metadata(&path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).expect("chmod fake agent");
        path
    }

    struct PathGuard(Option<String>);

    impl PathGuard {
        fn prepend(bin_dir: &Path) -> Self {
            let old_path = std::env::var("PATH").ok();
            let new_path = match old_path.as_deref() {
                Some(path) if !path.is_empty() => format!("{}:{}", bin_dir.display(), path),
                _ => bin_dir.display().to_string(),
            };
            std::env::set_var("PATH", new_path);
            Self(old_path)
        }
    }

    impl Drop for PathGuard {
        fn drop(&mut self) {
            if let Some(path) = self.0.take() {
                std::env::set_var("PATH", path);
            } else {
                std::env::remove_var("PATH");
            }
        }
    }

    #[tokio::test]
    #[ignore] // Modifies PATH; run with `cargo test -- --ignored` to include
    async fn parse_prd_times_out_and_uses_fallback_agent() {
        let _guard = crate::global_env_lock().lock().expect("lock env mutation");
        let dir = tempdir().expect("create tempdir");
        let bin_dir = dir.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("create bin dir");
        let prd_path = dir.path().join("prd.md");
        fs::write(&prd_path, "# Demo PRD").expect("write prd");

        write_fake_agent(&bin_dir, "claude", "while true; do :; done");
        write_fake_agent(
            &bin_dir,
            "codex",
            "echo '[{\"id\":\"T1\",\"title\":\"Task\",\"description\":\"Desc\",\"priority\":1,\"status\":\"pending\",\"depends_on\":[]}]'",
        );

        let _path_guard = PathGuard::prepend(&bin_dir);

        let task_list = parse_prd(&prd_path, "claude", None, 1)
            .await
            .expect("fallback should parse");

        assert_eq!(task_list.tasks.len(), 1);
        assert_eq!(task_list.tasks[0].id, "T1");
    }

    #[test]
    fn detects_claude_api_key_errors() {
        assert!(is_claude_api_key_error("Invalid API key provided"));
        assert!(is_claude_api_key_error("missing API key for request"));
        assert!(!is_claude_api_key_error("network unreachable"));
    }

    #[tokio::test]
    async fn parse_prd_claude_api_key_probe_falls_back_to_other_agent() {
        let _guard = crate::global_env_lock().lock().expect("lock env mutation");
        let dir = tempdir().expect("create tempdir");
        let bin_dir = dir.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("create bin dir");
        let prd_path = dir.path().join("prd.md");
        fs::write(&prd_path, "# Demo PRD").expect("write prd");

        write_fake_agent(
            &bin_dir,
            "claude",
            "echo 'Invalid API key for --print mode' 1>&2\nexit 1",
        );
        write_fake_agent(
            &bin_dir,
            "codex",
            "echo '[{\"id\":\"T1\",\"title\":\"Task\",\"description\":\"Desc\",\"priority\":1,\"status\":\"pending\",\"depends_on\":[]}]'",
        );

        let _path_guard = PathGuard::prepend(&bin_dir);

        let task_list = parse_prd(&prd_path, "claude", None, 5)
            .await
            .expect("fallback should parse");

        assert_eq!(task_list.tasks.len(), 1);
        assert_eq!(task_list.tasks[0].id, "T1");
    }
}
