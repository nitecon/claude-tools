use anyhow::{Context, Result};
use ignore::WalkBuilder;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::Path;
use std::time::SystemTime;

/// File indexer that maintains a SQLite-backed file index with change detection.
pub struct FileIndexer {
    conn: Connection,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileRecord {
    pub path: String,
    pub extension: String,
    pub size: u64,
    pub mtime_secs: i64,
    pub content_hash: Option<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct IndexStats {
    pub files_seen: usize,
    pub files_indexed: usize,
    pub files_skipped: usize,
    pub files_errored: usize,
}

impl std::fmt::Display for IndexStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Indexed {} files, skipped {} unchanged, {} errors ({} total)",
            self.files_indexed, self.files_skipped, self.files_errored, self.files_seen
        )
    }
}

impl FileIndexer {
    pub fn open(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(db_path)
            .with_context(|| format!("Failed to open file index at {}", db_path.display()))?;

        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;

            CREATE TABLE IF NOT EXISTS files (
                id INTEGER PRIMARY KEY,
                path TEXT UNIQUE NOT NULL,
                extension TEXT NOT NULL DEFAULT '',
                size INTEGER NOT NULL DEFAULT 0,
                mtime_secs INTEGER NOT NULL,
                content_hash TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_files_path ON files(path);
            CREATE INDEX IF NOT EXISTS idx_files_ext ON files(extension);
            CREATE INDEX IF NOT EXISTS idx_files_size ON files(size);
            CREATE INDEX IF NOT EXISTS idx_files_mtime ON files(mtime_secs);
            ",
        )?;

        Ok(Self { conn })
    }

    /// Open the file index in the `.claude-tools` directory of the given project root.
    pub fn open_for_project(project_root: &Path) -> Result<Self> {
        let db_path = project_root.join(".claude-tools").join("files.db");
        Self::open(&db_path)
    }

    /// Build or incrementally update the file index.
    pub fn build(&self, root: &Path, compute_hashes: bool) -> Result<IndexStats> {
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

            stats.files_seen += 1;

            let metadata = match std::fs::metadata(path) {
                Ok(m) => m,
                Err(_) => {
                    stats.files_errored += 1;
                    continue;
                }
            };

            let mtime = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let duration = mtime
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default();
            let mtime_secs = duration.as_secs() as i64;
            let size = metadata.len();

            let path_str = path.to_string_lossy();
            let extension = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_string();

            // Check if already indexed and unchanged
            let needs_update: bool = {
                let mut stmt = self
                    .conn
                    .prepare_cached("SELECT mtime_secs, size FROM files WHERE path = ?1")?;
                match stmt.query_row(params![path_str.as_ref()], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
                }) {
                    Ok((s, sz)) => s != mtime_secs || sz != size as i64,
                    Err(_) => true,
                }
            };

            if !needs_update {
                stats.files_skipped += 1;
                continue;
            }

            let content_hash = if compute_hashes {
                match std::fs::read(path) {
                    Ok(contents) => Some(blake3::hash(&contents).to_hex().to_string()),
                    Err(_) => None,
                }
            } else {
                None
            };

            self.conn.execute(
                "INSERT INTO files (path, extension, size, mtime_secs, content_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(path) DO UPDATE SET
                   extension = ?2, size = ?3, mtime_secs = ?4, content_hash = ?5",
                params![
                    path_str.as_ref(),
                    extension,
                    size as i64,
                    mtime_secs,
                    content_hash,
                ],
            )?;

            stats.files_indexed += 1;
        }

        Ok(stats)
    }

    /// Get the underlying connection for queries.
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Get total file count.
    pub fn file_count(&self) -> Result<usize> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))?;
        Ok(count as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_build_index() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        std::fs::write(root.join("file.rs"), "fn main() {}").unwrap();
        std::fs::write(root.join("README.md"), "# Hello").unwrap();
        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::write(root.join("src/lib.rs"), "pub fn lib() {}").unwrap();

        let db_path = root.join(".claude-tools/files.db");
        let indexer = FileIndexer::open(&db_path).unwrap();
        let stats = indexer.build(root, false).unwrap();

        assert_eq!(stats.files_seen, 3);
        assert_eq!(stats.files_indexed, 3);
    }

    #[test]
    fn test_incremental() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        std::fs::write(root.join("file.txt"), "hello").unwrap();

        let db_path = root.join(".claude-tools/files.db");
        let indexer = FileIndexer::open(&db_path).unwrap();

        let stats1 = indexer.build(root, false).unwrap();
        assert_eq!(stats1.files_indexed, 1);

        let stats2 = indexer.build(root, false).unwrap();
        assert_eq!(stats2.files_indexed, 0);
        assert_eq!(stats2.files_skipped, 1);
    }
}
