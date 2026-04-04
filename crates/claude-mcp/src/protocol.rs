use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    #[allow(dead_code)]
    pub jsonrpc: Option<String>,
    pub id: Value,
    pub method: String,
    pub params: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
}

impl JsonRpcResponse {
    pub fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Value, code: i32, message: String) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError { code, message }),
        }
    }
}

pub fn handle_initialize(id: Value) -> JsonRpcResponse {
    JsonRpcResponse::success(
        id,
        json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": "claude-tools",
                "version": env!("CLAUDE_TOOLS_VERSION")
            }
        }),
    )
}

pub fn handle_tools_list(id: Value) -> JsonRpcResponse {
    let tools = json!({
        "tools": [
            {
                "name": "tree",
                "description": "Token-efficient directory tree view. Respects .gitignore, configurable depth and max files per directory.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Directory to display (default: current directory)" },
                        "depth": { "type": "integer", "description": "Maximum depth (default: 3)", "default": 3 },
                        "max_files": { "type": "integer", "description": "Max files per directory before truncation (default: 20)", "default": 20 }
                    }
                }
            },
            {
                "name": "list",
                "description": "Smart directory listing. Directories first, then files alphabetically. Minimal output.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Directory to list (default: current directory)" },
                        "sizes": { "type": "boolean", "description": "Show file sizes", "default": false },
                        "show_hidden": { "type": "boolean", "description": "Show hidden files", "default": false }
                    }
                }
            },
            {
                "name": "file_ops",
                "description": "Cross-platform file operations: copy, move, mkdir, remove. Pure Rust, no shell commands.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "operation": { "type": "string", "enum": ["copy", "move", "mkdir", "remove"] },
                        "src": { "type": "string", "description": "Source path" },
                        "dst": { "type": "string", "description": "Destination path (for copy/move)" }
                    },
                    "required": ["operation", "src"]
                }
            },
            {
                "name": "extract_symbol",
                "description": "Extract a symbol's complete source code by name from a file. Returns the full definition with line numbers.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Symbol name to extract" },
                        "file": { "type": "string", "description": "File to search in. If not provided, searches the symbol index." }
                    },
                    "required": ["name"]
                }
            },
            {
                "name": "list_symbols",
                "description": "List all symbols (functions, classes, structs, etc.) in a file with their types and line ranges.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "file": { "type": "string", "description": "File to list symbols from" }
                    },
                    "required": ["file"]
                }
            },
            {
                "name": "search_symbols",
                "description": "Search the project-wide symbol index by name, type, or file pattern.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Symbol name search query" },
                        "kind": { "type": "string", "description": "Symbol type filter (fn, class, struct, enum, trait, etc.)" },
                        "file_pattern": { "type": "string", "description": "File path pattern filter" },
                        "limit": { "type": "integer", "description": "Max results (default: 20)", "default": 20 }
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "build_index",
                "description": "Build or incrementally update the project file and symbol index.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Directory to index (default: current directory)" },
                        "rebuild": { "type": "boolean", "description": "Force full rebuild", "default": false }
                    }
                }
            },
            {
                "name": "find_files",
                "description": "Search the file index by name pattern, extension, or size range.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string", "description": "File name pattern" },
                        "extension": { "type": "string", "description": "File extension filter" },
                        "min_size": { "type": "integer", "description": "Minimum file size in bytes" },
                        "max_size": { "type": "integer", "description": "Maximum file size in bytes" },
                        "limit": { "type": "integer", "description": "Max results (default: 20)", "default": 20 }
                    }
                }
            },
            {
                "name": "project_summary",
                "description": "Generate a compact project overview with language breakdown, file counts, and key files.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Directory to summarize (default: current directory)" }
                    }
                }
            }
        ]
    });

    JsonRpcResponse::success(id, tools)
}
