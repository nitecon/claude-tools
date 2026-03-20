use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::path::PathBuf;

pub async fn dispatch(tool_name: &str, args: Value) -> Result<Value> {
    match tool_name {
        "tree" => tool_tree(args),
        "list" => tool_list(args),
        "file_ops" => tool_file_ops(args),
        "extract_symbol" => tool_extract_symbol(args),
        "list_symbols" => tool_list_symbols(args),
        "search_symbols" => tool_search_symbols(args),
        "build_index" => tool_build_index(args),
        "find_files" => tool_find_files(args),
        "project_summary" => tool_project_summary(args),
        _ => Err(anyhow!("Unknown tool: {tool_name}")),
    }
}

fn tool_tree(args: Value) -> Result<Value> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    let depth = args.get("depth").and_then(|v| v.as_u64()).unwrap_or(3) as usize;
    let max_files = args.get("max_files").and_then(|v| v.as_u64()).unwrap_or(20) as usize;

    let options = claude_fs::tree::TreeOptions {
        max_depth: depth,
        max_files_per_dir: max_files,
    };

    let tree = claude_fs::tree::tree(&path, &options)?;
    let text = claude_fs::tree::render_tree_text(&tree, 0);

    Ok(mcp_text_content(&text))
}

fn tool_list(args: Value) -> Result<Value> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    let sizes = args.get("sizes").and_then(|v| v.as_bool()).unwrap_or(false);
    let show_hidden = args
        .get("show_hidden")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let options = claude_fs::list::ListOptions {
        show_sizes: sizes,
        show_hidden,
    };

    let entries = claude_fs::list::list_dir(&path, &options)?;
    let text = claude_fs::list::render_list_text(&entries);

    Ok(mcp_text_content(&text))
}

fn tool_file_ops(args: Value) -> Result<Value> {
    let operation = args
        .get("operation")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing 'operation' parameter"))?;

    let src = args
        .get("src")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing 'src' parameter"))?;

    let src_path = PathBuf::from(src);

    match operation {
        "copy" => {
            let dst = args
                .get("dst")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("Missing 'dst' parameter for copy"))?;
            claude_fs::ops::copy(&src_path, &PathBuf::from(dst))?;
            Ok(mcp_text_content(&format!("Copied {src} -> {dst}")))
        }
        "move" => {
            let dst = args
                .get("dst")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("Missing 'dst' parameter for move"))?;
            claude_fs::ops::move_path(&src_path, &PathBuf::from(dst))?;
            Ok(mcp_text_content(&format!("Moved {src} -> {dst}")))
        }
        "mkdir" => {
            claude_fs::ops::mkdir(&src_path)?;
            Ok(mcp_text_content(&format!("Created {src}")))
        }
        "remove" => {
            claude_fs::ops::remove(&src_path)?;
            Ok(mcp_text_content(&format!("Removed {src}")))
        }
        _ => Err(anyhow!("Unknown operation: {operation}")),
    }
}

fn tool_extract_symbol(args: Value) -> Result<Value> {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing 'name' parameter"))?;

    let mut parser = claude_symbols::SymbolParser::new();

    if let Some(file) = args.get("file").and_then(|v| v.as_str()) {
        let path = PathBuf::from(file);
        match parser.extract_symbol(&path, name)? {
            Some(source) => Ok(mcp_text_content(&format!("{source}"))),
            None => Ok(mcp_text_content(&format!(
                "Symbol '{name}' not found in {file}"
            ))),
        }
    } else {
        let root = std::env::current_dir()?;
        let index = claude_symbols::SymbolIndex::open_for_project(&root)?;
        let results = index.search(name, None, None, 5)?;

        if results.is_empty() {
            return Ok(mcp_text_content(&format!(
                "Symbol '{name}' not found in index"
            )));
        }

        let first = &results[0];
        match parser.extract_symbol(&first.file, name)? {
            Some(source) => Ok(mcp_text_content(&format!("{source}"))),
            None => {
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
                Ok(mcp_text_content(&text))
            }
        }
    }
}

fn tool_list_symbols(args: Value) -> Result<Value> {
    let file = args
        .get("file")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing 'file' parameter"))?;

    let path = PathBuf::from(file);
    let mut parser = claude_symbols::SymbolParser::new();
    let symbols = parser.parse_file(&path)?;

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
        text = format!("No symbols found in {file}");
    }

    Ok(mcp_text_content(&text))
}

fn tool_search_symbols(args: Value) -> Result<Value> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing 'query' parameter"))?;

    let kind = args.get("kind").and_then(|v| v.as_str());
    let file_pattern = args.get("file_pattern").and_then(|v| v.as_str());
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;

    let root = std::env::current_dir()?;
    let index = claude_symbols::SymbolIndex::open_for_project(&root)?;
    let results = index.search(query, kind, file_pattern, limit)?;

    if results.is_empty() {
        return Ok(mcp_text_content(&format!(
            "No symbols found matching '{query}'"
        )));
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

    Ok(mcp_text_content(&text))
}

fn tool_build_index(args: Value) -> Result<Value> {
    let root = args
        .get("path")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let rebuild = args
        .get("rebuild")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if rebuild {
        let cache_dir = root.join(".claude-tools");
        if cache_dir.exists() {
            std::fs::remove_dir_all(&cache_dir)?;
        }
    }

    let file_indexer = claude_search::indexer::FileIndexer::open_for_project(&root)?;
    let file_stats = file_indexer.build(&root, true)?;

    let symbol_index = claude_symbols::SymbolIndex::open_for_project(&root)?;
    let symbol_stats = symbol_index.build(&root)?;

    let (file_count, symbol_count) = symbol_index.stats()?;

    let text = format!(
        "Files: {file_stats}\nSymbols: {symbol_stats}\nTotal: {file_count} files, {symbol_count} symbols"
    );

    Ok(mcp_text_content(&text))
}

fn tool_find_files(args: Value) -> Result<Value> {
    let pattern = args.get("pattern").and_then(|v| v.as_str());
    let extension = args.get("extension").and_then(|v| v.as_str());
    let min_size = args.get("min_size").and_then(|v| v.as_u64());
    let max_size = args.get("max_size").and_then(|v| v.as_u64());
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;

    let root = std::env::current_dir()?;
    let indexer = claude_search::indexer::FileIndexer::open_for_project(&root)?;

    let results =
        claude_search::query::find_files(&indexer, pattern, extension, min_size, max_size, limit)?;

    if results.is_empty() {
        return Ok(mcp_text_content("No files found"));
    }

    let mut text = String::new();
    for r in &results {
        text.push_str(&format!("{}\n", r.path));
    }

    Ok(mcp_text_content(&text))
}

fn tool_project_summary(args: Value) -> Result<Value> {
    let root = args
        .get("path")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let indexer = claude_search::indexer::FileIndexer::open_for_project(&root)?;
    if indexer.file_count()? == 0 {
        indexer.build(&root, false)?;
    }

    let summary = claude_search::query::project_summary(&indexer)?;
    let text = claude_search::query::render_summary_text(&summary);

    Ok(mcp_text_content(&text))
}

/// Format a text result as MCP content array.
fn mcp_text_content(text: &str) -> Value {
    json!({
        "content": [
            {
                "type": "text",
                "text": text
            }
        ]
    })
}
