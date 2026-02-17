//! Callback hooks — notify external systems (e.g. OpenClaw) when events occur.

use serde::Serialize;
use std::time::Duration;

/// Events that can be sent to the callback hook.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum HookEvent {
    /// A single task/iteration completed successfully.
    TaskComplete {
        task_id: String,
        task_title: String,
        iteration: u32,
        duration_secs: u64,
        files_changed: Vec<String>,
        summary: String,
        progress: Progress,
    },
    /// A single task/iteration failed.
    TaskFailed {
        task_id: String,
        task_title: String,
        iteration: u32,
        duration_secs: u64,
        error: String,
        consecutive_failures: u32,
        progress: Progress,
    },
    /// All tasks finished — the full PRD is implemented.
    AllComplete {
        total_tasks: u32,
        total_iterations: u32,
        total_duration_secs: u64,
        summary: String,
        progress: Progress,
    },
    /// Ralph stopped due to circuit breaker (too many consecutive failures).
    CircuitBreaker {
        consecutive_failures: u32,
        last_error: String,
        progress: Progress,
    },
    /// Ralph stopped because it hit max iterations without finishing.
    MaxIterations {
        max_iterations: u32,
        progress: Progress,
    },
}

/// Progress snapshot included in every event.
#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub completed: u32,
    pub failed: u32,
    pub remaining: u32,
    pub total: u32,
}

/// Configuration for the callback hook.
#[derive(Debug, Clone)]
pub struct HookConfig {
    /// URL to POST events to.
    pub url: String,
    /// Optional bearer token for auth.
    pub token: Option<String>,
    /// Timeout for HTTP requests.
    pub timeout: Duration,
}

impl HookConfig {
    pub fn new(url: String, token: Option<String>) -> Self {
        Self {
            url,
            token,
            timeout: Duration::from_secs(10),
        }
    }
}

/// Send a hook event. Fires and forgets — errors are logged but don't stop Ralph.
pub async fn send_hook(config: &HookConfig, event: &HookEvent) {
    let event_name = match event {
        HookEvent::TaskComplete { .. } => "task_complete",
        HookEvent::TaskFailed { .. } => "task_failed",
        HookEvent::AllComplete { .. } => "all_complete",
        HookEvent::CircuitBreaker { .. } => "circuit_breaker",
        HookEvent::MaxIterations { .. } => "max_iterations",
    };

    let body = match serde_json::to_string(event) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("⚠️  Hook: failed to serialize event: {e}");
            return;
        }
    };

    // Use curl to avoid adding an HTTP client dependency (reqwest is heavy)
    let mut cmd = tokio::process::Command::new("curl");
    cmd.arg("-s")
        .arg("-X")
        .arg("POST")
        .arg("-H")
        .arg("Content-Type: application/json")
        .arg("-m")
        .arg(config.timeout.as_secs().to_string())
        .arg("--max-time")
        .arg(config.timeout.as_secs().to_string());

    if let Some(ref token) = config.token {
        cmd.arg("-H").arg(format!("Authorization: Bearer {token}"));
    }

    cmd.arg("-d").arg(&body).arg(&config.url);

    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());

    match cmd.output().await {
        Ok(output) if output.status.success() => {
            eprintln!("🔔  Hook: {event_name} → {}", config.url);
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            eprintln!(
                "⚠️  Hook: {event_name} failed ({}): {}",
                output.status,
                stderr.trim()
            );
        }
        Err(e) => {
            eprintln!("⚠️  Hook: {event_name} send error: {e}");
        }
    }
}
