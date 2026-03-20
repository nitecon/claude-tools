use anyhow::Result;
use serde::Serialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct ListEntry {
    pub name: String,
    pub entry_type: EntryType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryType {
    Dir,
    File,
    Symlink,
}

#[derive(Debug, Clone, Default)]
pub struct ListOptions {
    pub show_sizes: bool,
    pub show_hidden: bool,
}

/// List directory contents with minimal output.
/// Directories first, then files, alphabetically within each group.
pub fn list_dir(path: &Path, options: &ListOptions) -> Result<Vec<ListEntry>> {
    let path = if path.is_relative() {
        std::env::current_dir()?.join(path)
    } else {
        path.to_path_buf()
    };

    let mut dirs = Vec::new();
    let mut files = Vec::new();

    for entry in fs::read_dir(&path)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();

        if !options.show_hidden && name.starts_with('.') {
            continue;
        }

        let metadata = entry.metadata()?;
        let file_type = entry.file_type()?;

        let entry_type = if file_type.is_dir() {
            EntryType::Dir
        } else if file_type.is_symlink() {
            EntryType::Symlink
        } else {
            EntryType::File
        };

        let size = if options.show_sizes && entry_type == EntryType::File {
            Some(metadata.len())
        } else {
            None
        };

        let list_entry = ListEntry {
            name,
            entry_type,
            size,
        };

        match entry_type {
            EntryType::Dir => dirs.push(list_entry),
            _ => files.push(list_entry),
        }
    }

    dirs.sort_by(|a, b| a.name.cmp(&b.name));
    files.sort_by(|a, b| a.name.cmp(&b.name));

    dirs.extend(files);
    Ok(dirs)
}

/// Render list entries as compact text.
pub fn render_list_text(entries: &[ListEntry]) -> String {
    let mut output = String::new();
    for entry in entries {
        match entry.entry_type {
            EntryType::Dir => {
                output.push_str(&format!("{}/\n", entry.name));
            }
            EntryType::File => {
                if let Some(size) = entry.size {
                    output.push_str(&format!("{}  {}\n", entry.name, format_size(size)));
                } else {
                    output.push_str(&format!("{}\n", entry.name));
                }
            }
            EntryType::Symlink => {
                output.push_str(&format!("{}@\n", entry.name));
            }
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
    fn test_list_dir() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        fs::create_dir(root.join("subdir")).unwrap();
        fs::write(root.join("file.txt"), "hello").unwrap();
        fs::write(root.join(".hidden"), "secret").unwrap();

        let entries = list_dir(root, &ListOptions::default()).unwrap();
        assert_eq!(entries.len(), 2); // subdir + file.txt (hidden excluded)
        assert_eq!(entries[0].entry_type, EntryType::Dir);
        assert_eq!(entries[0].name, "subdir");
    }

    #[test]
    fn test_list_with_sizes() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        fs::write(root.join("file.txt"), "hello world").unwrap();

        let options = ListOptions {
            show_sizes: true,
            show_hidden: false,
        };
        let entries = list_dir(root, &options).unwrap();
        assert!(entries[0].size.is_some());
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(500), "500B");
        assert_eq!(format_size(1536), "1.5K");
        assert_eq!(format_size(2_097_152), "2.0M");
    }
}
