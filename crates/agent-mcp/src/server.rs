//! Unified MCP server merging 9 code tools, 6 comms tools, and 5 sync tools.
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
    agent_id: Option<String>,
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
    #[schemars(
        description = "A short, unique identifier for this agent instance (e.g., 'sre-agent', 'deploy-agent'). Use lowercase with hyphens. Enables per-agent message buffers."
    )]
    agent_id: Option<String>,
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

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ReplyToParams {
    #[schemars(description = "The numeric message ID to reply to.")]
    message_id: i64,
    #[schemars(description = "The reply text.")]
    content: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TakingActionOnParams {
    #[schemars(description = "The numeric message ID being acted on.")]
    message_id: i64,
    #[schemars(description = "A brief description of what action is being taken.")]
    message: String,
}

// ── Parameter structs — sync tools ──────────────────────────────────────────

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SyncPushParams {
    #[schemars(
        description = "Path to skill directory (must contain SKILL.md), or .md file for commands/agents"
    )]
    path: String,
    #[schemars(description = "Resource kind: 'skill', 'command', or 'agent'")]
    kind: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SyncPullParams {
    #[schemars(description = "Name of the resource to pull from the gateway")]
    name: String,
    #[schemars(description = "Destination directory (default: current directory)")]
    destination: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SyncListParams {
    #[schemars(
        description = "Filter by kind: 'skill', 'command', 'agent', or 'all' (default: 'all')"
    )]
    kind: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SyncDeleteParams {
    #[schemars(description = "Name of the resource to delete from the gateway")]
    name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SyncAllParams {
    #[schemars(
        description = "Root directory containing skill subdirectories and command .md files (default: current directory)"
    )]
    directory: Option<String>,
}

// ── Server struct ────────────────────────────────────────────────────────────

/// Unified MCP server exposing code exploration, file operations, and
/// optional gateway-backed communication tools.
#[derive(Clone)]
pub struct AgentToolsServer {
    tool_router: ToolRouter<Self>,
    gateway: Option<GatewayClient>,
    sync_client: Option<Arc<agent_sync::client::SyncClient>>,
    session: Arc<Mutex<CommsSession>>,
}

// ── Constant for gateway-not-configured message ──────────────────────────────

const NO_GATEWAY_MSG: &str =
    "Gateway not configured. Please ask the user to run: agent-tools setup gateway";

const NO_SYNC_MSG: &str = "Sync not configured. Please ask the user to run: agent-tools-mcp init";

// ── tool_router impl — all 20 tools ─────────────────────────────────────────

#[tool_router]
impl AgentToolsServer {
    /// Create a new `AgentToolsServer`.
    ///
    /// Pass `Some(GatewayClient)` to enable comms tools, or `None` to run in
    /// code-only mode (comms tools will return a configuration hint).
    pub fn new(
        gateway: Option<GatewayClient>,
        sync_client: Option<agent_sync::client::SyncClient>,
    ) -> Self {
        Self {
            tool_router: Self::tool_router(),
            gateway,
            sync_client: sync_client.map(Arc::new),
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
            agent_id,
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
                s.agent_id = agent_id;
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

        let (ident, agent_id) = {
            let s = self.session.lock().unwrap();
            (s.ident.clone(), s.agent_id.clone())
        };

        let Some(ident) = ident else {
            return "Error: identity not set. Call set_identity first.".to_string();
        };

        match gw.send_message(&ident, &content, agent_id.as_deref()).await {
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

        let (ident, agent_id) = {
            let s = self.session.lock().unwrap();
            (s.ident.clone(), s.agent_id.clone())
        };

        let Some(ident) = ident else {
            return "Error: identity not set. Call set_identity first.".to_string();
        };

        match gw.get_unread(&ident, agent_id.as_deref()).await {
            Ok(resp) => {
                if resp.messages.is_empty() {
                    return "no messages".to_string();
                }
                let mut lines: Vec<String> = resp
                    .messages
                    .iter()
                    .map(|m| {
                        let source_tag = match (m.source.as_str(), m.agent_id.as_deref()) {
                            ("agent", Some(aid)) => format!("[AGENT:{}]", aid),
                            ("agent", None) => "[AGENT]".to_string(),
                            _ => "[USER]".to_string(),
                        };
                        let type_tag = match m.message_type.as_deref() {
                            Some("reply") => " [REPLY]",
                            Some("action") => " [ACTION]",
                            _ => "",
                        };
                        let parent = m
                            .parent_message_id
                            .map(|pid| format!(" (re: msg {})", pid))
                            .unwrap_or_default();
                        format!(
                            "(id={}) {}{}{} {}",
                            m.id, source_tag, type_tag, parent, m.content
                        )
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

        let (ident, agent_id) = {
            let s = self.session.lock().unwrap();
            (s.ident.clone(), s.agent_id.clone())
        };

        let Some(ident) = ident else {
            return "Error: identity not set. Call set_identity first.".to_string();
        };

        match gw
            .confirm_read(&ident, message_id, agent_id.as_deref())
            .await
        {
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

    // ── Comms tools (14-15) ─────────────────────────────────────────────────

    /// Reply to a specific message.
    #[tool(
        description = "Send a reply to a specific message. Creates a native threaded reply in Discord. Other agents will see this reply in their unread queues. set_identity must be called first."
    )]
    async fn reply_to(
        &self,
        Parameters(ReplyToParams {
            message_id,
            content,
        }): Parameters<ReplyToParams>,
    ) -> String {
        let Some(ref gw) = self.gateway else {
            return NO_GATEWAY_MSG.to_string();
        };

        let (ident, agent_id) = {
            let s = self.session.lock().unwrap();
            (s.ident.clone(), s.agent_id.clone())
        };

        let Some(ident) = ident else {
            return "Error: identity not set. Call set_identity first.".to_string();
        };

        match gw
            .reply_to(&ident, message_id, &content, agent_id.as_deref())
            .await
        {
            Ok(resp) => format!(
                "Reply sent (id={}, parent={}).",
                resp.message_id, resp.parent_message_id
            ),
            Err(e) => format!("Error replying to message: {e}"),
        }
    }

    /// Signal that this agent is taking action on a message.
    #[tool(
        description = "Signal that this agent is actively working on a specific message. Posts a Discord thread reply formatted as [ACTION:agent-id] so users and other agents know the task is claimed. Call this before starting work on a request, then use reply_to when the work is complete. set_identity must be called first."
    )]
    async fn taking_action_on(
        &self,
        Parameters(TakingActionOnParams {
            message_id,
            message,
        }): Parameters<TakingActionOnParams>,
    ) -> String {
        let Some(ref gw) = self.gateway else {
            return NO_GATEWAY_MSG.to_string();
        };

        let (ident, agent_id) = {
            let s = self.session.lock().unwrap();
            (s.ident.clone(), s.agent_id.clone())
        };

        let Some(ident) = ident else {
            return "Error: identity not set. Call set_identity first.".to_string();
        };

        match gw
            .taking_action_on(&ident, message_id, &message, agent_id.as_deref())
            .await
        {
            Ok(resp) => format!(
                "Action signal sent (id={}, parent={}).",
                resp.message_id, resp.parent_message_id
            ),
            Err(e) => format!("Error signaling action: {e}"),
        }
    }

    // ── Sync tools (16-20) ──────────────────────────────────────────────────

    /// Push a skill, command, or agent to the gateway.
    #[tool(
        description = "Push a skill, command, or agent to the gateway for sharing across machines. For skills: pass a directory path containing SKILL.md, or a single .md file. For commands/agents: pass a .md file path. The resource name is derived from the directory or file name."
    )]
    async fn sync_push(
        &self,
        Parameters(SyncPushParams { path, kind }): Parameters<SyncPushParams>,
    ) -> String {
        let Some(ref client) = self.sync_client else {
            return NO_SYNC_MSG.to_string();
        };

        let path = std::path::PathBuf::from(&path);
        let path = match path.canonicalize() {
            Ok(p) => p,
            Err(e) => return format!("Error: could not resolve path '{}': {e}", path.display()),
        };

        match kind.as_str() {
            "skill" => {
                if path.is_dir() {
                    if !path.join("SKILL.md").exists() {
                        return format!("Error: '{}' does not contain SKILL.md", path.display());
                    }
                    let name = agent_comms::sanitize::sanitize_name(
                        &path.file_name().unwrap_or_default().to_string_lossy(),
                    );
                    if name.is_empty() {
                        return "Error: could not derive skill name from directory".to_string();
                    }
                    match agent_sync::zip_util::zip_skill_dir(&path) {
                        Ok((zip_bytes, checksum)) => {
                            let size = zip_bytes.len();
                            match client.upload(&name, zip_bytes).await {
                                Ok(_) => format!(
                                    "Pushed skill '{}' ({} bytes, checksum: {})",
                                    name,
                                    size,
                                    &checksum[..12]
                                ),
                                Err(e) => format!("Error uploading skill: {e}"),
                            }
                        }
                        Err(e) => format!("Error zipping skill: {e}"),
                    }
                } else if path.is_file() {
                    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if ext != "md" {
                        return format!("Error: skill file must have .md extension, got '.{ext}'");
                    }
                    let name = agent_comms::sanitize::sanitize_name(
                        &path.file_stem().unwrap_or_default().to_string_lossy(),
                    );
                    if name.is_empty() {
                        return "Error: could not derive skill name from file".to_string();
                    }
                    match agent_sync::zip_util::zip_single_file(&path) {
                        Ok((zip_bytes, checksum)) => {
                            let size = zip_bytes.len();
                            match client.upload(&name, zip_bytes).await {
                                Ok(_) => format!(
                                    "Pushed skill '{}' ({} bytes, checksum: {})",
                                    name,
                                    size,
                                    &checksum[..12]
                                ),
                                Err(e) => format!("Error uploading skill: {e}"),
                            }
                        }
                        Err(e) => format!("Error zipping skill file: {e}"),
                    }
                } else {
                    format!(
                        "Error: '{}' is neither a file nor a directory",
                        path.display()
                    )
                }
            }
            "command" | "agent" => {
                if !path.is_file() {
                    return format!("Error: {} push requires a .md file", kind);
                }
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if ext != "md" {
                    return format!("Error: {} file must have .md extension, got '.{ext}'", kind);
                }
                let name = agent_comms::sanitize::sanitize_name(
                    &path.file_stem().unwrap_or_default().to_string_lossy(),
                );
                if name.is_empty() {
                    return format!("Error: could not derive {} name from file", kind);
                }
                let markdown = match std::fs::read_to_string(&path) {
                    Ok(s) => s,
                    Err(e) => return format!("Error reading file: {e}"),
                };
                if markdown.is_empty() {
                    return format!("Error: {} file is empty", kind);
                }
                let size = markdown.len();
                let result = if kind == "command" {
                    client.upload_command(&name, markdown).await
                } else {
                    client.upload_agent(&name, markdown).await
                };
                match result {
                    Ok(_) => format!("Pushed {} '{}' ({} bytes)", kind, name, size),
                    Err(e) => format!("Error uploading {}: {e}", kind),
                }
            }
            other => format!(
                "Error: unknown kind '{}'. Use 'skill', 'command', or 'agent'.",
                other
            ),
        }
    }

    /// Pull a resource from the gateway.
    #[tool(
        description = "Pull a skill, command, or agent from the gateway to the local machine. Skills are extracted as directories; commands and agents are saved as .md files."
    )]
    async fn sync_pull(
        &self,
        Parameters(SyncPullParams { name, destination }): Parameters<SyncPullParams>,
    ) -> String {
        let Some(ref client) = self.sync_client else {
            return NO_SYNC_MSG.to_string();
        };

        let dest = destination
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("."));

        match client.download(&name).await {
            Ok(result) => match result.kind.as_str() {
                "command" | "agent" => {
                    let dest_file = dest.join(format!("{}.md", name));
                    if let Some(parent) = dest_file.parent() {
                        if let Err(e) = std::fs::create_dir_all(parent) {
                            return format!("Error creating directory: {e}");
                        }
                    }
                    match std::fs::write(&dest_file, &result.bytes) {
                        Ok(()) => format!(
                            "Pulled {} '{}' -> {}",
                            result.kind,
                            name,
                            dest_file.display()
                        ),
                        Err(e) => format!("Error writing file: {e}"),
                    }
                }
                _ => match agent_sync::zip_util::unzip_skill(&name, &result.bytes, &dest) {
                    Ok(out) => format!("Pulled skill '{}' -> {}", name, out.display()),
                    Err(e) => format!("Error extracting skill: {e}"),
                },
            },
            Err(e) => format!("Error downloading '{}': {e}", name),
        }
    }

    /// List resources on the gateway.
    #[tool(
        description = "List skills, commands, and/or agents stored on the gateway. Filter by kind or list all."
    )]
    async fn sync_list(
        &self,
        Parameters(SyncListParams { kind }): Parameters<SyncListParams>,
    ) -> String {
        let Some(ref client) = self.sync_client else {
            return NO_SYNC_MSG.to_string();
        };

        let kind_filter = kind.as_deref().unwrap_or("all");

        match client.list().await {
            Ok(all) => {
                let filtered: Vec<_> = all
                    .into_iter()
                    .filter(|s| match kind_filter {
                        "all" => true,
                        "skill" => s.kind != "command" && s.kind != "agent",
                        other => s.kind == other,
                    })
                    .collect();

                if filtered.is_empty() {
                    return format!(
                        "No {}s on gateway.",
                        if kind_filter == "all" {
                            "resource"
                        } else {
                            kind_filter
                        }
                    );
                }

                let mut text = format!(
                    "{:<30} {:<8} {:>10}  {:<14}\n",
                    "NAME", "KIND", "SIZE", "CHECKSUM"
                );
                text.push_str(&"-".repeat(70));
                text.push('\n');
                for s in &filtered {
                    text.push_str(&format!(
                        "{:<30} {:<8} {:>10}  {}\n",
                        s.name,
                        s.kind,
                        s.size,
                        &s.checksum[..std::cmp::min(12, s.checksum.len())]
                    ));
                }
                text
            }
            Err(e) => format!("Error listing resources: {e}"),
        }
    }

    /// Delete a resource from the gateway.
    #[tool(description = "Delete a skill, command, or agent from the gateway by name.")]
    async fn sync_delete(
        &self,
        Parameters(SyncDeleteParams { name }): Parameters<SyncDeleteParams>,
    ) -> String {
        let Some(ref client) = self.sync_client else {
            return NO_SYNC_MSG.to_string();
        };

        match client.delete(&name).await {
            Ok(()) => format!("Deleted '{}'.", name),
            Err(e) => format!("Error deleting '{}': {e}", name),
        }
    }

    /// Bidirectional sync of skills, commands, and agents.
    #[tool(
        description = "Bidirectional sync: push new/changed local skills, commands, and agents to the gateway; pull new remote ones locally. Skills are directories with SKILL.md. Commands are .md files in the sync directory. Agents are .md files in ~/.agentic/agents/."
    )]
    async fn sync_all(
        &self,
        Parameters(SyncAllParams { directory }): Parameters<SyncAllParams>,
    ) -> String {
        let Some(ref client) = self.sync_client else {
            return NO_SYNC_MSG.to_string();
        };

        let dir = directory
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let dir = match dir.canonicalize() {
            Ok(d) => d,
            Err(e) => return format!("Error: could not resolve directory: {e}"),
        };

        use sha2::{Digest, Sha256};
        use std::collections::{HashMap, HashSet};

        // Discover local skills
        let local_skills: Vec<(String, std::path::PathBuf)> = match std::fs::read_dir(&dir) {
            Ok(entries) => entries
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .filter(|e| e.path().join("SKILL.md").exists())
                .filter_map(|e| {
                    let name =
                        agent_comms::sanitize::sanitize_name(&e.file_name().to_string_lossy());
                    if name.is_empty() {
                        None
                    } else {
                        Some((name, e.path()))
                    }
                })
                .collect(),
            Err(e) => return format!("Error reading directory: {e}"),
        };

        let skill_names: HashSet<&str> = local_skills.iter().map(|(n, _)| n.as_str()).collect();

        // Discover local commands
        let local_commands: Vec<(String, std::path::PathBuf)> = match std::fs::read_dir(&dir) {
            Ok(entries) => entries
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                .filter(|e| {
                    e.path()
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .map(|ext| ext == "md")
                        .unwrap_or(false)
                })
                .filter(|e| {
                    !e.file_name()
                        .to_string_lossy()
                        .starts_with(|c: char| c.is_uppercase())
                })
                .filter_map(|e| {
                    let stem = e.path().file_stem()?.to_string_lossy().to_string();
                    let name = agent_comms::sanitize::sanitize_name(&stem);
                    if name.is_empty() || skill_names.contains(name.as_str()) {
                        None
                    } else {
                        Some((name, e.path()))
                    }
                })
                .collect(),
            Err(e) => return format!("Error reading directory: {e}"),
        };

        // Discover local agents
        let agents_dir = {
            let primary = agent_comms::config::home_dir()
                .join(".agentic")
                .join("agents");
            if primary.is_dir() {
                primary
            } else {
                let fallback = agent_comms::config::home_dir()
                    .join(".claude")
                    .join("agents");
                if fallback.is_dir() {
                    fallback
                } else {
                    primary
                }
            }
        };
        let cmd_names: HashSet<&str> = local_commands.iter().map(|(n, _)| n.as_str()).collect();
        let local_agents: Vec<(String, std::path::PathBuf)> = if agents_dir.is_dir() {
            match std::fs::read_dir(&agents_dir) {
                Ok(entries) => entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                    .filter(|e| {
                        e.path()
                            .extension()
                            .and_then(|ext| ext.to_str())
                            .map(|ext| ext == "md")
                            .unwrap_or(false)
                    })
                    .filter_map(|e| {
                        let stem = e.path().file_stem()?.to_string_lossy().to_string();
                        let name = agent_comms::sanitize::sanitize_name(&stem);
                        if name.is_empty()
                            || skill_names.contains(name.as_str())
                            || cmd_names.contains(name.as_str())
                        {
                            None
                        } else {
                            Some((name, e.path()))
                        }
                    })
                    .collect(),
                Err(_) => vec![],
            }
        } else {
            vec![]
        };

        let remote: HashMap<String, agent_sync::client::SyncMeta> = match client.list().await {
            Ok(list) => list.into_iter().map(|s| (s.name.clone(), s)).collect(),
            Err(e) => return format!("Error listing remote resources: {e}"),
        };

        let mut pushed = 0usize;
        let mut pulled = 0usize;
        let mut output = Vec::new();

        // Push skills
        for (name, path) in &local_skills {
            let local_checksum = match agent_sync::zip_util::zip_skill_dir(path) {
                Ok((_, c)) => c,
                Err(e) => {
                    output.push(format!("  error: skill '{}': {e}", name));
                    continue;
                }
            };
            let needs_push = match remote.get(name) {
                Some(r) => r.checksum != local_checksum,
                None => true,
            };
            if needs_push {
                match agent_sync::zip_util::zip_skill_dir(path) {
                    Ok((zip_bytes, checksum)) => {
                        let size = zip_bytes.len();
                        match client.upload(name, zip_bytes).await {
                            Ok(_) => {
                                output.push(format!(
                                    "  pushed skill '{}' ({} bytes, {})",
                                    name,
                                    size,
                                    &checksum[..12]
                                ));
                                pushed += 1;
                            }
                            Err(e) => {
                                output.push(format!("  error pushing skill '{}': {e}", name));
                            }
                        }
                    }
                    Err(e) => {
                        output.push(format!("  error zipping skill '{}': {e}", name));
                    }
                }
            }
        }

        // Push commands
        for (name, path) in &local_commands {
            let text = match std::fs::read_to_string(path) {
                Ok(s) => s,
                Err(e) => {
                    output.push(format!("  error reading command '{}': {e}", name));
                    continue;
                }
            };
            let local_checksum = {
                let mut hasher = Sha256::new();
                hasher.update(text.as_bytes());
                hex::encode(hasher.finalize())
            };
            let needs_push = match remote.get(name) {
                Some(r) => r.checksum != local_checksum,
                None => true,
            };
            if needs_push {
                let size = text.len();
                match client.upload_command(name, text).await {
                    Ok(_) => {
                        output.push(format!("  pushed command '{}' ({} bytes)", name, size));
                        pushed += 1;
                    }
                    Err(e) => {
                        output.push(format!("  error pushing command '{}': {e}", name));
                    }
                }
            }
        }

        // Push agents
        for (name, path) in &local_agents {
            let text = match std::fs::read_to_string(path) {
                Ok(s) => s,
                Err(e) => {
                    output.push(format!("  error reading agent '{}': {e}", name));
                    continue;
                }
            };
            let local_checksum = {
                let mut hasher = Sha256::new();
                hasher.update(text.as_bytes());
                hex::encode(hasher.finalize())
            };
            let needs_push = match remote.get(name) {
                Some(r) => r.checksum != local_checksum,
                None => true,
            };
            if needs_push {
                let size = text.len();
                match client.upload_agent(name, text).await {
                    Ok(_) => {
                        output.push(format!("  pushed agent '{}' ({} bytes)", name, size));
                        pushed += 1;
                    }
                    Err(e) => {
                        output.push(format!("  error pushing agent '{}': {e}", name));
                    }
                }
            }
        }

        // Pull remote entries not present locally
        let local_names: HashSet<&str> = local_skills
            .iter()
            .map(|(n, _)| n.as_str())
            .chain(local_commands.iter().map(|(n, _)| n.as_str()))
            .chain(local_agents.iter().map(|(n, _)| n.as_str()))
            .collect();
        for name in remote.keys() {
            if !local_names.contains(name.as_str()) {
                match client.download(name).await {
                    Ok(result) => match result.kind.as_str() {
                        "agent" => {
                            let _ = std::fs::create_dir_all(&agents_dir);
                            let dest = agents_dir.join(format!("{}.md", name));
                            match std::fs::write(&dest, &result.bytes) {
                                Ok(()) => {
                                    output.push(format!(
                                        "  pulled agent '{}' -> {}",
                                        name,
                                        dest.display()
                                    ));
                                    pulled += 1;
                                }
                                Err(e) => {
                                    output.push(format!("  error writing agent '{}': {e}", name));
                                }
                            }
                        }
                        "command" => {
                            let dest = dir.join(format!("{}.md", name));
                            match std::fs::write(&dest, &result.bytes) {
                                Ok(()) => {
                                    output.push(format!(
                                        "  pulled command '{}' -> {}",
                                        name,
                                        dest.display()
                                    ));
                                    pulled += 1;
                                }
                                Err(e) => {
                                    output.push(format!("  error writing command '{}': {e}", name));
                                }
                            }
                        }
                        _ => match agent_sync::zip_util::unzip_skill(name, &result.bytes, &dir) {
                            Ok(out) => {
                                output.push(format!(
                                    "  pulled skill '{}' -> {}",
                                    name,
                                    out.display()
                                ));
                                pulled += 1;
                            }
                            Err(e) => {
                                output.push(format!("  error extracting skill '{}': {e}", name));
                            }
                        },
                    },
                    Err(e) => {
                        output.push(format!("  error downloading '{}': {e}", name));
                    }
                }
            }
        }

        output.push(format!(
            "Sync complete: {} pushed, {} pulled.",
            pushed, pulled
        ));
        output.join("\n")
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
             communication with users via gateway channels, and skill/command/agent sync. \
             Code tools work immediately. Comms tools (set_identity, send_message, \
             get_messages, confirm_read, reply_to, taking_action_on) require gateway configuration. \
             Sync tools (sync_push, sync_pull, sync_list, sync_delete, sync_all) require gateway configuration."
                .to_string(),
        )
    }
}
