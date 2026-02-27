use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fmt;

/// The type of signal emitted by the system.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum SignalKind {
    Init,
    Exit,
    Remind,
    Wait,
    Kill,
}

impl fmt::Display for SignalKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SignalKind::Init => write!(f, "INIT"),
            SignalKind::Exit => write!(f, "EXIT"),
            SignalKind::Remind => write!(f, "REMIND"),
            SignalKind::Wait => write!(f, "WAIT"),
            SignalKind::Kill => write!(f, "KILL"),
        }
    }
}

/// A single signal entry in the log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalEntry {
    pub timestamp: DateTime<Local>,
    pub pid: u64,
    pub kind: SignalKind,
    pub message: String,
}

impl fmt::Display for SignalEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}] PID {} {:<8} {}",
            self.timestamp.format("%H:%M:%S"),
            self.pid,
            self.kind,
            self.message
        )
    }
}

impl SignalEntry {
    pub fn new(pid: u64, kind: SignalKind, message: impl Into<String>) -> Self {
        Self {
            timestamp: Local::now(),
            pid,
            kind,
            message: message.into(),
        }
    }
}

/// Rolling signal window that keeps the last N entries.
pub struct SignalWindow {
    entries: VecDeque<SignalEntry>,
    capacity: usize,
}

impl SignalWindow {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn push(&mut self, entry: SignalEntry) {
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    /// Get the last `count` entries (or all if count > len).
    pub fn last(&self, count: usize) -> Vec<&SignalEntry> {
        let skip = self.entries.len().saturating_sub(count);
        self.entries.iter().skip(skip).collect()
    }

    /// Get all entries in the window.
    pub fn all(&self) -> Vec<&SignalEntry> {
        self.entries.iter().collect()
    }

    /// Format the window as a display string for the LLM.
    pub fn format_window(&self, count: usize) -> String {
        let entries = self.last(count);
        if entries.is_empty() {
            return "Signal window: (empty)".to_string();
        }
        let mut out = format!("Signal window (last {}):\n", entries.len());
        for entry in entries {
            out.push_str(&format!("{}\n", entry));
        }
        out
    }

    /// Serialize the window entries as JSON.
    pub fn to_json(&self, count: usize) -> serde_json::Value {
        let entries = self.last(count);
        serde_json::to_value(entries).unwrap_or(serde_json::Value::Array(vec![]))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_capacity() {
        let mut window = SignalWindow::new(3);
        for i in 1..=5 {
            window.push(SignalEntry::new(i, SignalKind::Init, format!("task {}", i)));
        }
        assert_eq!(window.len(), 3);
        let all = window.all();
        assert_eq!(all[0].pid, 3);
        assert_eq!(all[1].pid, 4);
        assert_eq!(all[2].pid, 5);
    }

    #[test]
    fn last_n() {
        let mut window = SignalWindow::new(10);
        for i in 1..=5 {
            window.push(SignalEntry::new(i, SignalKind::Exit, format!("done {}", i)));
        }
        let last2 = window.last(2);
        assert_eq!(last2.len(), 2);
        assert_eq!(last2[0].pid, 4);
        assert_eq!(last2[1].pid, 5);
    }
}
