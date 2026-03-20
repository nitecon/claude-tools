use anyhow::Result;
use ignore::WalkBuilder;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct TreeEntry {
    pub name: String,
    pub is_dir: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<TreeEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct TreeOptions {
    pub max_depth: usize,
    pub max_files_per_dir: usize,
}

impl Default for TreeOptions {
    fn default() -> Self {
        Self {
            max_depth: 3,
            max_files_per_dir: 20,
        }
    }
}

/// Generate a token-efficient tree view of a directory.
pub fn tree(root: &Path, options: &TreeOptions) -> Result<TreeEntry> {
    let root = if root.is_relative() {
        std::env::current_dir()?.join(root)
    } else {
        root.to_path_buf()
    };

    let root_name = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string());

    let mut dir_tree: BTreeMap<String, Vec<(String, bool)>> = BTreeMap::new();

    let walker = WalkBuilder::new(&root)
        .max_depth(Some(options.max_depth))
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
        if path == root {
            continue;
        }

        let relative = match path.strip_prefix(&root) {
            Ok(r) => r,
            Err(_) => continue,
        };

        let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);

        // Get parent directory key
        let parent_key = relative
            .parent()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();

        let name = relative
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        dir_tree.entry(parent_key).or_default().push((name, is_dir));
    }

    let children = build_tree_children("", &dir_tree, options.max_files_per_dir);

    Ok(TreeEntry {
        name: root_name,
        is_dir: true,
        children,
        truncated: None,
    })
}

fn build_tree_children(
    prefix: &str,
    dir_tree: &BTreeMap<String, Vec<(String, bool)>>,
    max_files: usize,
) -> Vec<TreeEntry> {
    let entries = match dir_tree.get(prefix) {
        Some(e) => e,
        None => return Vec::new(),
    };

    // Sort: directories first, then alphabetically
    let mut sorted: Vec<_> = entries.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    let total = sorted.len();
    let truncated = if total > max_files {
        Some(total - max_files)
    } else {
        None
    };

    let mut result: Vec<TreeEntry> = sorted
        .iter()
        .take(max_files)
        .map(|(name, is_dir)| {
            let child_prefix = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };

            let children = if *is_dir {
                build_tree_children(&child_prefix, dir_tree, max_files)
            } else {
                Vec::new()
            };

            TreeEntry {
                name: name.clone(),
                is_dir: *is_dir,
                children,
                truncated: None,
            }
        })
        .collect();

    if let Some(count) = truncated {
        result.push(TreeEntry {
            name: format!("[+{count} more]"),
            is_dir: false,
            children: Vec::new(),
            truncated: Some(count),
        });
    }

    result
}

/// Render a tree to compact text format.
pub fn render_tree_text(entry: &TreeEntry, indent: usize) -> String {
    let mut output = String::new();
    let prefix = "  ".repeat(indent);

    if entry.is_dir {
        output.push_str(&format!("{prefix}{}/\n", entry.name));
    } else {
        output.push_str(&format!("{prefix}{}\n", entry.name));
    }

    for child in &entry.children {
        output.push_str(&render_tree_text(child, indent + 1));
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_tree_basic() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}").unwrap();
        fs::write(root.join("src/lib.rs"), "").unwrap();
        fs::write(root.join("Cargo.toml"), "[package]").unwrap();

        let result = tree(root, &TreeOptions::default()).unwrap();
        assert!(result.is_dir);
        assert!(!result.children.is_empty());
    }

    #[test]
    fn test_tree_max_files_truncation() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        for i in 0..25 {
            fs::write(root.join(format!("file_{i:02}.txt")), "").unwrap();
        }

        let options = TreeOptions {
            max_depth: 1,
            max_files_per_dir: 10,
        };
        let result = tree(root, &options).unwrap();

        // Should have 10 files + 1 "[+N more]" entry
        assert_eq!(result.children.len(), 11);
        let last = result.children.last().unwrap();
        assert!(last.name.contains("+"));
    }

    #[test]
    fn test_render_text() {
        let entry = TreeEntry {
            name: "project".to_string(),
            is_dir: true,
            truncated: None,
            children: vec![
                TreeEntry {
                    name: "src".to_string(),
                    is_dir: true,
                    truncated: None,
                    children: vec![TreeEntry {
                        name: "main.rs".to_string(),
                        is_dir: false,
                        children: Vec::new(),
                        truncated: None,
                    }],
                },
                TreeEntry {
                    name: "Cargo.toml".to_string(),
                    is_dir: false,
                    children: Vec::new(),
                    truncated: None,
                },
            ],
        };

        let text = render_tree_text(&entry, 0);
        assert!(text.contains("project/"));
        assert!(text.contains("  src/"));
        assert!(text.contains("    main.rs"));
        assert!(text.contains("  Cargo.toml"));
    }
}
