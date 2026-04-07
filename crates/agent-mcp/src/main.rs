//! Entry point for the unified `agent-tools-mcp` MCP server.
//!
//! Merges 9 code tools (tree, list, file_ops, extract_symbol, list_symbols,
//! search_symbols, build_index, find_files, project_summary) with 4 comms
//! tools (set_identity, send_message, get_messages, confirm_read) into a
//! single rmcp-based server served over stdio.

mod server;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rmcp::ServiceExt;
use tokio::io::{stdin, stdout};

// ── CLI definition ───────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "agent-tools-mcp", about = "agent-tools MCP server")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Gateway base URL (override config)
    #[arg(long, env = "GATEWAY_URL")]
    url: Option<String>,

    /// Gateway API key (override config)
    #[arg(long, env = "GATEWAY_API_KEY")]
    api_key: Option<String>,

    /// Default project identity
    #[arg(long, env = "DEFAULT_PROJECT_IDENT")]
    default_project: Option<String>,

    /// HTTP timeout in milliseconds
    #[arg(long, env = "GATEWAY_TIMEOUT_MS")]
    timeout_ms: Option<u64>,
}

#[derive(Subcommand)]
enum Command {
    /// Interactive setup -- creates ~/.agentic/agent-tools/gateway.conf
    Init,
    /// Check for a newer version and update the binary in place
    Update,
}

// ── Entry point ──────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    agent_updater::cleanup_old_binaries();

    // Load config (gateway.conf + env)
    let config = agent_comms::config::load_config();

    let cli = Cli::parse();

    if let Some(Command::Init) = cli.command {
        return agent_comms::config::run_setup_gateway();
    }
    if let Some(Command::Update) = cli.command {
        return agent_updater::manual_update_blocking();
    }

    // Log to stderr so it does not corrupt the stdio MCP stream.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_env("RUST_LOG")
                .add_directive("agent_tools=info".parse()?),
        )
        .init();

    // Build gateway client if configured
    let cli_url_provided = cli.url.is_some();
    let gw_url = cli.url.or(config.gateway.url);
    let gw_key = cli.api_key.or(config.gateway.api_key);
    let gw_timeout = cli.timeout_ms.or(config.gateway.timeout_ms).unwrap_or(5000);

    let gateway = match (gw_url, gw_key) {
        (Some(url), Some(key)) => {
            match agent_comms::gateway::GatewayClient::new(url, key, gw_timeout) {
                Ok(gw) => Some(gw),
                Err(e) => {
                    tracing::warn!("Failed to create gateway client: {e}");
                    None
                }
            }
        }
        _ => None,
    };

    if cli_url_provided && gateway.is_none() {
        tracing::warn!(
            "--url was provided but gateway client could not be created (missing --api-key?)"
        );
    }

    let server = server::AgentToolsServer::new(gateway.clone());

    // Auto-register default identity if configured
    let default_project = cli.default_project.or(config.gateway.default_project);
    if let (Some(ident), Some(gw)) = (default_project.as_deref(), &gateway) {
        match gw.register_project(ident, None).await {
            Ok(resp) => server.set_default_ident(resp.ident, resp.channel_name),
            Err(e) => tracing::warn!("Failed to auto-register '{ident}': {e}"),
        }
    }

    // Background update check
    agent_updater::spawn_update_check();

    let transport = (stdin(), stdout());
    let running = server.serve(transport).await.context("serve MCP")?;
    running.waiting().await.context("MCP server closed")?;

    Ok(())
}
