use crate::error::{DispatchError, Result};
use tokio::process::Command;

/// Client for invoking dmcp commands.
/// dispatch delegates all MCP server management to dmcp.
pub struct DmcpClient;

impl DmcpClient {
    /// Check that dmcp is available on PATH.
    pub async fn check_available() -> Result<()> {
        let output = Command::new("dmcp")
            .arg("paths")
            .output()
            .await
            .map_err(|_| DispatchError::DmcpNotFound)?;

        if !output.status.success() {
            return Err(DispatchError::DmcpNotFound);
        }
        Ok(())
    }

    /// Call a tool on an MCP server via `dmcp call <server> <tool> --args <json>`.
    /// Returns the stdout output as a string.
    pub async fn call_tool(
        server: &str,
        tool: &str,
        params: &serde_json::Value,
    ) -> Result<String> {
        let mut cmd = Command::new("dmcp");
        cmd.arg("call").arg(server).arg(tool);

        if !params.is_null() && params != &serde_json::json!({}) {
            cmd.arg("--args").arg(params.to_string());
        }

        let output = cmd.output().await.map_err(|e| {
            DispatchError::DmcpError(format!("failed to spawn dmcp: {}", e))
        })?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            Ok(stdout.trim().to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let msg = if stderr.is_empty() { stdout } else { stderr };
            Err(DispatchError::DmcpError(msg.trim().to_string()))
        }
    }

    /// Browse servers via `dmcp browse -k <keywords> --json`.
    pub async fn browse(keywords: &[String]) -> Result<serde_json::Value> {
        let mut cmd = Command::new("dmcp");
        cmd.arg("browse").arg("--json");
        for kw in keywords {
            cmd.arg("-k").arg(kw);
        }

        let output = cmd.output().await.map_err(|e| {
            DispatchError::DmcpError(format!("failed to spawn dmcp: {}", e))
        })?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        if output.status.success() {
            serde_json::from_str(stdout.trim())
                .map_err(|e| DispatchError::DmcpError(format!("invalid JSON from dmcp browse: {}", e)))
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(DispatchError::DmcpError(
                stderr.trim().to_string(),
            ))
        }
    }

    /// List tools for a server via `dmcp tools <id> --json`.
    pub async fn list_tools(server: &str) -> Result<serde_json::Value> {
        let output = Command::new("dmcp")
            .arg("tools")
            .arg(server)
            .arg("--json")
            .output()
            .await
            .map_err(|e| {
                DispatchError::DmcpError(format!("failed to spawn dmcp: {}", e))
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        if output.status.success() {
            serde_json::from_str(stdout.trim())
                .map_err(|e| DispatchError::DmcpError(format!("invalid JSON from dmcp tools: {}", e)))
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(DispatchError::DmcpError(
                stderr.trim().to_string(),
            ))
        }
    }
}
