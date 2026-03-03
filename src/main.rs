use std::process;

use dispatch::mcp_client::DmcpClient;
use dispatch::mcp_server;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

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
    // Initialize tracing — logs go to stderr so they don't interfere with
    // the JSON-RPC protocol on stdout.  Control verbosity with RUST_LOG:
    //   RUST_LOG=dispatch=debug  dispatch serve
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("dispatch=info")),
        )
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();

    let args: Vec<String> = std::env::args().collect();

    let command = args.get(1).map(|s| s.as_str()).unwrap_or("help");

    match command {
        "serve" => {
            // Check dmcp availability before starting
            if let Err(e) = DmcpClient::check_available().await {
                error!("dmcp not found: {}", e);
                eprintln!("Make sure dmcp is installed and on your PATH.");
                eprintln!("  cargo install --git https://github.com/YakupAtahanov/dmcp");
                process::exit(1);
            }

            info!("starting dispatch server (stdio)");
            if let Err(e) = mcp_server::serve().await {
                error!("server error: {}", e);
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
