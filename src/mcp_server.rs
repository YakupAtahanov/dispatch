use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

use crate::orchestrator::Orchestrator;
use crate::task::{TaskDef, TimerDef};

// --- JSON-RPC types ---

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

impl JsonRpcResponse {
    fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Value, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

// --- MCP protocol constants ---

const MCP_VERSION: &str = "2024-11-05";
const SERVER_NAME: &str = "dispatch";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

// --- Tool definitions ---

fn tool_definitions() -> Value {
    json!([
        {
            "name": "dispatch",
            "description": "Dispatch a list of tasks for concurrent execution via MCP servers. Each task specifies a server, tool, parameters, and optional reminder interval. Returns immediately with assigned PIDs, then blocks until all tasks complete or a reminder fires — returning the signal window.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "tasks": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "server": { "type": "string", "description": "MCP server ID (as known to dmcp)" },
                                "tool": { "type": "string", "description": "Tool name on the server" },
                                "params": { "type": "object", "description": "Parameters to pass to the tool" },
                                "remind_after": { "type": "integer", "description": "Seconds before firing a reminder (omit for no reminder)" }
                            },
                            "required": ["server", "tool"]
                        },
                        "description": "List of tasks to dispatch concurrently"
                    }
                },
                "required": ["tasks"]
            }
        },
        {
            "name": "kill",
            "description": "Terminate running tasks by PID.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "pids": {
                        "type": "array",
                        "items": { "type": "integer" },
                        "description": "PIDs of tasks to kill"
                    }
                },
                "required": ["pids"]
            }
        },
        {
            "name": "wait",
            "description": "Acknowledge a reminder and let the task continue running. A new reminder will fire after the same interval. After acknowledging, blocks until the next event (task completion or reminder).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "pids": {
                        "type": "array",
                        "items": { "type": "integer" },
                        "description": "PIDs of tasks to continue waiting on"
                    }
                },
                "required": ["pids"]
            }
        },
        {
            "name": "status",
            "description": "Get current state of all active tasks.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        },
        {
            "name": "log",
            "description": "Get the signal window (last N entries).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "count": {
                        "type": "integer",
                        "description": "Number of entries to return (default: 20)"
                    }
                }
            }
        },
        {
            "name": "timer",
            "description": "Set a one-shot timer. Fires a REMIND signal after the specified duration, then exits. No MCP server involved — pure delay. Use for goal deferral, scheduled re-checks, or any timed reminder. Killable via the kill tool.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "label": {
                        "type": "string",
                        "description": "Human-readable label for the timer (appears in signals and status)"
                    },
                    "duration": {
                        "type": "integer",
                        "description": "Seconds until the REMIND signal fires"
                    },
                    "metadata": {
                        "type": "object",
                        "description": "Arbitrary key-value data passed through to the REMIND signal (opaque to dispatch)"
                    }
                },
                "required": ["label", "duration"]
            }
        }
    ])
}

// --- Server ---

pub async fn serve() -> io::Result<()> {
    let orchestrator = Arc::new(Mutex::new(Orchestrator::new()));

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();

    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        let request: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = JsonRpcResponse::error(
                    Value::Null,
                    -32700,
                    format!("Parse error: {}", e),
                );
                write_response(&mut stdout, &resp).await?;
                continue;
            }
        };

        let _ = &request.jsonrpc; // acknowledge field

        let id = request.id.clone().unwrap_or(Value::Null);

        let response = match request.method.as_str() {
            "initialize" => handle_initialize(id),

            "notifications/initialized" => {
                // Client acknowledgment, no response needed
                continue;
            }

            "tools/list" => handle_tools_list(id),

            "tools/call" => {
                handle_tools_call(id, request.params, Arc::clone(&orchestrator)).await
            }

            "ping" => JsonRpcResponse::success(id, json!({})),

            _ => {
                // For notifications (no id), silently ignore
                if request.id.is_none() {
                    continue;
                }
                JsonRpcResponse::error(id, -32601, "Method not found")
            }
        };

        write_response(&mut stdout, &response).await?;
    }

    // Client disconnected — clean up
    orchestrator.lock().await.shutdown();
    Ok(())
}

fn handle_initialize(id: Value) -> JsonRpcResponse {
    JsonRpcResponse::success(
        id,
        json!({
            "protocolVersion": MCP_VERSION,
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": SERVER_NAME,
                "version": SERVER_VERSION
            }
        }),
    )
}

fn handle_tools_list(id: Value) -> JsonRpcResponse {
    JsonRpcResponse::success(
        id,
        json!({
            "tools": tool_definitions()
        }),
    )
}

async fn handle_tools_call(
    id: Value,
    params: Value,
    orchestrator: Arc<Mutex<Orchestrator>>,
) -> JsonRpcResponse {
    let tool_name = params
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or(json!({}));

    match tool_name {
        "dispatch" => handle_dispatch(id, arguments, orchestrator).await,
        "kill" => handle_kill(id, arguments, orchestrator).await,
        "wait" => handle_wait(id, arguments, orchestrator).await,
        "status" => handle_status(id, orchestrator).await,
        "log" => handle_log(id, arguments, orchestrator).await,
        "timer" => handle_timer(id, arguments, orchestrator).await,
        _ => JsonRpcResponse::error(id, -32602, format!("Unknown tool: {}", tool_name)),
    }
}

async fn handle_dispatch(
    id: Value,
    arguments: Value,
    orchestrator: Arc<Mutex<Orchestrator>>,
) -> JsonRpcResponse {
    // Parse task definitions
    let tasks: Vec<TaskDef> = match arguments.get("tasks") {
        Some(tasks_val) => match serde_json::from_value(tasks_val.clone()) {
            Ok(t) => t,
            Err(e) => {
                return JsonRpcResponse::error(
                    id,
                    -32602,
                    format!("Invalid tasks: {}", e),
                );
            }
        },
        None => {
            return JsonRpcResponse::error(id, -32602, "Missing 'tasks' parameter");
        }
    };

    if tasks.is_empty() {
        return JsonRpcResponse::error(id, -32602, "Empty task list");
    }

    // Dispatch tasks
    let pids = orchestrator.lock().await.dispatch(tasks);

    // Block until all tasks complete or a reminder fires
    let signal_window = orchestrator.lock().await.wait_for_event().await;

    match signal_window {
        Ok(window) => JsonRpcResponse::success(
            id,
            json!({
                "content": [{
                    "type": "text",
                    "text": window
                }],
                "pids": pids
            }),
        ),
        Err(e) => JsonRpcResponse::error(id, -32000, format!("Dispatch error: {}", e)),
    }
}

async fn handle_kill(
    id: Value,
    arguments: Value,
    orchestrator: Arc<Mutex<Orchestrator>>,
) -> JsonRpcResponse {
    let pids: Vec<u64> = match arguments.get("pids") {
        Some(v) => match serde_json::from_value(v.clone()) {
            Ok(p) => p,
            Err(e) => {
                return JsonRpcResponse::error(
                    id,
                    -32602,
                    format!("Invalid pids: {}", e),
                );
            }
        },
        None => {
            return JsonRpcResponse::error(id, -32602, "Missing 'pids' parameter");
        }
    };

    let mut orch = orchestrator.lock().await;
    match orch.kill(&pids) {
        Ok(killed) => {
            let window = orch.log_text(20);
            JsonRpcResponse::success(
                id,
                json!({
                    "content": [{
                        "type": "text",
                        "text": window
                    }],
                    "killed": killed
                }),
            )
        }
        Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
    }
}

async fn handle_wait(
    id: Value,
    arguments: Value,
    orchestrator: Arc<Mutex<Orchestrator>>,
) -> JsonRpcResponse {
    let pids: Vec<u64> = match arguments.get("pids") {
        Some(v) => match serde_json::from_value(v.clone()) {
            Ok(p) => p,
            Err(e) => {
                return JsonRpcResponse::error(
                    id,
                    -32602,
                    format!("Invalid pids: {}", e),
                );
            }
        },
        None => {
            return JsonRpcResponse::error(id, -32602, "Missing 'pids' parameter");
        }
    };

    // Record WAIT signals
    {
        let mut orch = orchestrator.lock().await;
        if let Err(e) = orch.wait(&pids) {
            return JsonRpcResponse::error(id, -32000, e.to_string());
        }
    }

    // Block until next event
    let signal_window = orchestrator.lock().await.wait_for_event().await;

    match signal_window {
        Ok(window) => JsonRpcResponse::success(
            id,
            json!({
                "content": [{
                    "type": "text",
                    "text": window
                }],
                "waited": pids
            }),
        ),
        Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
    }
}

async fn handle_status(id: Value, orchestrator: Arc<Mutex<Orchestrator>>) -> JsonRpcResponse {
    let orch = orchestrator.lock().await;
    let statuses = orch.status();
    JsonRpcResponse::success(
        id,
        json!({
            "content": [{
                "type": "text",
                "text": serde_json::to_string_pretty(&statuses).unwrap_or_default()
            }],
            "tasks": statuses
        }),
    )
}

async fn handle_log(
    id: Value,
    arguments: Value,
    orchestrator: Arc<Mutex<Orchestrator>>,
) -> JsonRpcResponse {
    let count = arguments
        .get("count")
        .and_then(|v| v.as_u64())
        .unwrap_or(20) as usize;

    let orch = orchestrator.lock().await;
    let window_text = orch.log_text(count);
    let window_json = orch.log_json(count);

    JsonRpcResponse::success(
        id,
        json!({
            "content": [{
                "type": "text",
                "text": window_text
            }],
            "entries": window_json
        }),
    )
}

async fn handle_timer(
    id: Value,
    arguments: Value,
    orchestrator: Arc<Mutex<Orchestrator>>,
) -> JsonRpcResponse {
    let label = match arguments.get("label").and_then(|v| v.as_str()) {
        Some(l) => l.to_string(),
        None => {
            return JsonRpcResponse::error(id, -32602, "Missing 'label' parameter");
        }
    };

    let duration = match arguments.get("duration").and_then(|v| v.as_u64()) {
        Some(d) => d,
        None => {
            return JsonRpcResponse::error(id, -32602, "Missing or invalid 'duration' parameter");
        }
    };

    let metadata = arguments.get("metadata").cloned();

    let def = TimerDef {
        label,
        duration,
        metadata,
    };

    let pid = orchestrator.lock().await.dispatch_timer(def);

    // Block until the timer fires (or gets killed)
    let signal_window = orchestrator.lock().await.wait_for_event().await;

    match signal_window {
        Ok(window) => JsonRpcResponse::success(
            id,
            json!({
                "content": [{
                    "type": "text",
                    "text": window
                }],
                "pid": pid
            }),
        ),
        Err(e) => JsonRpcResponse::error(id, -32000, format!("Timer error: {}", e)),
    }
}

async fn write_response(
    stdout: &mut io::Stdout,
    response: &JsonRpcResponse,
) -> io::Result<()> {
    let json = serde_json::to_string(response).unwrap();
    stdout.write_all(json.as_bytes()).await?;
    stdout.write_all(b"\n").await?;
    stdout.flush().await?;
    Ok(())
}
