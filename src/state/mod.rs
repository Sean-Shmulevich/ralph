use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

// ── Task model ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Complete,
    Failed,
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskStatus::Pending => write!(f, "pending"),
            TaskStatus::InProgress => write!(f, "in_progress"),
            TaskStatus::Complete => write!(f, "complete"),
            TaskStatus::Failed => write!(f, "failed"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub description: String,
    pub priority: u32,
    pub status: TaskStatus,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskList {
    pub version: u32,
    pub prd_path: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tasks: Vec<Task>,
}

// ── Lock file model ───────────────────────────────────────────────────────────

/// Written to `.ralph/lock` while a `ralph run` is active.
/// The `ralph status` command reads this to display progress.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockFile {
    /// PID of the `ralph` process holding this lock.
    pub pid: u32,
    /// Current task description (e.g. "T2 — Implement login handler").
    pub current_task: String,
    /// Human-readable progress string (e.g. "2/8 done").
    pub progress: String,
    /// Wall-clock start time of the overall run.
    pub started_at: DateTime<Utc>,
    /// Path to the PRD being executed.
    pub prd_path: String,
    /// Agent name in use.
    pub agent: String,
}

// ── Shared loop status (for TUI and watch command) ────────────────────────────

/// The high-level lifecycle state of a single `ralph watch` loop.
#[derive(Debug, Clone, PartialEq)]
pub enum LoopState {
    Starting,
    Parsing,
    Running,
    Complete,
    Failed(String),
    Stopped,
}

impl std::fmt::Display for LoopState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoopState::Starting => write!(f, "starting"),
            LoopState::Parsing => write!(f, "parsing"),
            LoopState::Running => write!(f, "running"),
            LoopState::Complete => write!(f, "complete"),
            LoopState::Failed(e) => write!(f, "failed: {}", e),
            LoopState::Stopped => write!(f, "stopped"),
        }
    }
}

/// Live status of one orchestrator loop, shared between the loop task and the TUI.
#[derive(Debug)]
pub struct LoopStatus {
    /// Slug name derived from the PRD filename.
    pub name: String,
    /// PRD file path string for display.
    pub prd_path: String,
    /// Agent name in use.
    pub agent: String,
    /// Current high-level lifecycle state.
    pub state: LoopState,
    /// Description of the current task being executed.
    pub current_task: String,
    /// Number of tasks completed so far.
    pub tasks_done: u32,
    /// Total number of tasks in the PRD.
    pub tasks_total: u32,
    /// Current iteration number.
    pub iteration: u32,
    /// When this loop started (for elapsed time display).
    pub started_at: std::time::Instant,
    /// Recent log lines for TUI display (capped at 500).
    pub recent_logs: VecDeque<String>,
}

impl LoopStatus {
    pub fn new(name: String, prd_path: String, agent: String) -> Self {
        Self {
            name,
            prd_path,
            agent,
            state: LoopState::Starting,
            current_task: "—".to_string(),
            tasks_done: 0,
            tasks_total: 0,
            iteration: 0,
            started_at: std::time::Instant::now(),
            recent_logs: VecDeque::with_capacity(500),
        }
    }

    /// Append a log line, evicting the oldest if we're at capacity.
    pub fn push_log(&mut self, line: String) {
        if self.recent_logs.len() >= 500 {
            self.recent_logs.pop_front();
        }
        self.recent_logs.push_back(line);
    }

    /// Human-readable elapsed time since `started_at`.
    pub fn elapsed_str(&self) -> String {
        let secs = self.started_at.elapsed().as_secs();
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        let s = secs % 60;
        if h > 0 {
            format!("{}h{}m", h, m)
        } else if m > 0 {
            format!("{}m{}s", m, s)
        } else {
            format!("{}s", s)
        }
    }
}

/// Thread-safe handle to a `LoopStatus` shared between the loop task and the TUI.
pub type SharedLoopStatus = Arc<Mutex<LoopStatus>>;

// ── State manager ─────────────────────────────────────────────────────────────

/// Manages all on-disk state inside `.ralph/` under the project root.
pub struct StateManager {
    pub ralph_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub tasks_file: PathBuf,
    pub progress_file: PathBuf,
    pub lock_file: PathBuf,
}

impl StateManager {
    /// Create a new `StateManager` rooted at `workdir/.ralph/`.
    /// Creates the directory tree on first use.
    pub fn new(workdir: &Path) -> Result<Self> {
        let ralph_dir = workdir.join(".ralph");
        let logs_dir = ralph_dir.join("logs");

        fs::create_dir_all(&logs_dir)
            .context("Failed to create .ralph/logs/ directory")?;

        Ok(Self {
            tasks_file: ralph_dir.join("tasks.json"),
            progress_file: ralph_dir.join("progress.md"),
            lock_file: ralph_dir.join("lock"),
            logs_dir,
            ralph_dir,
        })
    }

    /// Create a `StateManager` rooted at `workdir/.ralph-<name>/`.
    /// Used by `ralph watch` so each parallel loop has its own isolated state.
    pub fn new_named(workdir: &Path, name: &str) -> Result<Self> {
        let ralph_dir = workdir.join(format!(".ralph-{}", name));
        let logs_dir = ralph_dir.join("logs");

        fs::create_dir_all(&logs_dir)
            .with_context(|| format!("Failed to create .ralph-{}/logs/ directory", name))?;

        Ok(Self {
            tasks_file: ralph_dir.join("tasks.json"),
            progress_file: ralph_dir.join("progress.md"),
            lock_file: ralph_dir.join("lock"),
            logs_dir,
            ralph_dir,
        })
    }

    // ── tasks.json ────────────────────────────────────────────────────────────

    pub fn load_tasks(&self) -> Result<Option<TaskList>> {
        if !self.tasks_file.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(&self.tasks_file)
            .context("Failed to read .ralph/tasks.json")?;

        let list: TaskList =
            serde_json::from_str(&content).context("Failed to parse .ralph/tasks.json")?;

        Ok(Some(list))
    }

    /// Atomically write tasks.json (write to tmp → fsync → rename).
    pub fn save_tasks(&self, tasks: &TaskList) -> Result<()> {
        let content =
            serde_json::to_string_pretty(tasks).context("Failed to serialise task list")?;

        // Write to a temp file in the same directory so rename is atomic.
        let mut tmp = tempfile::NamedTempFile::new_in(&self.ralph_dir)
            .context("Failed to create temp file for tasks.json")?;

        tmp.write_all(content.as_bytes())
            .context("Failed to write temp tasks.json")?;

        tmp.persist(&self.tasks_file)
            .map_err(|e| anyhow::anyhow!("Failed to atomically replace tasks.json: {}", e))?;

        Ok(())
    }

    // ── Lock file ─────────────────────────────────────────────────────────────

    /// Write (or overwrite) the lock file with current run metadata.
    pub fn write_lock(&self, lock: &LockFile) -> Result<()> {
        let content =
            serde_json::to_string_pretty(lock).context("Failed to serialise lock file")?;
        fs::write(&self.lock_file, content).context("Failed to write .ralph/lock")?;
        Ok(())
    }

    /// Remove the lock file (called on clean exit).
    pub fn remove_lock(&self) {
        let _ = fs::remove_file(&self.lock_file);
    }

    /// Read the lock file, if it exists.
    pub fn read_lock(&self) -> Result<Option<LockFile>> {
        if !self.lock_file.exists() {
            return Ok(None);
        }
        let content =
            fs::read_to_string(&self.lock_file).context("Failed to read .ralph/lock")?;
        let lock: LockFile =
            serde_json::from_str(&content).context("Failed to parse .ralph/lock")?;
        Ok(Some(lock))
    }

    // ── Log paths ─────────────────────────────────────────────────────────────

    pub fn log_path(&self, iteration: u32, task_id: &str) -> PathBuf {
        self.logs_dir
            .join(format!("iteration-{iteration}-{task_id}.log"))
    }

    // ── progress.md ───────────────────────────────────────────────────────────

    /// Append a timestamped entry to progress.md.
    pub fn append_progress(&self, entry: &str) -> Result<()> {
        let timestamp = Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
        let line = format!("\n## {timestamp}\n\n{entry}\n");

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.progress_file)
            .context("Failed to open progress.md")?;

        file.write_all(line.as_bytes())
            .context("Failed to write to progress.md")?;

        Ok(())
    }
}
