//! ClubBench MCP server binary.
//!
//! Starts an MCP Streamable HTTP server exposing the headless environment to
//! MCP-capable agents. Runs on a **single-threaded** tokio runtime so the
//! thread-local seeded RNG keeps episodes deterministic.

use clap::Parser;

#[derive(Parser)]
#[command(name = "clubbench-mcp", about = "ClubBench MCP server")]
struct Cli {
    /// Port to listen on
    #[arg(long, default_value_t = 8890)]
    port: u16,
}

fn main() -> Result<(), String> {
    let cli = Cli::parse();
    println!("ClubBench MCP server on 127.0.0.1:{}", cli.port);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("failed to build runtime: {e}"))?;
    runtime.block_on(clubbench::mcp::serve(cli.port))
}
