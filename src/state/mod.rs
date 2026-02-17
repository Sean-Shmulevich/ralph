use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
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

        fs::create_dir_all(&logs_dir).context("Failed to create .ralph/logs/ directory")?;

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

        let content =
            fs::read_to_string(&self.tasks_file).context("Failed to read .ralph/tasks.json")?;

        let list: TaskList =
            serde_json::from_str(&content).context("Failed to parse .ralph/tasks.json")?;
        validate_task_list(&list).context("Invalid .ralph/tasks.json")?;

        Ok(Some(list))
    }

    /// Read tasks.json if it exists.
    #[cfg(test)]
    pub fn read_tasks(&self) -> Result<Option<TaskList>> {
        self.load_tasks()
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

    /// Atomically write tasks.json.
    #[cfg(test)]
    pub fn write_tasks(&self, tasks: &TaskList) -> Result<()> {
        self.save_tasks(tasks)
    }

    /// Return the highest-priority pending task whose dependencies are complete.
    #[cfg(test)]
    pub fn pick_next_task<'a>(&self, task_list: &'a TaskList) -> Option<&'a Task> {
        let complete_ids: HashSet<&str> = task_list
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Complete)
            .map(|t| t.id.as_str())
            .collect();

        task_list
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Pending)
            .filter(|t| {
                t.depends_on
                    .iter()
                    .all(|dep| complete_ids.contains(dep.as_str()))
            })
            .min_by_key(|t| t.priority)
    }

    /// Mark one task complete and persist tasks.json.
    #[cfg(test)]
    pub fn mark_complete(&self, task_id: &str) -> Result<()> {
        let mut list = self
            .load_tasks()?
            .ok_or_else(|| anyhow::anyhow!("tasks.json does not exist"))?;

        let task = list
            .tasks
            .iter_mut()
            .find(|t| t.id == task_id)
            .ok_or_else(|| anyhow::anyhow!("task not found: {}", task_id))?;

        task.status = TaskStatus::Complete;
        task.completed_at = Some(Utc::now());
        list.updated_at = Utc::now();

        self.save_tasks(&list)
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
        let content = fs::read_to_string(&self.lock_file).context("Failed to read .ralph/lock")?;
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

fn validate_task_list(task_list: &TaskList) -> Result<()> {
    let mut seen_ids = HashSet::new();
    for task in &task_list.tasks {
        if !seen_ids.insert(task.id.as_str()) {
            anyhow::bail!("Duplicate task id detected: {}", task.id);
        }
    }

    let mut indegree: HashMap<&str, usize> = HashMap::new();
    let mut outgoing: HashMap<&str, Vec<&str>> = HashMap::new();
    for task in &task_list.tasks {
        indegree.insert(task.id.as_str(), task.depends_on.len());
    }

    for task in &task_list.tasks {
        for dep in &task.depends_on {
            if !indegree.contains_key(dep.as_str()) {
                anyhow::bail!("Task '{}' depends on unknown task '{}'", task.id, dep);
            }
            outgoing
                .entry(dep.as_str())
                .or_default()
                .push(task.id.as_str());
        }
    }

    let mut queue: VecDeque<&str> = indegree
        .iter()
        .filter_map(|(id, degree)| (*degree == 0).then_some(*id))
        .collect();
    let mut visited = 0usize;

    while let Some(id) = queue.pop_front() {
        visited += 1;
        if let Some(dependents) = outgoing.get(id) {
            for dependent in dependents {
                if let Some(entry) = indegree.get_mut(dependent) {
                    *entry -= 1;
                    if *entry == 0 {
                        queue.push_back(dependent);
                    }
                }
            }
        }
    }

    if visited != task_list.tasks.len() {
        anyhow::bail!("Circular task dependencies detected in tasks.json");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn sample_task_list() -> TaskList {
        let now = Utc::now();
        TaskList {
            version: 1,
            prd_path: "tests/PRD.md".to_string(),
            created_at: now,
            updated_at: now,
            tasks: vec![
                Task {
                    id: "T1".to_string(),
                    title: "First".to_string(),
                    description: "first task".to_string(),
                    priority: 2,
                    status: TaskStatus::Pending,
                    depends_on: vec![],
                    completed_at: None,
                    notes: Some("note-1".to_string()),
                },
                Task {
                    id: "T2".to_string(),
                    title: "Second".to_string(),
                    description: "second task".to_string(),
                    priority: 1,
                    status: TaskStatus::Pending,
                    depends_on: vec![],
                    completed_at: None,
                    notes: None,
                },
            ],
        }
    }

    #[test]
    fn state_manager_new_creates_ralph_directory_tree() {
        let dir = tempdir().expect("create tempdir");
        let state = StateManager::new(dir.path()).expect("create state manager");

        assert!(state.ralph_dir.exists());
        assert!(state.ralph_dir.is_dir());
        assert!(state.logs_dir.exists());
        assert!(state.logs_dir.is_dir());
    }

    #[test]
    fn write_and_read_tasks_roundtrip_preserves_fields() {
        let dir = tempdir().expect("create tempdir");
        let state = StateManager::new(dir.path()).expect("create state manager");
        let original = sample_task_list();

        state.write_tasks(&original).expect("write tasks");
        let loaded = state
            .read_tasks()
            .expect("read tasks")
            .expect("tasks should exist");

        assert_eq!(loaded.version, original.version);
        assert_eq!(loaded.prd_path, original.prd_path);
        assert_eq!(loaded.created_at, original.created_at);
        assert_eq!(loaded.updated_at, original.updated_at);
        assert_eq!(loaded.tasks.len(), original.tasks.len());

        for (got, expected) in loaded.tasks.iter().zip(original.tasks.iter()) {
            assert_eq!(got.id, expected.id);
            assert_eq!(got.title, expected.title);
            assert_eq!(got.description, expected.description);
            assert_eq!(got.priority, expected.priority);
            assert_eq!(got.status, expected.status);
            assert_eq!(got.depends_on, expected.depends_on);
            assert_eq!(got.completed_at, expected.completed_at);
            assert_eq!(got.notes, expected.notes);
        }
    }

    #[test]
    fn write_tasks_failure_does_not_partially_overwrite_existing_file() {
        let dir = tempdir().expect("create tempdir");
        let state = StateManager::new(dir.path()).expect("create state manager");

        let mut original = sample_task_list();
        original.tasks[0].title = "original".to_string();
        state.write_tasks(&original).expect("initial write");

        let before = fs::read_to_string(&state.tasks_file).expect("read baseline tasks file");

        let mut replacement = sample_task_list();
        replacement.tasks[0].title = "replacement".to_string();

        let original_permissions = fs::metadata(&state.ralph_dir)
            .expect("metadata")
            .permissions();
        let mut readonly_permissions = original_permissions.clone();
        readonly_permissions.set_readonly(true);
        fs::set_permissions(&state.ralph_dir, readonly_permissions).expect("set readonly");

        let write_result = state.write_tasks(&replacement);

        fs::set_permissions(&state.ralph_dir, original_permissions).expect("restore permissions");

        assert!(write_result.is_err());

        let after = fs::read_to_string(&state.tasks_file).expect("read tasks file after failure");
        assert_eq!(after, before);

        let loaded = state
            .read_tasks()
            .expect("read tasks")
            .expect("tasks should exist");
        assert_eq!(loaded.tasks[0].title, "original");
    }

    #[test]
    fn pick_next_task_returns_highest_priority_pending_task() {
        let dir = tempdir().expect("create tempdir");
        let state = StateManager::new(dir.path()).expect("create state manager");
        let list = sample_task_list();

        let picked = state.pick_next_task(&list).expect("task should be picked");
        assert_eq!(picked.id, "T2");
    }

    #[test]
    fn pick_next_task_respects_dependencies() {
        let dir = tempdir().expect("create tempdir");
        let state = StateManager::new(dir.path()).expect("create state manager");
        let now = Utc::now();
        let list = TaskList {
            version: 1,
            prd_path: "tests/PRD.md".to_string(),
            created_at: now,
            updated_at: now,
            tasks: vec![
                Task {
                    id: "A".to_string(),
                    title: "blocked".to_string(),
                    description: "blocked task".to_string(),
                    priority: 1,
                    status: TaskStatus::Pending,
                    depends_on: vec!["B".to_string()],
                    completed_at: None,
                    notes: None,
                },
                Task {
                    id: "B".to_string(),
                    title: "dependency".to_string(),
                    description: "dependency task".to_string(),
                    priority: 2,
                    status: TaskStatus::Pending,
                    depends_on: vec![],
                    completed_at: None,
                    notes: None,
                },
            ],
        };

        let picked = state.pick_next_task(&list).expect("task should be picked");
        assert_eq!(picked.id, "B");
    }

    #[test]
    fn mark_complete_sets_status_and_persists() {
        let dir = tempdir().expect("create tempdir");
        let state = StateManager::new(dir.path()).expect("create state manager");
        let list = sample_task_list();
        state.write_tasks(&list).expect("write initial tasks");

        state.mark_complete("T1").expect("mark task complete");

        let loaded = state
            .read_tasks()
            .expect("read tasks")
            .expect("tasks should exist");
        let task = loaded
            .tasks
            .iter()
            .find(|t| t.id == "T1")
            .expect("task should exist");
        assert_eq!(task.status, TaskStatus::Complete);
        assert!(task.completed_at.is_some());
    }

    #[test]
    fn append_progress_appends_without_overwriting() {
        let dir = tempdir().expect("create tempdir");
        let state = StateManager::new(dir.path()).expect("create state manager");

        state.append_progress("entry one").expect("append first");
        state.append_progress("entry two").expect("append second");

        let content = fs::read_to_string(&state.progress_file).expect("read progress");
        assert!(content.contains("entry one"));
        assert!(content.contains("entry two"));
        assert!(
            content.find("entry one").expect("first entry exists")
                < content.find("entry two").expect("second entry exists")
        );
    }

    #[test]
    fn valid_tasks_json_deserializes_correctly() {
        let dir = tempdir().expect("create tempdir");
        let state = StateManager::new(dir.path()).expect("create state manager");
        let json = r#"{
  "version": 1,
  "prd_path": "tests/PRD.md",
  "created_at": "2026-02-17T11:25:50Z",
  "updated_at": "2026-02-17T11:25:50Z",
  "tasks": [
    {
      "id": "T1",
      "title": "Parse tasks",
      "description": "Parse PRD output into tasks",
      "priority": 1,
      "status": "pending",
      "depends_on": []
    }
  ]
}"#;

        fs::write(&state.tasks_file, json).expect("write tasks file");
        let loaded = state
            .read_tasks()
            .expect("read tasks")
            .expect("tasks should exist");

        assert_eq!(loaded.tasks.len(), 1);
        assert_eq!(loaded.tasks[0].id, "T1");
    }

    #[test]
    fn missing_required_fields_produce_clear_error() {
        let dir = tempdir().expect("create tempdir");
        let state = StateManager::new(dir.path()).expect("create state manager");
        let json = r#"{
  "version": 1,
  "prd_path": "tests/PRD.md",
  "created_at": "2026-02-17T11:25:50Z",
  "updated_at": "2026-02-17T11:25:50Z",
  "tasks": [
    {
      "id": "T1",
      "description": "Missing title",
      "priority": 1,
      "status": "pending",
      "depends_on": []
    }
  ]
}"#;

        fs::write(&state.tasks_file, json).expect("write tasks file");
        let err = state.read_tasks().expect_err("missing title should fail");
        let msg = format!("{:#}", err);

        assert!(msg.contains("Failed to parse .ralph/tasks.json"));
        assert!(msg.to_ascii_lowercase().contains("missing field"));
        assert!(msg.contains("title"));
    }

    #[test]
    fn empty_task_list_is_handled_gracefully() {
        let dir = tempdir().expect("create tempdir");
        let state = StateManager::new(dir.path()).expect("create state manager");
        let json = r#"{
  "version": 1,
  "prd_path": "tests/PRD.md",
  "created_at": "2026-02-17T11:25:50Z",
  "updated_at": "2026-02-17T11:25:50Z",
  "tasks": []
}"#;

        fs::write(&state.tasks_file, json).expect("write tasks file");
        let loaded = state
            .read_tasks()
            .expect("read tasks")
            .expect("tasks should exist");

        assert!(loaded.tasks.is_empty());
        assert!(state.pick_next_task(&loaded).is_none());
    }

    #[test]
    fn duplicate_task_ids_are_detected() {
        let dir = tempdir().expect("create tempdir");
        let state = StateManager::new(dir.path()).expect("create state manager");
        let json = r#"{
  "version": 1,
  "prd_path": "tests/PRD.md",
  "created_at": "2026-02-17T11:25:50Z",
  "updated_at": "2026-02-17T11:25:50Z",
  "tasks": [
    {
      "id": "T1",
      "title": "First",
      "description": "first",
      "priority": 1,
      "status": "pending",
      "depends_on": []
    },
    {
      "id": "T1",
      "title": "Second",
      "description": "second",
      "priority": 2,
      "status": "pending",
      "depends_on": []
    }
  ]
}"#;

        fs::write(&state.tasks_file, json).expect("write tasks file");
        let err = state.read_tasks().expect_err("duplicate ids should fail");
        let msg = format!("{:#}", err);
        assert!(msg.to_ascii_lowercase().contains("duplicate"));
        assert!(msg.contains("T1"));
    }

    #[test]
    fn circular_dependencies_are_detected() {
        let dir = tempdir().expect("create tempdir");
        let state = StateManager::new(dir.path()).expect("create state manager");
        let json = r#"{
  "version": 1,
  "prd_path": "tests/PRD.md",
  "created_at": "2026-02-17T11:25:50Z",
  "updated_at": "2026-02-17T11:25:50Z",
  "tasks": [
    {
      "id": "T1",
      "title": "First",
      "description": "first",
      "priority": 1,
      "status": "pending",
      "depends_on": ["T2"]
    },
    {
      "id": "T2",
      "title": "Second",
      "description": "second",
      "priority": 2,
      "status": "pending",
      "depends_on": ["T1"]
    }
  ]
}"#;

        fs::write(&state.tasks_file, json).expect("write tasks file");
        let err = state
            .read_tasks()
            .expect_err("circular dependencies should fail");
        let msg = format!("{:#}", err);
        assert!(msg.to_ascii_lowercase().contains("circular"));
    }
}
