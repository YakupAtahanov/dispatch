use std::collections::HashMap;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{self, Duration};
use tracing::debug;

/// A reminder event sent when a task exceeds its remind_after duration.
#[derive(Debug)]
pub struct ReminderEvent {
    pub pid: u64,
    pub elapsed_secs: u64,
}

/// Manages per-task reminder timers.
pub struct ReminderManager {
    timers: HashMap<u64, JoinHandle<()>>,
    tx: mpsc::Sender<ReminderEvent>,
}

impl ReminderManager {
    pub fn new(tx: mpsc::Sender<ReminderEvent>) -> Self {
        Self {
            timers: HashMap::new(),
            tx,
        }
    }

    /// Start a recurring reminder timer for a task.
    /// Fires every `interval_secs` until cancelled.
    pub fn start(&mut self, pid: u64, interval_secs: u64) {
        debug!(pid, interval_secs, "starting reminder timer");
        let tx = self.tx.clone();
        let handle = tokio::spawn(async move {
            let interval = Duration::from_secs(interval_secs);
            let mut elapsed: u64 = 0;
            loop {
                time::sleep(interval).await;
                elapsed += interval_secs;
                if tx
                    .send(ReminderEvent {
                        pid,
                        elapsed_secs: elapsed,
                    })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        self.timers.insert(pid, handle);
    }

    /// Cancel the reminder timer for a task.
    pub fn cancel(&mut self, pid: u64) {
        if let Some(handle) = self.timers.remove(&pid) {
            debug!(pid, "cancelling reminder timer");
            handle.abort();
        }
    }

    /// Cancel all reminder timers.
    pub fn cancel_all(&mut self) {
        for (_, handle) in self.timers.drain() {
            handle.abort();
        }
    }
}

impl Drop for ReminderManager {
    fn drop(&mut self) {
        self.cancel_all();
    }
}
