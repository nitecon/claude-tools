use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

mod updater;

#[derive(Parser)]
#[command(
    name = "agent-tools",
    about = "Token-efficient tools for AI coding agents"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Token-efficient directory tree view
    Tree {
        /// Directory to display (default: current directory)
        path: Option<PathBuf>,
        /// Maximum depth (default: 3)
        #[arg(short, long, default_value = "3")]
        depth: usize,
        /// Maximum files per directory before truncation (default: 20)
        #[arg(short, long, default_value = "20")]
        max_files: usize,
    },

    /// Smart directory listing
    List {
        /// Directory to list (default: current directory)
        path: Option<PathBuf>,
        /// Show file sizes
        #[arg(short, long)]
        sizes: bool,
        /// Show hidden files
        #[arg(short = 'a', long)]
        all: bool,
    },

    /// Extract a symbol's source code by name
    Symbol {
        /// Symbol name to extract
        name: String,
        /// File to search in (if not specified, searches index)
        #[arg(short, long)]
        file: Option<PathBuf>,
        /// Symbol type filter (function, class, struct, etc.)
        #[arg(short = 't', long = "type")]
        kind: Option<String>,
    },

    /// List all symbols in a file
    Symbols {
        /// File to list symbols from
        file: PathBuf,
        /// Symbol type filter
        #[arg(short = 't', long = "type")]
        kind: Option<String>,
    },

    /// Search the project-wide symbol index
    Search {
        /// Search query
        query: String,
        /// Search type: "symbol" or "file"
        #[arg(short = 't', long = "type", default_value = "symbol")]
        search_type: String,
        /// File pattern filter
        #[arg(short, long)]
        file: Option<String>,
        /// Maximum results (default: 20)
        #[arg(short, long, default_value = "20")]
        limit: usize,
    },

    /// Build or update the project index
    Index {
        /// Directory to index (default: current directory)
        path: Option<PathBuf>,
        /// Force rebuild (ignore cached data)
        #[arg(long)]
        rebuild: bool,
    },

    /// Show a compact project summary
    Summary {
        /// Directory to summarize (default: current directory)
        path: Option<PathBuf>,
    },

    /// Copy a file or directory
    Cp {
        /// Source path
        src: PathBuf,
        /// Destination path
        dst: PathBuf,
    },

    /// Move a file or directory
    Mv {
        /// Source path
        src: PathBuf,
        /// Destination path
        dst: PathBuf,
    },

    /// Create directories recursively
    Mkdir {
        /// Directory path to create
        path: PathBuf,
    },

    /// Remove a file or directory
    Rm {
        /// Path to remove
        path: PathBuf,
    },

    /// Start MCP stdio server
    Serve,

    /// Check for updates and install the latest version
    Update,

    /// Print version information
    Version,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Auto-update check on every invocation (rate-limited, non-blocking for most calls)
    // Skip for update/version commands to avoid double-checking
    if !matches!(cli.command, Commands::Update | Commands::Version) {
        updater::auto_update();
    }

    match cli.command {
        Commands::Tree {
            path,
            depth,
            max_files,
        } => cmd_tree(path, depth, max_files),

        Commands::List { path, sizes, all } => cmd_list(path, sizes, all),

        Commands::Symbol { name, file, kind } => cmd_symbol(&name, file, kind),

        Commands::Symbols { file, kind } => cmd_symbols(&file, kind),

        Commands::Search {
            query,
            search_type,
            file,
            limit,
        } => cmd_search(&query, &search_type, file, limit),

        Commands::Index { path, rebuild } => cmd_index(path, rebuild),

        Commands::Summary { path } => cmd_summary(path),

        Commands::Cp { src, dst } => {
            agent_fs::ops::copy(&src, &dst)?;
            println!("Copied {} -> {}", src.display(), dst.display());
            Ok(())
        }

        Commands::Mv { src, dst } => {
            agent_fs::ops::move_path(&src, &dst)?;
            println!("Moved {} -> {}", src.display(), dst.display());
            Ok(())
        }

        Commands::Mkdir { path } => {
            agent_fs::ops::mkdir(&path)?;
            println!("Created {}", path.display());
            Ok(())
        }

        Commands::Rm { path } => {
            agent_fs::ops::remove(&path)?;
            println!("Removed {}", path.display());
            Ok(())
        }

        Commands::Serve => {
            eprintln!("Use `agent-tools-mcp` binary for MCP server");
            std::process::exit(1);
        }

        Commands::Update => updater::manual_update(),

        Commands::Version => {
            println!("agent-tools {}", env!("AGENT_TOOLS_VERSION"));
            Ok(())
        }
    }
}

fn cmd_tree(path: Option<PathBuf>, depth: usize, max_files: usize) -> Result<()> {
    let path = path.unwrap_or_else(|| PathBuf::from("."));
    let options = agent_fs::tree::TreeOptions {
        max_depth: depth,
        max_files_per_dir: max_files,
    };
    let tree = agent_fs::tree::tree(&path, &options)?;
    print!("{}", agent_fs::tree::render_tree_text(&tree, 0));
    Ok(())
}

fn cmd_list(path: Option<PathBuf>, sizes: bool, all: bool) -> Result<()> {
    let path = path.unwrap_or_else(|| PathBuf::from("."));
    let options = agent_fs::list::ListOptions {
        show_sizes: sizes,
        show_hidden: all,
    };
    let entries = agent_fs::list::list_dir(&path, &options)?;
    print!("{}", agent_fs::list::render_list_text(&entries));
    Ok(())
}

fn cmd_symbol(name: &str, file: Option<PathBuf>, kind: Option<String>) -> Result<()> {
    if let Some(file_path) = file {
        // Direct file extraction
        let mut parser = agent_symbols::SymbolParser::new();
        match parser.extract_symbol(&file_path, name)? {
            Some(source) => {
                println!("{source}");
            }
            None => {
                eprintln!("Symbol '{name}' not found in {}", file_path.display());
                std::process::exit(1);
            }
        }
    } else {
        // Search index
        let root = std::env::current_dir()?;
        let index = agent_symbols::SymbolIndex::open_for_project(&root)?;
        let results = index.search(name, kind.as_deref(), None, 10)?;

        if results.is_empty() {
            eprintln!("Symbol '{name}' not found in index. Run `agent-tools index` first.");
            std::process::exit(1);
        }

        // Extract source from the first match
        let first = &results[0];
        let mut parser = agent_symbols::SymbolParser::new();
        match parser.extract_symbol(&first.file, name)? {
            Some(source) => println!("{source}"),
            None => {
                // Fallback: just show location
                for r in &results {
                    println!(
                        "{} {} {}:{}-{}",
                        r.kind,
                        r.name,
                        r.file.display(),
                        r.start_line,
                        r.end_line
                    );
                }
            }
        }
    }
    Ok(())
}

fn cmd_symbols(file: &Path, kind: Option<String>) -> Result<()> {
    let mut parser = agent_symbols::SymbolParser::new();
    let symbols = parser.parse_file(file)?;

    for s in &symbols {
        if let Some(ref k) = kind {
            let kind_str = format!("{}", s.kind);
            if kind_str != *k {
                continue;
            }
        }
        let parent_info = s
            .parent
            .as_ref()
            .map(|p| format!(" (in {p})"))
            .unwrap_or_default();
        println!(
            "{:<10} {:<30} {}:{}-{}{}",
            format!("{}", s.kind),
            s.name,
            s.file.display(),
            s.start_line,
            s.end_line,
            parent_info
        );
    }
    Ok(())
}

fn cmd_search(query: &str, search_type: &str, file: Option<String>, limit: usize) -> Result<()> {
    let root = std::env::current_dir()?;

    match search_type {
        "symbol" => {
            let index = agent_symbols::SymbolIndex::open_for_project(&root)?;
            let results = index.search(query, None, file.as_deref(), limit)?;

            if results.is_empty() {
                eprintln!("No symbols found matching '{query}'");
                return Ok(());
            }

            for r in &results {
                println!(
                    "{:<10} {:<30} {}:{}-{}",
                    format!("{}", r.kind),
                    r.name,
                    r.file.display(),
                    r.start_line,
                    r.end_line
                );
            }
        }
        "file" => {
            let indexer = agent_search::indexer::FileIndexer::open_for_project(&root)?;
            let results =
                agent_search::query::find_files(&indexer, Some(query), None, None, None, limit)?;

            if results.is_empty() {
                eprintln!("No files found matching '{query}'");
                return Ok(());
            }

            for r in &results {
                println!("{}", r.path);
            }
        }
        _ => {
            eprintln!("Unknown search type: {search_type}. Use 'symbol' or 'file'.");
            std::process::exit(1);
        }
    }
    Ok(())
}

fn cmd_index(path: Option<PathBuf>, rebuild: bool) -> Result<()> {
    let root =
        path.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    if rebuild {
        let data_dir = agent_core::project_data_dir(&root);
        if data_dir.exists() {
            std::fs::remove_dir_all(&data_dir)?;
            println!("Cleared existing index at {}", data_dir.display());
        }
    }

    // Build file index
    print!("Indexing files... ");
    let file_indexer = agent_search::indexer::FileIndexer::open_for_project(&root)?;
    let file_stats = file_indexer.build(&root, true)?;
    println!("{file_stats}");

    // Build symbol index
    print!("Indexing symbols... ");
    let symbol_index = agent_symbols::SymbolIndex::open_for_project(&root)?;
    let symbol_stats = symbol_index.build(&root)?;
    println!("{symbol_stats}");

    let (file_count, symbol_count) = symbol_index.stats()?;
    println!("\nTotal: {file_count} files, {symbol_count} symbols");

    Ok(())
}

fn cmd_summary(path: Option<PathBuf>) -> Result<()> {
    let root =
        path.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // Ensure index exists
    let indexer = agent_search::indexer::FileIndexer::open_for_project(&root)?;
    if indexer.file_count()? == 0 {
        println!("No index found. Building...");
        indexer.build(&root, false)?;
    }

    let summary = agent_search::query::project_summary(&indexer)?;
    print!("{}", agent_search::query::render_summary_text(&summary));
    Ok(())
}
