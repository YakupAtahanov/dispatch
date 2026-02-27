use std::process;

use dispatch::mcp_client::DmcpClient;
use dispatch::mcp_server;

fn print_usage() {
    eprintln!("dispatch — Signal-driven task orchestrator for MCP servers");
    eprintln!();
    eprintln!("Usage:");
    eprintln!("  dispatch serve    Run as MCP server (stdio)");
    eprintln!("  dispatch help     Show this help message");
    eprintln!();
    eprintln!("dispatch exposes itself as an MCP server. Connect from any");
    eprintln!("MCP client (Claude, Cursor, etc.) via stdio.");
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();

    let command = args.get(1).map(|s| s.as_str()).unwrap_or("help");

    match command {
        "serve" => {
            // Check dmcp availability before starting
            if let Err(e) = DmcpClient::check_available().await {
                eprintln!("Error: {}", e);
                eprintln!("Make sure dmcp is installed and on your PATH.");
                eprintln!("  cargo install --git https://github.com/YakupAtahanov/dmcp");
                process::exit(1);
            }

            if let Err(e) = mcp_server::serve().await {
                eprintln!("Server error: {}", e);
                process::exit(1);
            }
        }
        "help" | "--help" | "-h" => {
            print_usage();
        }
        _ => {
            eprintln!("Unknown command: {}", command);
            print_usage();
            process::exit(1);
        }
    }
}
