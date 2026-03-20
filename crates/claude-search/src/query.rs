use crate::indexer::FileIndexer;
use anyhow::Result;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct FileMatch {
    pub path: String,
    pub extension: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectSummary {
    pub total_files: usize,
    pub total_size: u64,
    pub languages: Vec<LanguageBreakdown>,
    pub key_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LanguageBreakdown {
    pub extension: String,
    pub file_count: usize,
    pub total_size: u64,
}

/// Find files by name pattern, extension, or size range.
pub fn find_files(
    indexer: &FileIndexer,
    name_pattern: Option<&str>,
    extension: Option<&str>,
    min_size: Option<u64>,
    max_size: Option<u64>,
    limit: usize,
) -> Result<Vec<FileMatch>> {
    let conn = indexer.connection();

    let mut sql = String::from("SELECT path, extension, size FROM files WHERE 1=1");
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(pattern) = name_pattern {
        sql.push_str(&format!(" AND path LIKE ?{}", param_values.len() + 1));
        param_values.push(Box::new(format!("%{pattern}%")));
    }

    if let Some(ext) = extension {
        sql.push_str(&format!(" AND extension = ?{}", param_values.len() + 1));
        param_values.push(Box::new(ext.to_string()));
    }

    if let Some(min) = min_size {
        sql.push_str(&format!(" AND size >= ?{}", param_values.len() + 1));
        param_values.push(Box::new(min as i64));
    }

    if let Some(max) = max_size {
        sql.push_str(&format!(" AND size <= ?{}", param_values.len() + 1));
        param_values.push(Box::new(max as i64));
    }

    sql.push_str(&format!(
        " ORDER BY mtime_secs DESC LIMIT ?{}",
        param_values.len() + 1
    ));
    param_values.push(Box::new(limit as i64));

    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(|p| p.as_ref()).collect();

    let mut stmt = conn.prepare(&sql)?;
    let results = stmt.query_map(param_refs.as_slice(), |row| {
        Ok(FileMatch {
            path: row.get(0)?,
            extension: row.get(1)?,
            size: row.get::<_, i64>(2)? as u64,
        })
    })?;

    let mut matches = Vec::new();
    for result in results {
        matches.push(result?);
    }

    Ok(matches)
}

/// Generate a compact project summary.
pub fn project_summary(indexer: &FileIndexer) -> Result<ProjectSummary> {
    let conn = indexer.connection();

    let total_files = indexer.file_count()?;

    let total_size: i64 =
        conn.query_row("SELECT COALESCE(SUM(size), 0) FROM files", [], |row| {
            row.get(0)
        })?;

    // Language breakdown by extension
    let mut stmt = conn.prepare(
        "SELECT extension, COUNT(*), SUM(size) FROM files
         WHERE extension != ''
         GROUP BY extension
         ORDER BY COUNT(*) DESC
         LIMIT 20",
    )?;

    let languages: Vec<LanguageBreakdown> = stmt
        .query_map([], |row| {
            Ok(LanguageBreakdown {
                extension: row.get(0)?,
                file_count: row.get::<_, i64>(1)? as usize,
                total_size: row.get::<_, i64>(2)? as u64,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();

    // Key files (common important files)
    let key_file_names = [
        "Cargo.toml",
        "package.json",
        "go.mod",
        "requirements.txt",
        "pyproject.toml",
        "Makefile",
        "CMakeLists.txt",
        "Dockerfile",
        ".uproject",
        "README.md",
        "BUILD",
        "build.gradle",
        "pom.xml",
    ];

    let mut key_files = Vec::new();
    for name in &key_file_names {
        let pattern = format!("%{name}");
        let mut stmt = conn.prepare("SELECT path FROM files WHERE path LIKE ?1 LIMIT 3")?;
        let paths: Vec<String> = stmt
            .query_map([&pattern], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();
        key_files.extend(paths);
    }

    Ok(ProjectSummary {
        total_files,
        total_size: total_size as u64,
        languages,
        key_files,
    })
}

/// Render a project summary as compact text.
pub fn render_summary_text(summary: &ProjectSummary) -> String {
    let mut output = String::new();

    output.push_str(&format!(
        "Files: {}  Size: {}\n\n",
        summary.total_files,
        format_size(summary.total_size)
    ));

    if !summary.languages.is_empty() {
        output.push_str("Languages:\n");
        for lang in &summary.languages {
            output.push_str(&format!(
                "  .{:<12} {:>5} files  {}\n",
                lang.extension,
                lang.file_count,
                format_size(lang.total_size)
            ));
        }
        output.push('\n');
    }

    if !summary.key_files.is_empty() {
        output.push_str("Key files:\n");
        for f in &summary.key_files {
            output.push_str(&format!("  {f}\n"));
        }
    }

    output
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.1}G", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1}M", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1}K", bytes as f64 / KB as f64)
    } else {
        format!("{bytes}B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_find_files() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        std::fs::write(root.join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(root.join("lib.rs"), "pub fn lib() {}").unwrap();
        std::fs::write(root.join("README.md"), "# Hello").unwrap();

        let db_path = root.join(".claude-tools/files.db");
        let indexer = FileIndexer::open(&db_path).unwrap();
        indexer.build(root, false).unwrap();

        let results = find_files(&indexer, None, Some("rs"), None, None, 10).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_project_summary() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        std::fs::write(root.join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]").unwrap();

        let db_path = root.join(".claude-tools/files.db");
        let indexer = FileIndexer::open(&db_path).unwrap();
        indexer.build(root, false).unwrap();

        let summary = project_summary(&indexer).unwrap();
        assert_eq!(summary.total_files, 2);
    }
}
