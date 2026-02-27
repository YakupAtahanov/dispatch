use thiserror::Error;

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error("dmcp not found on PATH")]
    DmcpNotFound,

    #[error("dmcp invocation failed: {0}")]
    DmcpError(String),

    #[error("task not found: PID {0}")]
    TaskNotFound(u64),

    #[error("task already finished: PID {0}")]
    TaskAlreadyFinished(u64),

    #[error("invalid request: {0}")]
    InvalidRequest(String),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("signal channel closed")]
    ChannelClosed,
}

pub type Result<T> = std::result::Result<T, DispatchError>;
