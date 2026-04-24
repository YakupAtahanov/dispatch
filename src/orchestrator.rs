use std::collections::HashMap;

use serde_json::json;
use tokio::sync::mpsc;
use tokio::time::Duration;
use tracing::{debug, info, warn};

use crate::error::{DispatchError, Result};
use crate::mcp_client::DmcpClient;
use crate::pid;
use crate::reminder::{ReminderEvent, ReminderManager};
use crate::signal::{SignalEntry, SignalKind, SignalWindow};
use crate::task::{Task, TaskDef, TaskKind, TaskState, TaskStatus, TimerDef};

const DEFAULT_WINDOW_SIZE: usize = 20;
const REMINDER_CHANNEL_SIZE: usize = 64;
const TASK_CHANNEL_SIZE: usize = 64;

/// Event from a completed task.
struct TaskResult {
    pid: u64,
    kind: TaskResultKind,
}

enum TaskResultKind {
    McpComplete(std::result::Result<String, String>),
    TimerExpired {
        label: String,
        metadata: Option<serde_json::Value>,
        elapsed: u64,
    },
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

    /// Dispatch a list of MCP tasks. Returns the assigned PIDs.
    pub fn dispatch(&mut self, task_defs: Vec<TaskDef>) -> Vec<u64> {
        info!(count = task_defs.len(), "dispatching MCP tasks");
        let mut pids = Vec::with_capacity(task_defs.len());

        for def in task_defs {
            let task_pid = pid::next_pid();
            let remind_after = def.remind_after;
            let mut task = Task::new_mcp(task_pid, def);
            debug!(pid = task_pid, desc = %task.description(), "spawning MCP task");

            // Log INIT signal with the task's provenance nonce
            let init_entry = SignalEntry::new(task_pid, SignalKind::Init, task.description());
            let init_entry = match task.nonce.as_deref() {
                Some(h) => init_entry.with_nonce(h),
                None => init_entry,
            };
            self.signal_window.push(init_entry);

            // Start reminder timer if configured
            if let Some(secs) = remind_after {
                if secs > 0 {
                    self.reminder_mgr.start(task_pid, secs);
                }
            }

            // Spawn the async task
            let tx = self.task_result_tx.clone();
            let mcp_def = task.mcp_def();
            let server = mcp_def.server.clone();
            let tool = mcp_def.tool.clone();
            let params = mcp_def.params.clone();

            let join_handle = tokio::spawn(async move {
                let result = DmcpClient::call_tool(&server, &tool, &params).await;
                let output = match result {
                    Ok(stdout) => Ok(stdout),
                    Err(e) => Err(format!("Error: {}", e)),
                };
                let _ = tx
                    .send(TaskResult {
                        pid: task_pid,
                        kind: TaskResultKind::McpComplete(output),
                    })
                    .await;
            });

            task.abort_handle = Some(join_handle.abort_handle());
            self.tasks.insert(task_pid, task);
            pids.push(task_pid);
        }

        pids
    }

    /// Dispatch a timer. Returns the assigned PID.
    pub fn dispatch_timer(&mut self, def: TimerDef) -> u64 {
        let task_pid = pid::next_pid();
        info!(pid = task_pid, label = %def.label, duration = def.duration, "dispatching timer");
        let mut task = Task::new_timer(task_pid, def.clone());

        // Log INIT signal (timers carry no nonce — output is hardcoded, not external)
        self.signal_window.push(SignalEntry::with_payload(
            task_pid,
            SignalKind::Init,
            task.description(),
            json!({
                "pid": task_pid,
                "type": "INIT",
                "label": def.label,
                "metadata": def.metadata,
                "duration": def.duration,
            }),
        ));

        // Spawn the timer task
        let tx = self.task_result_tx.clone();
        let duration = def.duration;
        let label = def.label.clone();
        let metadata = def.metadata.clone();

        let join_handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(duration)).await;
            let _ = tx
                .send(TaskResult {
                    pid: task_pid,
                    kind: TaskResultKind::TimerExpired {
                        label,
                        metadata,
                        elapsed: duration,
                    },
                })
                .await;
        });

        task.abort_handle = Some(join_handle.abort_handle());
        self.tasks.insert(task_pid, task);
        task_pid
    }

    /// Kill tasks by PID. Returns which PIDs were actually killed.
    pub fn kill(&mut self, pids: &[u64]) -> Result<Vec<u64>> {
        debug!(pids = ?pids, "kill requested");
        let mut killed = Vec::new();

        for &task_pid in pids {
            let task = self
                .tasks
                .get_mut(&task_pid)
                .ok_or(DispatchError::TaskNotFound(task_pid))?;

            if !task.is_running() {
                debug!(pid = task_pid, "skip kill — task not running");
                continue;
            }

            // Abort the tokio task
            if let Some(handle) = task.abort_handle.take() {
                handle.abort();
            }

            let message = match &task.kind {
                TaskKind::Mcp(_) => "Terminated by LLM".to_string(),
                TaskKind::Timer(def) => format!("timer \"{}\" cancelled", def.label),
            };

            task.mark_killed();
            self.reminder_mgr.cancel(task_pid);

            info!(pid = task_pid, %message, "task killed");
            self.signal_window.push(SignalEntry::new(
                task_pid,
                SignalKind::Kill,
                message,
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
            debug!("wait_for_event: no running tasks, returning immediately");
            return Ok(self.signal_window.format_window(DEFAULT_WINDOW_SIZE));
        }

        debug!("wait_for_event: blocking until next event");
        loop {
            tokio::select! {
                // A task completed
                Some(result) = self.task_result_rx.recv() => {
                    self.handle_task_result(result);

                    // If all tasks are done, return immediately
                    if !self.has_running_tasks() {
                        debug!("all tasks completed, waking LLM");
                        return Ok(self.signal_window.format_window(DEFAULT_WINDOW_SIZE));
                    }
                }

                // A reminder fired
                Some(event) = self.reminder_rx.recv() => {
                    // Only fire reminder if task is still running
                    if let Some(task) = self.tasks.get(&event.pid) {
                        if task.is_running() {
                            info!(pid = event.pid, elapsed = event.elapsed_secs, "reminder fired, waking LLM");
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
                    warn!("event channels closed unexpectedly");
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

        // Snapshot nonce before mutably borrowing signal_window
        let task_nonce = self.tasks.get(&result.pid).and_then(|t| t.nonce.clone());

        match result.kind {
            TaskResultKind::McpComplete(ref output) => {
                match output {
                    Ok(_) => info!(pid = result.pid, "MCP task completed"),
                    Err(ref e) => warn!(pid = result.pid, error = %e, "MCP task failed"),
                }

                let raw = match output {
                    Ok(out) => {
                        if out.is_empty() {
                            "(no output)".to_string()
                        } else {
                            out.clone()
                        }
                    }
                    Err(err) => err.clone(),
                };

                // Wrap output in provenance delimiters so the LLM treats it as data
                let message = match task_nonce.as_deref() {
                    Some(h) => format!("<{h}>{raw}</{h}>"),
                    None => raw,
                };

                let exit_entry = match task_nonce {
                    Some(h) => {
                        SignalEntry::new(result.pid, SignalKind::Exit, message).with_nonce(h)
                    }
                    None => SignalEntry::new(result.pid, SignalKind::Exit, message),
                };

                self.signal_window.push(exit_entry);
            }
            TaskResultKind::TimerExpired {
                label,
                metadata,
                elapsed,
            } => {
                info!(pid = result.pid, %label, elapsed, "timer expired");
                // Fire REMIND signal with full payload
                self.signal_window.push(SignalEntry::with_payload(
                    result.pid,
                    SignalKind::Remind,
                    format!("timer \"{}\" — {}s elapsed", label, elapsed),
                    json!({
                        "pid": result.pid,
                        "type": "REMIND",
                        "label": label,
                        "metadata": metadata,
                        "elapsed": elapsed,
                    }),
                ));

                // Then fire EXIT signal
                self.signal_window.push(SignalEntry::new(
                    result.pid,
                    SignalKind::Exit,
                    "timer completed",
                ));
            }
        }

        if let Some(task) = self.tasks.get_mut(&result.pid) {
            task.mark_exited();
        }
    }

    /// Clean up: kill all running tasks.
    pub fn shutdown(&mut self) {
        info!("shutting down orchestrator");
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::SignalKind;
    use crate::task::TimerDef;

    /// Helper: create a TimerDef with optional metadata.
    fn timer_def(label: &str, duration: u64, metadata: Option<serde_json::Value>) -> TimerDef {
        TimerDef {
            label: label.to_string(),
            duration,
            metadata,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn basic_timer_fires_init_remind_exit() {
        let mut orch = Orchestrator::new();
        let pid = orch.dispatch_timer(timer_def("test_timer", 2, None));
        assert!(pid > 0);

        // INIT should already be in the signal window
        let signals = orch.signal_window.all();
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].kind, SignalKind::Init);
        assert!(signals[0].message.contains("test_timer"));

        // Task should be running
        assert!(orch.has_running_tasks());

        // Wait for the timer to fire
        let result = orch.wait_for_event().await;
        assert!(result.is_ok());

        // After timer fires we should have: INIT, REMIND, EXIT
        let signals = orch.signal_window.all();
        assert_eq!(signals.len(), 3);
        assert_eq!(signals[0].kind, SignalKind::Init);
        assert_eq!(signals[1].kind, SignalKind::Remind);
        assert_eq!(signals[2].kind, SignalKind::Exit);

        // REMIND payload should contain the correct fields
        let payload = signals[1].payload.as_ref().expect("REMIND should have payload");
        assert_eq!(payload["type"], "REMIND");
        assert_eq!(payload["label"], "test_timer");
        assert_eq!(payload["elapsed"], 2);
        assert_eq!(payload["pid"], pid);

        // EXIT message should indicate timer completed
        assert!(signals[2].message.contains("timer completed"));

        // Timer signals carry no nonce
        assert!(signals[0].nonce.is_none());
        assert!(signals[2].nonce.is_none());

        // No more running tasks
        assert!(!orch.has_running_tasks());
    }

    #[tokio::test(start_paused = true)]
    async fn kill_timer_prevents_remind() {
        let mut orch = Orchestrator::new();
        let pid = orch.dispatch_timer(timer_def("kill_me", 60, None));

        // Advance a bit (but not to 60s)
        tokio::time::advance(Duration::from_secs(1)).await;

        // Kill the timer
        let killed = orch.kill(&[pid]).expect("kill should succeed");
        assert_eq!(killed, vec![pid]);

        // Should have INIT + KILL
        let signals = orch.signal_window.all();
        assert_eq!(signals.len(), 2);
        assert_eq!(signals[0].kind, SignalKind::Init);
        assert_eq!(signals[1].kind, SignalKind::Kill);
        assert!(signals[1].message.contains("cancelled"));

        // Advance past the original duration — no REMIND should appear
        tokio::time::advance(Duration::from_secs(120)).await;
        orch.drain_results();

        let signals = orch.signal_window.all();
        assert_eq!(signals.len(), 2); // Still just INIT + KILL

        assert!(!orch.has_running_tasks());
    }

    #[tokio::test(start_paused = true)]
    async fn multiple_timers_fire_independently() {
        let mut orch = Orchestrator::new();

        let pid1 = orch.dispatch_timer(timer_def("fast", 1, None));
        let pid2 = orch.dispatch_timer(timer_def("medium", 3, None));
        let pid3 = orch.dispatch_timer(timer_def("slow", 5, None));

        // 3 INIT signals
        assert_eq!(orch.signal_window.all().len(), 3);

        // First wait — fast timer fires at 1s
        let _ = orch.wait_for_event().await;
        // fast fires: REMIND + EXIT, but medium and slow still running
        // Actually wait_for_event returns when all tasks are done OR a timer fires.
        // Since fast fires at 1s, we get its REMIND+EXIT but still have 2 running.
        // wait_for_event returns when !has_running_tasks OR a reminder fires.
        // But timer results come through task_result_rx, not reminder_rx.
        // So it returns only when a task finishes and no more running, or all finish.
        // With 3 timers, it'll return after ALL finish.

        // All 3 should be done after wait_for_event (since it loops until no running tasks)
        let signals = orch.signal_window.all();

        // 3 INIT + 3 REMIND + 3 EXIT = 9
        assert_eq!(signals.len(), 9);

        // Verify each timer got its own REMIND
        let reminds: Vec<_> = signals
            .iter()
            .filter(|s| s.kind == SignalKind::Remind)
            .collect();
        assert_eq!(reminds.len(), 3);

        // Verify PIDs are distinct
        let mut remind_pids: Vec<u64> = reminds.iter().map(|s| s.pid).collect();
        remind_pids.sort();
        remind_pids.dedup();
        assert_eq!(remind_pids.len(), 3);

        // All three PIDs should be present
        assert!(remind_pids.contains(&pid1));
        assert!(remind_pids.contains(&pid2));
        assert!(remind_pids.contains(&pid3));

        assert!(!orch.has_running_tasks());
    }

    #[tokio::test(start_paused = true)]
    async fn metadata_passthrough() {
        let meta = json!({
            "goal_id": "abc123",
            "type": "goal_defer",
            "priority": 5
        });

        let mut orch = Orchestrator::new();
        let pid = orch.dispatch_timer(timer_def("goal_reminder", 2, Some(meta.clone())));

        // INIT payload should carry metadata
        let init_payload = orch.signal_window.all()[0]
            .payload
            .as_ref()
            .expect("INIT should have payload");
        assert_eq!(init_payload["metadata"], meta);

        // Wait for timer to fire
        let _ = orch.wait_for_event().await;

        // REMIND payload should carry the same metadata, unchanged
        let signals = orch.signal_window.all();
        let remind = signals.iter().find(|s| s.kind == SignalKind::Remind).unwrap();
        let remind_payload = remind.payload.as_ref().expect("REMIND should have payload");
        assert_eq!(remind_payload["metadata"], meta);
        assert_eq!(remind_payload["label"], "goal_reminder");
        assert_eq!(remind_payload["pid"], pid);
    }

    #[tokio::test(start_paused = true)]
    async fn status_shows_timer_with_remaining_time() {
        let mut orch = Orchestrator::new();
        orch.dispatch_timer(timer_def("check_build", 60, None));

        // Advance 10 seconds
        tokio::time::advance(Duration::from_secs(10)).await;

        let statuses = orch.status();
        assert_eq!(statuses.len(), 1);

        let status = &statuses[0];
        assert_eq!(status.state, TaskState::Running);
        match &status.kind {
            crate::task::TaskStatusKind::Timer { label, fires_in } => {
                assert_eq!(label, "check_build");
                // fires_in should be ~50 (60 - 10)
                assert!(*fires_in <= 50, "fires_in should be <= 50, got {}", fires_in);
                assert!(*fires_in >= 49, "fires_in should be >= 49, got {}", fires_in);
            }
            other => panic!("Expected timer status, got {:?}", other),
        }
    }

    #[test]
    fn kill_nonexistent_pid_returns_error() {
        let mut orch = Orchestrator::new();
        let result = orch.kill(&[999]);
        assert!(result.is_err());
    }
}
