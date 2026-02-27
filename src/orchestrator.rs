use std::collections::HashMap;

use tokio::sync::mpsc;

use crate::error::{DispatchError, Result};
use crate::mcp_client::DmcpClient;
use crate::pid;
use crate::reminder::{ReminderEvent, ReminderManager};
use crate::signal::{SignalEntry, SignalKind, SignalWindow};
use crate::task::{Task, TaskDef, TaskState, TaskStatus};

const DEFAULT_WINDOW_SIZE: usize = 20;
const REMINDER_CHANNEL_SIZE: usize = 64;
const TASK_CHANNEL_SIZE: usize = 64;

/// Event from a completed task.
struct TaskResult {
    pid: u64,
    output: std::result::Result<String, String>,
}

/// The orchestrator manages tasks, signals, and reminders.
pub struct Orchestrator {
    tasks: HashMap<u64, Task>,
    signal_window: SignalWindow,
    reminder_mgr: ReminderManager,
    reminder_rx: mpsc::Receiver<ReminderEvent>,
    task_result_rx: mpsc::Receiver<TaskResult>,
    task_result_tx: mpsc::Sender<TaskResult>,
}

impl Orchestrator {
    pub fn new() -> Self {
        let (reminder_tx, reminder_rx) = mpsc::channel(REMINDER_CHANNEL_SIZE);
        let (task_result_tx, task_result_rx) = mpsc::channel(TASK_CHANNEL_SIZE);
        Self {
            tasks: HashMap::new(),
            signal_window: SignalWindow::new(DEFAULT_WINDOW_SIZE),
            reminder_mgr: ReminderManager::new(reminder_tx),
            reminder_rx,
            task_result_rx,
            task_result_tx,
        }
    }

    /// Dispatch a list of tasks. Returns the assigned PIDs.
    pub fn dispatch(&mut self, task_defs: Vec<TaskDef>) -> Vec<u64> {
        let mut pids = Vec::with_capacity(task_defs.len());

        for def in task_defs {
            let task_pid = pid::next_pid();
            let mut task = Task::new(task_pid, def);

            // Log INIT signal
            self.signal_window.push(SignalEntry::new(
                task_pid,
                SignalKind::Init,
                task.description(),
            ));

            // Start reminder timer if configured
            if let Some(secs) = task.def.remind_after {
                if secs > 0 {
                    self.reminder_mgr.start(task_pid, secs);
                }
            }

            // Spawn the async task
            let tx = self.task_result_tx.clone();
            let server = task.def.server.clone();
            let tool = task.def.tool.clone();
            let params = task.def.params.clone();

            let join_handle = tokio::spawn(async move {
                let result = DmcpClient::call_tool(&server, &tool, &params).await;
                let output = match result {
                    Ok(stdout) => Ok(stdout),
                    Err(e) => Err(format!("Error: {}", e)),
                };
                let _ = tx.send(TaskResult { pid: task_pid, output }).await;
            });

            task.abort_handle = Some(join_handle.abort_handle());
            self.tasks.insert(task_pid, task);
            pids.push(task_pid);
        }

        pids
    }

    /// Kill tasks by PID. Returns which PIDs were actually killed.
    pub fn kill(&mut self, pids: &[u64]) -> Result<Vec<u64>> {
        let mut killed = Vec::new();

        for &task_pid in pids {
            let task = self
                .tasks
                .get_mut(&task_pid)
                .ok_or(DispatchError::TaskNotFound(task_pid))?;

            if !task.is_running() {
                continue;
            }

            // Abort the tokio task
            if let Some(handle) = task.abort_handle.take() {
                handle.abort();
            }

            task.mark_killed();
            self.reminder_mgr.cancel(task_pid);

            self.signal_window.push(SignalEntry::new(
                task_pid,
                SignalKind::Kill,
                "Terminated by LLM",
            ));

            killed.push(task_pid);
        }

        Ok(killed)
    }

    /// Acknowledge reminders for tasks, letting them continue.
    pub fn wait(&mut self, pids: &[u64]) -> Result<Vec<u64>> {
        let mut waited = Vec::new();

        for &task_pid in pids {
            let task = self
                .tasks
                .get(&task_pid)
                .ok_or(DispatchError::TaskNotFound(task_pid))?;

            if !task.is_running() {
                continue;
            }

            self.signal_window.push(SignalEntry::new(
                task_pid,
                SignalKind::Wait,
                "LLM decided to continue waiting",
            ));

            waited.push(task_pid);
        }

        Ok(waited)
    }

    /// Get status of all tasks.
    pub fn status(&self) -> Vec<TaskStatus> {
        self.tasks.values().map(TaskStatus::from).collect()
    }

    /// Get the signal window formatted as text.
    pub fn log_text(&self, count: usize) -> String {
        self.signal_window.format_window(count)
    }

    /// Get the signal window as JSON.
    pub fn log_json(&self, count: usize) -> serde_json::Value {
        self.signal_window.to_json(count)
    }

    /// Check if there are any running tasks.
    pub fn has_running_tasks(&self) -> bool {
        self.tasks
            .values()
            .any(|t| t.state == TaskState::Running)
    }

    /// Wait for the next event (task completion or reminder).
    /// Returns the updated signal window as text.
    /// This is the blocking call that keeps the LLM "asleep" until something happens.
    pub async fn wait_for_event(&mut self) -> Result<String> {
        if !self.has_running_tasks() {
            return Ok(self.signal_window.format_window(DEFAULT_WINDOW_SIZE));
        }

        loop {
            tokio::select! {
                // A task completed
                Some(result) = self.task_result_rx.recv() => {
                    self.handle_task_result(result);

                    // If all tasks are done, return immediately
                    if !self.has_running_tasks() {
                        return Ok(self.signal_window.format_window(DEFAULT_WINDOW_SIZE));
                    }
                }

                // A reminder fired
                Some(event) = self.reminder_rx.recv() => {
                    // Only fire reminder if task is still running
                    if let Some(task) = self.tasks.get(&event.pid) {
                        if task.is_running() {
                            self.signal_window.push(SignalEntry::new(
                                event.pid,
                                SignalKind::Remind,
                                format!("Running for {}s", event.elapsed_secs),
                            ));
                            // Wake the LLM on reminder
                            return Ok(self.signal_window.format_window(DEFAULT_WINDOW_SIZE));
                        }
                    }
                }

                else => {
                    return Err(DispatchError::ChannelClosed);
                }
            }
        }
    }

    /// Drain any pending task results without blocking.
    pub fn drain_results(&mut self) {
        while let Ok(result) = self.task_result_rx.try_recv() {
            self.handle_task_result(result);
        }
    }

    fn handle_task_result(&mut self, result: TaskResult) {
        self.reminder_mgr.cancel(result.pid);

        let message = match &result.output {
            Ok(output) => {
                if output.is_empty() {
                    "(no output)".to_string()
                } else {
                    output.clone()
                }
            }
            Err(err) => err.clone(),
        };

        self.signal_window.push(SignalEntry::new(
            result.pid,
            SignalKind::Exit,
            message,
        ));

        if let Some(task) = self.tasks.get_mut(&result.pid) {
            task.mark_exited();
        }
    }

    /// Clean up: kill all running tasks.
    pub fn shutdown(&mut self) {
        let running_pids: Vec<u64> = self
            .tasks
            .values()
            .filter(|t| t.is_running())
            .map(|t| t.pid)
            .collect();

        for task_pid in running_pids {
            if let Some(task) = self.tasks.get_mut(&task_pid) {
                if let Some(handle) = task.abort_handle.take() {
                    handle.abort();
                }
                task.mark_killed();
            }
        }

        self.reminder_mgr.cancel_all();
    }
}

impl Drop for Orchestrator {
    fn drop(&mut self) {
        self.shutdown();
    }
}
