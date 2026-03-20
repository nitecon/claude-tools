use crate::extractor::{Symbol, SymbolKind};
use crate::languages::Language;
use crate::parser::SymbolParser;
use anyhow::{Context, Result};
use ignore::WalkBuilder;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Persistent symbol index backed by SQLite.
pub struct SymbolIndex {
    conn: Connection,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolMatch {
    pub name: String,
    pub kind: SymbolKind,
    pub file: PathBuf,
    pub start_line: usize,
    pub end_line: usize,
    pub language: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

impl From<&Symbol> for SymbolMatch {
    fn from(s: &Symbol) -> Self {
        Self {
            name: s.name.clone(),
            kind: s.kind,
            file: s.file.clone(),
            start_line: s.start_line,
            end_line: s.end_line,
            language: s.language.to_string(),
            parent: s.parent.clone(),
        }
    }
}

impl SymbolIndex {
    /// Open or create a symbol index at the given path.
    pub fn open(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(db_path)
            .with_context(|| format!("Failed to open index at {}", db_path.display()))?;

        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;

            CREATE TABLE IF NOT EXISTS files (
                id INTEGER PRIMARY KEY,
                path TEXT UNIQUE NOT NULL,
                mtime_secs INTEGER NOT NULL,
                mtime_nanos INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS symbols (
                id INTEGER PRIMARY KEY,
                file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                start_line INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                language TEXT NOT NULL,
                parent TEXT,
                UNIQUE(file_id, name, start_line)
            );

            CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);
            CREATE INDEX IF NOT EXISTS idx_symbols_kind ON symbols(kind);
            CREATE INDEX IF NOT EXISTS idx_symbols_file ON symbols(file_id);
            ",
        )?;

        Ok(Self { conn })
    }

    /// Open or create a symbol index in the `.claude-tools` directory of the given project root.
    pub fn open_for_project(project_root: &Path) -> Result<Self> {
        let db_path = project_root.join(".claude-tools").join("symbols.db");
        Self::open(&db_path)
    }

    /// Build or incrementally update the index for all supported files under root.
    pub fn build(&self, root: &Path) -> Result<IndexStats> {
        let mut parser = SymbolParser::new();
        let mut stats = IndexStats::default();

        let walker = WalkBuilder::new(root)
            .hidden(true)
            .git_ignore(true)
            .git_global(false)
            .git_exclude(true)
            .build();

        for entry in walker {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            // Skip unsupported languages
            if Language::from_path(path).is_err() {
                continue;
            }

            stats.files_seen += 1;

            // Check if file needs re-indexing
            let metadata = match std::fs::metadata(path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let mtime = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let duration = mtime
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default();
            let mtime_secs = duration.as_secs() as i64;
            let mtime_nanos = duration.subsec_nanos() as i64;

            let path_str = path.to_string_lossy();

            // Check existing mtime
            let needs_update: bool = {
                let mut stmt = self
                    .conn
                    .prepare_cached("SELECT mtime_secs, mtime_nanos FROM files WHERE path = ?1")?;
                match stmt.query_row(params![path_str.as_ref()], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
                }) {
                    Ok((s, n)) => s != mtime_secs || n != mtime_nanos,
                    Err(_) => true, // new file
                }
            };

            if !needs_update {
                stats.files_skipped += 1;
                continue;
            }

            // Parse and index
            match parser.parse_file(path) {
                Ok(symbols) => {
                    self.index_file(path, &symbols, mtime_secs, mtime_nanos)?;
                    stats.files_indexed += 1;
                    stats.symbols_indexed += symbols.len();
                }
                Err(_) => {
                    stats.files_errored += 1;
                }
            }
        }

        Ok(stats)
    }

    fn index_file(
        &self,
        path: &Path,
        symbols: &[Symbol],
        mtime_secs: i64,
        mtime_nanos: i64,
    ) -> Result<()> {
        let path_str = path.to_string_lossy();

        // Upsert file record
        self.conn.execute(
            "INSERT INTO files (path, mtime_secs, mtime_nanos) VALUES (?1, ?2, ?3)
             ON CONFLICT(path) DO UPDATE SET mtime_secs = ?2, mtime_nanos = ?3",
            params![path_str.as_ref(), mtime_secs, mtime_nanos],
        )?;

        let file_id: i64 = self.conn.query_row(
            "SELECT id FROM files WHERE path = ?1",
            params![path_str.as_ref()],
            |row| row.get(0),
        )?;

        // Clear old symbols for this file
        self.conn
            .execute("DELETE FROM symbols WHERE file_id = ?1", params![file_id])?;

        // Insert new symbols
        let mut stmt = self.conn.prepare_cached(
            "INSERT OR IGNORE INTO symbols (file_id, name, kind, start_line, end_line, language, parent)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;

        for symbol in symbols {
            stmt.execute(params![
                file_id,
                symbol.name,
                format!("{}", symbol.kind),
                symbol.start_line as i64,
                symbol.end_line as i64,
                format!("{}", symbol.language),
                symbol.parent,
            ])?;
        }

        Ok(())
    }

    /// Search symbols by name (exact, prefix, or contains).
    pub fn search(
        &self,
        query: &str,
        kind_filter: Option<&str>,
        file_pattern: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SymbolMatch>> {
        let mut sql = String::from(
            "SELECT s.name, s.kind, f.path, s.start_line, s.end_line, s.language, s.parent
             FROM symbols s
             JOIN files f ON s.file_id = f.id
             WHERE s.name LIKE ?1",
        );

        let name_pattern = format!("%{query}%");
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(name_pattern)];

        if let Some(kind) = kind_filter {
            sql.push_str(" AND s.kind = ?");
            sql.push_str(&(param_values.len() + 1).to_string());
            param_values.push(Box::new(kind.to_string()));
        }

        if let Some(pattern) = file_pattern {
            sql.push_str(" AND f.path LIKE ?");
            sql.push_str(&(param_values.len() + 1).to_string());
            let file_like = format!("%{pattern}%");
            param_values.push(Box::new(file_like));
        }

        // Prioritize exact matches, then prefix, then contains
        sql.push_str(&format!(
            " ORDER BY
              CASE WHEN s.name = ?{} THEN 0
                   WHEN s.name LIKE ?{} THEN 1
                   ELSE 2
              END,
              s.name
             LIMIT ?{}",
            param_values.len() + 1,
            param_values.len() + 2,
            param_values.len() + 3,
        ));

        param_values.push(Box::new(query.to_string()));
        param_values.push(Box::new(format!("{query}%")));
        param_values.push(Box::new(limit as i64));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();

        let mut stmt = self.conn.prepare(&sql)?;
        let results = stmt.query_map(param_refs.as_slice(), |row| {
            Ok(SymbolMatch {
                name: row.get(0)?,
                kind: parse_symbol_kind(&row.get::<_, String>(1)?),
                file: PathBuf::from(row.get::<_, String>(2)?),
                start_line: row.get::<_, i64>(3)? as usize,
                end_line: row.get::<_, i64>(4)? as usize,
                language: row.get(5)?,
                parent: row.get(6)?,
            })
        })?;

        let mut matches = Vec::new();
        for result in results {
            matches.push(result?);
        }

        Ok(matches)
    }

    /// Get all symbols in a specific file.
    pub fn symbols_in_file(&self, path: &Path) -> Result<Vec<SymbolMatch>> {
        let path_str = path.to_string_lossy();

        let mut stmt = self.conn.prepare(
            "SELECT s.name, s.kind, f.path, s.start_line, s.end_line, s.language, s.parent
             FROM symbols s
             JOIN files f ON s.file_id = f.id
             WHERE f.path LIKE ?1
             ORDER BY s.start_line",
        )?;

        let pattern = format!("%{}", path_str);
        let results = stmt.query_map(params![pattern], |row| {
            Ok(SymbolMatch {
                name: row.get(0)?,
                kind: parse_symbol_kind(&row.get::<_, String>(1)?),
                file: PathBuf::from(row.get::<_, String>(2)?),
                start_line: row.get::<_, i64>(3)? as usize,
                end_line: row.get::<_, i64>(4)? as usize,
                language: row.get(5)?,
                parent: row.get(6)?,
            })
        })?;

        let mut matches = Vec::new();
        for result in results {
            matches.push(result?);
        }

        Ok(matches)
    }

    /// Get total counts.
    pub fn stats(&self) -> Result<(usize, usize)> {
        let file_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))?;
        let symbol_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM symbols", [], |row| row.get(0))?;
        Ok((file_count as usize, symbol_count as usize))
    }
}

fn parse_symbol_kind(s: &str) -> SymbolKind {
    match s {
        "fn" => SymbolKind::Function,
        "method" => SymbolKind::Method,
        "class" => SymbolKind::Class,
        "struct" => SymbolKind::Struct,
        "enum" => SymbolKind::Enum,
        "trait" => SymbolKind::Trait,
        "interface" => SymbolKind::Interface,
        "impl" => SymbolKind::Impl,
        "mod" => SymbolKind::Module,
        "namespace" => SymbolKind::Namespace,
        "macro" => SymbolKind::Macro,
        "type" => SymbolKind::Type,
        "const" => SymbolKind::Constant,
        "var" => SymbolKind::Variable,
        "prop" => SymbolKind::Property,
        _ => SymbolKind::Variable,
    }
}

#[derive(Debug, Default, Serialize)]
pub struct IndexStats {
    pub files_seen: usize,
    pub files_indexed: usize,
    pub files_skipped: usize,
    pub files_errored: usize,
    pub symbols_indexed: usize,
}

impl std::fmt::Display for IndexStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Indexed {} files ({} symbols), skipped {} unchanged, {} errors",
            self.files_indexed, self.symbols_indexed, self.files_skipped, self.files_errored
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_project(dir: &Path) {
        std::fs::write(
            dir.join("main.rs"),
            r#"
fn main() {
    println!("hello");
}

struct Config {
    name: String,
    value: i32,
}

impl Config {
    fn new(name: &str) -> Self {
        Config { name: name.to_string(), value: 0 }
    }
}
"#,
        )
        .unwrap();

        std::fs::write(
            dir.join("helper.py"),
            r#"
def process_data(items):
    return [x * 2 for x in items]

class DataProcessor:
    def __init__(self):
        self.data = []

    def run(self):
        pass
"#,
        )
        .unwrap();
    }

    #[test]
    fn test_build_and_search() {
        let dir = TempDir::new().unwrap();
        create_test_project(dir.path());

        let db_path = dir.path().join(".claude-tools/symbols.db");
        let index = SymbolIndex::open(&db_path).unwrap();

        let stats = index.build(dir.path()).unwrap();
        assert!(stats.files_indexed >= 2);
        assert!(stats.symbols_indexed >= 4);

        // Search for 'main'
        let results = index.search("main", None, None, 10).unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].name, "main");

        // Search by kind
        let results = index.search("Config", Some("struct"), None, 10).unwrap();
        assert!(!results.is_empty());
    }

    #[test]
    fn test_incremental_update() {
        let dir = TempDir::new().unwrap();
        create_test_project(dir.path());

        let db_path = dir.path().join(".claude-tools/symbols.db");
        let index = SymbolIndex::open(&db_path).unwrap();

        // First build
        let stats1 = index.build(dir.path()).unwrap();
        assert!(stats1.files_indexed >= 2);

        // Second build without changes: should skip
        let stats2 = index.build(dir.path()).unwrap();
        assert_eq!(stats2.files_indexed, 0);
        assert!(stats2.files_skipped >= 2);
    }
}
