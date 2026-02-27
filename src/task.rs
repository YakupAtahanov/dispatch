use serde::{Deserialize, Serialize};

/// Task state machine: Init → Running → Exit | Killed
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskState {
    Running,
    Exited,
    Killed,
}

/// A task definition as received from the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDef {
    pub server: String,
    pub tool: String,
    #[serde(default)]
    pub params: serde_json::Value,
    pub remind_after: Option<u64>,
}

/// A live task being tracked by the orchestrator.
#[derive(Debug)]
pub struct Task {
    pub pid: u64,
    pub def: TaskDef,
    pub state: TaskState,
    /// Handle to cancel the running task.
    pub abort_handle: Option<tokio::task::AbortHandle>,
}

impl Task {
    pub fn new(pid: u64, def: TaskDef) -> Self {
        Self {
            pid,
            def,
            state: TaskState::Running,
            abort_handle: None,
        }
    }

    pub fn is_running(&self) -> bool {
        self.state == TaskState::Running
    }

    pub fn mark_exited(&mut self) {
        self.state = TaskState::Exited;
    }

    pub fn mark_killed(&mut self) {
        self.state = TaskState::Killed;
    }

    /// Short description for signal messages.
    pub fn description(&self) -> String {
        let params_str = if self.def.params.is_null() || self.def.params == serde_json::json!({}) {
            String::new()
        } else {
            format!(" {}", self.def.params)
        };
        format!("{} {}{}", self.def.server, self.def.tool, params_str)
    }
}

/// Status snapshot of a task, suitable for serialization.
#[derive(Debug, Serialize)]
pub struct TaskStatus {
    pub pid: u64,
    pub server: String,
    pub tool: String,
    pub state: TaskState,
}

impl From<&Task> for TaskStatus {
    fn from(task: &Task) -> Self {
        Self {
            pid: task.pid,
            server: task.def.server.clone(),
            tool: task.def.tool.clone(),
            state: task.state.clone(),
        }
    }
}
