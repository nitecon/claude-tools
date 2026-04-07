//! Unified MCP server merging 9 code tools and 4 comms tools.
//!
//! All tool methods live in the single `#[tool_router]` impl block as required
//! by rmcp. Parameter structs are defined inline for clarity.

use agent_comms::gateway::GatewayClient;
use rmcp::{
    handler::server::router::tool::ToolRouter,
    handler::server::wrapper::Parameters,
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router, ServerHandler,
};
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

// ── Session state (comms) ────────────────────────────────────────────────────

#[derive(Default)]
struct CommsSession {
    ident: Option<String>,
    channel_name: Option<String>,
}

// ── Parameter structs — code tools ───────────────────────────────────────────

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TreeParams {
    #[schemars(description = "Directory to display (default: current directory)")]
    path: Option<String>,
    #[schemars(description = "Maximum depth (default: 3)")]
    depth: Option<u64>,
    #[schemars(description = "Max files per directory before truncation (default: 20)")]
    max_files: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ListParams {
    #[schemars(description = "Directory to list (default: current directory)")]
    path: Option<String>,
    #[schemars(description = "Show file sizes")]
    sizes: Option<bool>,
    #[schemars(description = "Show hidden files")]
    show_hidden: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FileOpsParams {
    #[schemars(description = "Operation: copy, move, mkdir, or remove")]
    operation: String,
    #[schemars(description = "Source path")]
    src: String,
    #[schemars(description = "Destination path (for copy/move)")]
    dst: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ExtractSymbolParams {
    #[schemars(description = "Symbol name to extract")]
    name: String,
    #[schemars(description = "File to search in. If not provided, searches the symbol index.")]
    file: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ListSymbolsParams {
    #[schemars(description = "File to list symbols from")]
    file: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SearchSymbolsParams {
    #[schemars(description = "Symbol name search query")]
    query: String,
    #[schemars(description = "Symbol type filter (fn, class, struct, enum, trait, etc.)")]
    kind: Option<String>,
    #[schemars(description = "File path pattern filter")]
    file_pattern: Option<String>,
    #[schemars(description = "Max results (default: 20)")]
    limit: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct BuildIndexParams {
    #[schemars(description = "Directory to index (default: current directory)")]
    path: Option<String>,
    #[schemars(description = "Force full rebuild")]
    rebuild: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FindFilesParams {
    #[schemars(description = "File name pattern")]
    pattern: Option<String>,
    #[schemars(description = "File extension filter")]
    extension: Option<String>,
    #[schemars(description = "Minimum file size in bytes")]
    min_size: Option<u64>,
    #[schemars(description = "Maximum file size in bytes")]
    max_size: Option<u64>,
    #[schemars(description = "Max results (default: 20)")]
    limit: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ProjectSummaryParams {
    #[schemars(description = "Directory to summarize (default: current directory)")]
    path: Option<String>,
}

// ── Parameter structs — comms tools ──────────────────────────────────────────

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SetIdentityParams {
    #[schemars(
        description = "Git remote URL (e.g. github.com/org/repo.git) or directory name identifying this project"
    )]
    project_ident: String,
    #[schemars(
        description = "Channel plugin to use: 'discord', 'slack', 'email', etc. Omit to use the gateway's default."
    )]
    channel: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SendMessageParams {
    #[schemars(description = "The message content to send to the user")]
    content: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ConfirmReadParams {
    #[schemars(
        description = "The numeric message ID to confirm as read. Get this from the get_messages output."
    )]
    message_id: i64,
}

// ── Server struct ────────────────────────────────────────────────────────────

/// Unified MCP server exposing code exploration, file operations, and
/// optional gateway-backed communication tools.
#[derive(Clone)]
pub struct AgentToolsServer {
    tool_router: ToolRouter<Self>,
    gateway: Option<GatewayClient>,
    session: Arc<Mutex<CommsSession>>,
}

// ── Constant for gateway-not-configured message ──────────────────────────────

const NO_GATEWAY_MSG: &str =
    "Gateway not configured. Please ask the user to run: agent-tools setup gateway";

// ── tool_router impl — all 13 tools ─────────────────────────────────────────

#[tool_router]
impl AgentToolsServer {
    /// Create a new `AgentToolsServer`.
    ///
    /// Pass `Some(GatewayClient)` to enable comms tools, or `None` to run in
    /// code-only mode (comms tools will return a configuration hint).
    pub fn new(gateway: Option<GatewayClient>) -> Self {
        Self {
            tool_router: Self::tool_router(),
            gateway,
            session: Arc::new(Mutex::new(CommsSession::default())),
        }
    }

    // ── Code tools (1-9) ─────────────────────────────────────────────────────

    /// Token-efficient directory tree view.
    #[tool(
        description = "Token-efficient directory tree view. Respects .gitignore, configurable depth and max files per directory."
    )]
    fn tree(&self, Parameters(params): Parameters<TreeParams>) -> String {
        let path = params
            .path
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let depth = params.depth.unwrap_or(3) as usize;
        let max_files = params.max_files.unwrap_or(20) as usize;

        let options = agent_fs::tree::TreeOptions {
            max_depth: depth,
            max_files_per_dir: max_files,
        };

        match agent_fs::tree::tree(&path, &options) {
            Ok(tree) => agent_fs::tree::render_tree_text(&tree, 0),
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Smart directory listing.
    #[tool(
        description = "Smart directory listing. Directories first, then files alphabetically. Minimal output."
    )]
    fn list(&self, Parameters(params): Parameters<ListParams>) -> String {
        let path = params
            .path
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let sizes = params.sizes.unwrap_or(false);
        let show_hidden = params.show_hidden.unwrap_or(false);

        let options = agent_fs::list::ListOptions {
            show_sizes: sizes,
            show_hidden,
        };

        match agent_fs::list::list_dir(&path, &options) {
            Ok(entries) => agent_fs::list::render_list_text(&entries),
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Cross-platform file operations.
    #[tool(
        description = "Cross-platform file operations: copy, move, mkdir, remove. Pure Rust, no shell commands."
    )]
    fn file_ops(&self, Parameters(params): Parameters<FileOpsParams>) -> String {
        let src_path = PathBuf::from(&params.src);

        match params.operation.as_str() {
            "copy" => {
                let Some(dst) = params.dst.as_deref() else {
                    return "Error: Missing 'dst' parameter for copy".to_string();
                };
                match agent_fs::ops::copy(&src_path, &PathBuf::from(dst)) {
                    Ok(()) => format!("Copied {} -> {}", params.src, dst),
                    Err(e) => format!("Error: {e}"),
                }
            }
            "move" => {
                let Some(dst) = params.dst.as_deref() else {
                    return "Error: Missing 'dst' parameter for move".to_string();
                };
                match agent_fs::ops::move_path(&src_path, &PathBuf::from(dst)) {
                    Ok(()) => format!("Moved {} -> {}", params.src, dst),
                    Err(e) => format!("Error: {e}"),
                }
            }
            "mkdir" => match agent_fs::ops::mkdir(&src_path) {
                Ok(()) => format!("Created {}", params.src),
                Err(e) => format!("Error: {e}"),
            },
            "remove" => match agent_fs::ops::remove(&src_path) {
                Ok(()) => format!("Removed {}", params.src),
                Err(e) => format!("Error: {e}"),
            },
            other => format!("Error: Unknown operation: {other}"),
        }
    }

    /// Extract a symbol's complete source code by name.
    #[tool(
        description = "Extract a symbol's complete source code by name from a file. Returns the full definition with line numbers."
    )]
    fn extract_symbol(&self, Parameters(params): Parameters<ExtractSymbolParams>) -> String {
        let mut parser = agent_symbols::SymbolParser::new();

        if let Some(ref file) = params.file {
            let path = PathBuf::from(file);
            match parser.extract_symbol(&path, &params.name) {
                Ok(Some(source)) => format!("{source}"),
                Ok(None) => format!("Symbol '{}' not found in {}", params.name, file),
                Err(e) => format!("Error: {e}"),
            }
        } else {
            let root = match std::env::current_dir() {
                Ok(r) => r,
                Err(e) => return format!("Error: {e}"),
            };
            let index = match agent_symbols::SymbolIndex::open_for_project(&root) {
                Ok(i) => i,
                Err(e) => return format!("Error: {e}"),
            };
            let results = match index.search(&params.name, None, None, 5) {
                Ok(r) => r,
                Err(e) => return format!("Error: {e}"),
            };

            if results.is_empty() {
                return format!("Symbol '{}' not found in index", params.name);
            }

            let first = &results[0];
            match parser.extract_symbol(&first.file, &params.name) {
                Ok(Some(source)) => format!("{source}"),
                Ok(None) => {
                    let mut text = String::new();
                    for r in &results {
                        text.push_str(&format!(
                            "{} {} {}:{}-{}\n",
                            r.kind,
                            r.name,
                            r.file.display(),
                            r.start_line,
                            r.end_line
                        ));
                    }
                    text
                }
                Err(e) => format!("Error: {e}"),
            }
        }
    }

    /// List all symbols in a file.
    #[tool(
        description = "List all symbols (functions, classes, structs, etc.) in a file with their types and line ranges."
    )]
    fn list_symbols(&self, Parameters(params): Parameters<ListSymbolsParams>) -> String {
        let path = PathBuf::from(&params.file);
        let mut parser = agent_symbols::SymbolParser::new();

        let symbols = match parser.parse_file(&path) {
            Ok(s) => s,
            Err(e) => return format!("Error: {e}"),
        };

        let mut text = String::new();
        for s in &symbols {
            let parent_info = s
                .parent
                .as_ref()
                .map(|p| format!(" (in {p})"))
                .unwrap_or_default();
            text.push_str(&format!(
                "{:<10} {:<30} L{}-{}{}\n",
                format!("{}", s.kind),
                s.name,
                s.start_line,
                s.end_line,
                parent_info,
            ));
        }

        if text.is_empty() {
            text = format!("No symbols found in {}", params.file);
        }

        text
    }

    /// Search the project-wide symbol index.
    #[tool(description = "Search the project-wide symbol index by name, type, or file pattern.")]
    fn search_symbols(&self, Parameters(params): Parameters<SearchSymbolsParams>) -> String {
        let kind = params.kind.as_deref();
        let file_pattern = params.file_pattern.as_deref();
        let limit = params.limit.unwrap_or(20) as usize;

        let root = match std::env::current_dir() {
            Ok(r) => r,
            Err(e) => return format!("Error: {e}"),
        };
        let index = match agent_symbols::SymbolIndex::open_for_project(&root) {
            Ok(i) => i,
            Err(e) => return format!("Error: {e}"),
        };
        let results = match index.search(&params.query, kind, file_pattern, limit) {
            Ok(r) => r,
            Err(e) => return format!("Error: {e}"),
        };

        if results.is_empty() {
            return format!("No symbols found matching '{}'", params.query);
        }

        let mut text = String::new();
        for r in &results {
            text.push_str(&format!(
                "{:<10} {:<30} {}:{}-{}\n",
                format!("{}", r.kind),
                r.name,
                r.file.display(),
                r.start_line,
                r.end_line
            ));
        }

        text
    }

    /// Build or update the project index.
    #[tool(description = "Build or incrementally update the project file and symbol index.")]
    fn build_index(&self, Parameters(params): Parameters<BuildIndexParams>) -> String {
        let root = params
            .path
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let rebuild = params.rebuild.unwrap_or(false);

        if rebuild {
            let data_dir = agent_core::project_data_dir(&root);
            if data_dir.exists() {
                if let Err(e) = std::fs::remove_dir_all(&data_dir) {
                    return format!("Error removing data dir: {e}");
                }
            }
        }

        let file_indexer = match agent_search::indexer::FileIndexer::open_for_project(&root) {
            Ok(i) => i,
            Err(e) => return format!("Error: {e}"),
        };
        let file_stats = match file_indexer.build(&root, true) {
            Ok(s) => s,
            Err(e) => return format!("Error building file index: {e}"),
        };

        let symbol_index = match agent_symbols::SymbolIndex::open_for_project(&root) {
            Ok(i) => i,
            Err(e) => return format!("Error: {e}"),
        };
        let symbol_stats = match symbol_index.build(&root) {
            Ok(s) => s,
            Err(e) => return format!("Error building symbol index: {e}"),
        };

        let (file_count, symbol_count) = match symbol_index.stats() {
            Ok(s) => s,
            Err(e) => return format!("Error reading stats: {e}"),
        };

        format!(
            "Files: {file_stats}\nSymbols: {symbol_stats}\nTotal: {file_count} files, {symbol_count} symbols"
        )
    }

    /// Search the file index.
    #[tool(description = "Search the file index by name pattern, extension, or size range.")]
    fn find_files(&self, Parameters(params): Parameters<FindFilesParams>) -> String {
        let pattern = params.pattern.as_deref();
        let extension = params.extension.as_deref();
        let min_size = params.min_size;
        let max_size = params.max_size;
        let limit = params.limit.unwrap_or(20) as usize;

        let root = match std::env::current_dir() {
            Ok(r) => r,
            Err(e) => return format!("Error: {e}"),
        };
        let indexer = match agent_search::indexer::FileIndexer::open_for_project(&root) {
            Ok(i) => i,
            Err(e) => return format!("Error: {e}"),
        };

        let results = match agent_search::query::find_files(
            &indexer, pattern, extension, min_size, max_size, limit,
        ) {
            Ok(r) => r,
            Err(e) => return format!("Error: {e}"),
        };

        if results.is_empty() {
            return "No files found".to_string();
        }

        let mut text = String::new();
        for r in &results {
            text.push_str(&format!("{}\n", r.path));
        }

        text
    }

    /// Generate a compact project overview.
    #[tool(
        description = "Generate a compact project overview with language breakdown, file counts, and key files."
    )]
    fn project_summary(&self, Parameters(params): Parameters<ProjectSummaryParams>) -> String {
        let root = params
            .path
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

        let indexer = match agent_search::indexer::FileIndexer::open_for_project(&root) {
            Ok(i) => i,
            Err(e) => return format!("Error: {e}"),
        };

        if indexer.file_count().unwrap_or(0) == 0 {
            if let Err(e) = indexer.build(&root, false) {
                return format!("Error building index: {e}");
            }
        }

        match agent_search::query::project_summary(&indexer) {
            Ok(summary) => agent_search::query::render_summary_text(&summary),
            Err(e) => format!("Error: {e}"),
        }
    }

    // ── Comms tools (10-13) ──────────────────────────────────────────────────

    /// Set the project identity for this session.
    #[tool(
        description = "Set the project identity for this agent session. Pass a git remote URL (e.g. github.com/org/repo.git) or a directory name. Optionally specify a channel plugin (discord, slack, email). Must be called before send_message or get_messages."
    )]
    async fn set_identity(
        &self,
        Parameters(SetIdentityParams {
            project_ident,
            channel,
        }): Parameters<SetIdentityParams>,
    ) -> String {
        let Some(ref gw) = self.gateway else {
            return NO_GATEWAY_MSG.to_string();
        };

        match gw
            .register_project(&project_ident, channel.as_deref())
            .await
        {
            Ok(resp) => {
                let mut s = self.session.lock().unwrap();
                s.ident = Some(resp.ident.clone());
                s.channel_name = Some(resp.channel_name.clone());
                format!(
                    "Identity set to '{}' via {} channel.",
                    resp.ident, resp.channel_name
                )
            }
            Err(e) => format!("Error registering project: {e}"),
        }
    }

    /// Send a message to the user via the project's configured channel.
    #[tool(
        description = "Send a message to the user via the project's configured channel. set_identity must be called first."
    )]
    async fn send_message(
        &self,
        Parameters(SendMessageParams { content }): Parameters<SendMessageParams>,
    ) -> String {
        let Some(ref gw) = self.gateway else {
            return NO_GATEWAY_MSG.to_string();
        };

        let ident = {
            let s = self.session.lock().unwrap();
            s.ident.clone()
        };

        let Some(ident) = ident else {
            return "Error: identity not set. Call set_identity first.".to_string();
        };

        match gw.send_message(&ident, &content).await {
            Ok(resp) => format!("Message sent (id={}).", resp.message_id),
            Err(e) => format!("Error sending message: {e}"),
        }
    }

    /// Get unconfirmed messages from the project's channel.
    #[tool(
        description = "Get unconfirmed messages from the project's channel. Returns messages with their IDs. You MUST call confirm_read for each message after you have read and acted on it. Messages will keep reappearing until confirmed. set_identity must be called first."
    )]
    async fn get_messages(&self) -> String {
        let Some(ref gw) = self.gateway else {
            return NO_GATEWAY_MSG.to_string();
        };

        let ident = {
            let s = self.session.lock().unwrap();
            s.ident.clone()
        };

        let Some(ident) = ident else {
            return "Error: identity not set. Call set_identity first.".to_string();
        };

        match gw.get_unread(&ident).await {
            Ok(resp) => {
                if resp.messages.is_empty() {
                    return "no messages".to_string();
                }
                let mut lines: Vec<String> = resp
                    .messages
                    .iter()
                    .map(|m| {
                        let prefix = if m.source == "agent" {
                            "[AGENT]"
                        } else {
                            "[USER]"
                        };
                        format!("(id={}) {} {}", m.id, prefix, m.content)
                    })
                    .collect();
                lines.push(String::new());
                lines.push(
                    "IMPORTANT: You MUST call confirm_read for each message above \
                     (by its id) after you have read and acted on it. \
                     Unconfirmed messages will reappear on the next get_messages call."
                        .to_string(),
                );
                lines.join("\n")
            }
            Err(e) => format!("Error fetching messages: {e}"),
        }
    }

    /// Confirm that a message has been read and acted upon.
    #[tool(
        description = "Confirm that you have read and acted on a specific message. You MUST call this for every message returned by get_messages after you have handled it. Pass the numeric message_id from the get_messages output. Messages will keep reappearing in get_messages until confirmed."
    )]
    async fn confirm_read(
        &self,
        Parameters(ConfirmReadParams { message_id }): Parameters<ConfirmReadParams>,
    ) -> String {
        let Some(ref gw) = self.gateway else {
            return NO_GATEWAY_MSG.to_string();
        };

        let ident = {
            let s = self.session.lock().unwrap();
            s.ident.clone()
        };

        let Some(ident) = ident else {
            return "Error: identity not set. Call set_identity first.".to_string();
        };

        match gw.confirm_read(&ident, message_id).await {
            Ok(resp) => {
                if resp.confirmed {
                    format!("Message {message_id} confirmed as read.")
                } else {
                    format!("Message {message_id} was already confirmed or does not exist.")
                }
            }
            Err(e) => format!("Error confirming message: {e}"),
        }
    }
}

// ── Public helper for pre-setting default identity ───────────────────────────

impl AgentToolsServer {
    /// Pre-set the project identity (used when `DEFAULT_PROJECT_IDENT` is configured).
    pub fn set_default_ident(&self, ident: String, channel_name: String) {
        let mut s = self.session.lock().unwrap();
        s.ident = Some(ident);
        s.channel_name = Some(channel_name);
    }
}

// ── ServerHandler trait ──────────────────────────────────────────────────────

#[tool_handler]
impl ServerHandler for AgentToolsServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "agent-tools: Code exploration, symbol extraction, file operations, \
             and optional communication with users via gateway channels. \
             Code tools work immediately. Comms tools (set_identity, send_message, \
             get_messages, confirm_read) require gateway configuration."
                .to_string(),
        )
    }
}
