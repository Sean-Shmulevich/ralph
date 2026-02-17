use anyhow::{Context, Result};
use chrono::Utc;
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

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
) -> Result<TaskList> {
    let prd_content = std::fs::read_to_string(prd_path)
        .with_context(|| format!("Cannot read PRD file: {}", prd_path.display()))?;

    let prompt = format!("{}{}", PARSE_SYSTEM_PROMPT, prd_content);

    eprintln!(
        "🔍  Parsing PRD with {} (this may take a moment)…",
        agent
    );

    let raw = run_agent(agent, model, &prompt).await?;

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
    let task_list = parse_prd(&args.prd, &args.agent, args.model.as_deref()).await?;

    println!("\n📋  Tasks extracted from PRD:\n");
    for task in &task_list.tasks {
        let icon = "⏳";
        let deps = if task.depends_on.is_empty() {
            "none".to_string()
        } else {
            task.depends_on.join(", ")
        };
        println!("  {} {} — {}", icon, task.id, task.title);
        println!(
            "    Priority: {}  │  Depends on: {}",
            task.priority, deps
        );
        println!("    {}", task.description);
        println!();
    }
    println!("Total: {} tasks", task_list.tasks.len());

    if let Some(ref output) = args.output {
        let content = serde_json::to_string_pretty(&task_list)
            .context("Failed to serialise task list")?;
        std::fs::write(output, content)
            .with_context(|| format!("Failed to write {}", output.display()))?;
        println!("\n✅  Saved to {}", output.display());
    }

    Ok(())
}

// ── Private helpers ───────────────────────────────────────────────────────────

async fn run_agent(agent: &str, model: Option<&str>, prompt: &str) -> Result<String> {
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
        other => anyhow::bail!("Unknown agent for parsing: {}", other),
    };

    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let output = cmd
        .output()
        .await
        .context("Failed to spawn agent for PRD parsing")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Agent failed during PRD parsing:\n{}", stderr.trim());
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
