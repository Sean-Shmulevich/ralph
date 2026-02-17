//! OpenClaw notification integration.
//!
//! When `--notify <channel>:<target>` is set, Ralph sends progress messages
//! directly to the specified chat channel via OpenClaw's `/tools/invoke` API,
//! calling the `message` tool. No AI middleman — messages are delivered exactly
//! as formatted.

use crate::hooks::HookEvent;
use std::path::Path;

/// Parsed notify target (e.g. `discord:1234567890`).
#[derive(Debug, Clone)]
pub struct NotifyTarget {
    pub channel: String, // "discord", "telegram", etc.
    pub to: String,      // channel id or recipient
}

impl NotifyTarget {
    /// Parse `"discord:123456"` or `"discord"` (no target = use last).
    pub fn parse(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.splitn(2, ':').collect();
        match parts.len() {
            2 => Some(Self {
                channel: parts[0].to_string(),
                to: parts[1].to_string(),
            }),
            1 => Some(Self {
                channel: parts[0].to_string(),
                to: String::new(),
            }),
            _ => None,
        }
    }
}

/// Configuration for OpenClaw notify integration.
#[derive(Debug, Clone)]
pub struct NotifyConfig {
    /// OpenClaw gateway base URL (e.g. `http://127.0.0.1:18789`).
    pub gateway_url: String,
    /// Gateway auth token.
    pub gateway_token: String,
    /// Where to deliver.
    pub target: NotifyTarget,
    /// PRD name for context in messages.
    pub prd_name: String,
}

impl NotifyConfig {
    /// Build from env vars + CLI flag.
    /// Tries OPENCLAW_GATEWAY_TOKEN, then OPENCLAW_TOKEN, then OPENCLAW_HOOKS_TOKEN.
    pub fn from_env(notify_flag: &str, prd_name: &str) -> Option<Self> {
        let target = NotifyTarget::parse(notify_flag)?;

        let gateway_token = std::env::var("OPENCLAW_GATEWAY_TOKEN")
            .or_else(|_| std::env::var("OPENCLAW_TOKEN"))
            .or_else(|_| std::env::var("OPENCLAW_HOOKS_TOKEN"))
            .ok()?;

        let gateway_url = std::env::var("OPENCLAW_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:18789".to_string());

        Some(Self {
            gateway_url,
            gateway_token,
            target,
            prd_name: prd_name.to_string(),
        })
    }
}

/// Format a hook event into a human-readable message for chat.
fn format_event(config: &NotifyConfig, event: &HookEvent, log_tail: Option<&str>) -> String {
    let prd = &config.prd_name;
    match event {
        HookEvent::TaskComplete {
            task_id,
            task_title,
            iteration,
            duration_secs,
            progress,
            ..
        } => {
            format!(
                "✅ **{task_id}** — {task_title} (iter {iteration}, {duration_secs}s)\n📊 `[{prd}]` {}/{} tasks done",
                progress.completed, progress.total
            )
        }
        HookEvent::TaskFailed {
            task_id,
            task_title,
            iteration,
            error,
            consecutive_failures,
            progress,
            ..
        } => {
            let mut msg = format!(
                "❌ **{task_id}** — {task_title} failed (iter {iteration}, {consecutive_failures} consecutive)\n📊 `[{prd}]` {}/{} done\nError: {}",
                progress.completed, progress.total, truncate(error, 200)
            );
            if let Some(tail) = log_tail {
                msg.push_str(&format!("\n```\n{}\n```", truncate(tail, 500)));
            }
            msg
        }
        HookEvent::AllComplete {
            total_tasks,
            total_iterations,
            total_duration_secs,
            ..
        } => {
            let mins = total_duration_secs / 60;
            let secs = total_duration_secs % 60;
            format!(
                "🎉 **All {total_tasks} tasks complete!**\n📊 `[{prd}]` {total_iterations} iterations, {mins}m{secs}s total"
            )
        }
        HookEvent::CircuitBreaker {
            consecutive_failures,
            last_error,
            progress,
        } => {
            let mut msg = format!(
                "⚠️ **Circuit breaker triggered** — {consecutive_failures} consecutive failures\n📊 `[{prd}]` {}/{} done\nLast error: {}",
                progress.completed, progress.total, truncate(last_error, 200)
            );
            if let Some(tail) = log_tail {
                msg.push_str(&format!("\n```\n{}\n```", truncate(tail, 500)));
            }
            msg
        }
        HookEvent::MaxIterations {
            max_iterations,
            progress,
        } => {
            format!(
                "⚠️ **Hit max iterations ({max_iterations})**\n📊 `[{prd}]` {}/{} done — some tasks remain",
                progress.completed, progress.total
            )
        }
    }
}

/// Truncate a string to max_len chars, adding "…" if truncated.
fn truncate(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        s
    } else {
        // Find a valid UTF-8 char boundary at or before max_len
        let mut end = max_len;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }
}

/// Read the last N lines from a log file.
fn read_log_tail(log_path: &Path, lines: usize) -> Option<String> {
    let content = std::fs::read_to_string(log_path).ok()?;
    let all_lines: Vec<&str> = content.lines().collect();
    let start = all_lines.len().saturating_sub(lines);
    Some(all_lines[start..].join("\n"))
}

/// Send a notification to OpenClaw via /tools/invoke → message tool.
/// Direct delivery — no AI middleman, message arrives exactly as formatted.
pub async fn send_notify(
    config: &NotifyConfig,
    event: &HookEvent,
    log_path: Option<&Path>,
) {
    // For failure events, grab log tail
    let log_tail = match event {
        HookEvent::TaskFailed { .. } | HookEvent::CircuitBreaker { .. } => {
            log_path.and_then(|p| read_log_tail(p, 15))
        }
        _ => None,
    };

    let message = format_event(config, event, log_tail.as_deref());

    // Build the /tools/invoke payload for the message tool
    let mut msg_args = serde_json::json!({
        "action": "send",
        "channel": config.target.channel,
        "message": message,
    });

    if !config.target.to.is_empty() {
        msg_args["target"] = serde_json::Value::String(config.target.to.clone());
    }

    let payload = serde_json::json!({
        "tool": "message",
        "args": msg_args,
    });

    let url = format!("{}/tools/invoke", config.gateway_url);
    let body = serde_json::to_string(&payload).unwrap_or_default();

    let mut cmd = tokio::process::Command::new("curl");
    cmd.arg("-s")
        .arg("-X")
        .arg("POST")
        .arg("-H")
        .arg("Content-Type: application/json")
        .arg("-H")
        .arg(format!("Authorization: Bearer {}", config.gateway_token))
        .arg("-m")
        .arg("15")
        .arg("-d")
        .arg(&body)
        .arg(&url)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    match cmd.output().await {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.contains("\"ok\":true") {
                eprintln!("🔔  Notify: sent to {}:{}", config.target.channel, config.target.to);
            } else {
                eprintln!("⚠️  Notify: gateway responded but message may not have delivered: {}", stdout.chars().take(200).collect::<String>());
            }
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            eprintln!(
                "⚠️  Notify: failed ({}): {}",
                output.status,
                stderr.trim()
            );
        }
        Err(e) => {
            eprintln!("⚠️  Notify: send error: {e}");
        }
    }
}
