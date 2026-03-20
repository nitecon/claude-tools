use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// Copy a file or directory recursively.
pub fn copy(src: &Path, dst: &Path) -> Result<()> {
    if src.is_dir() {
        copy_dir_recursive(src, dst)
    } else {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create parent dirs for {}", dst.display()))?;
        }
        fs::copy(src, dst)
            .with_context(|| format!("Failed to copy {} to {}", src.display(), dst.display()))?;
        Ok(())
    }
}

/// Move a file or directory.
pub fn move_path(src: &Path, dst: &Path) -> Result<()> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create parent dirs for {}", dst.display()))?;
    }

    // Try rename first (fast, same filesystem)
    match fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(_) => {
            // Cross-filesystem: copy then remove
            copy(src, dst)?;
            remove(src)?;
            Ok(())
        }
    }
}

/// Create directories recursively (like `mkdir -p`).
pub fn mkdir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)
        .with_context(|| format!("Failed to create directory {}", path.display()))?;
    Ok(())
}

/// Remove a file or directory recursively.
pub fn remove(path: &Path) -> Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)
            .with_context(|| format!("Failed to remove directory {}", path.display()))?;
    } else {
        fs::remove_file(path)
            .with_context(|| format!("Failed to remove file {}", path.display()))?;
    }
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)
        .with_context(|| format!("Failed to create directory {}", dst.display()))?;

    for entry in
        fs::read_dir(src).with_context(|| format!("Failed to read directory {}", src.display()))?
    {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path).with_context(|| {
                format!(
                    "Failed to copy {} to {}",
                    src_path.display(),
                    dst_path.display()
                )
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_copy_file() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("source.txt");
        let dst = dir.path().join("dest.txt");

        fs::write(&src, "hello").unwrap();
        copy(&src, &dst).unwrap();

        assert_eq!(fs::read_to_string(&dst).unwrap(), "hello");
        assert!(src.exists()); // source still exists
    }

    #[test]
    fn test_copy_dir() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src_dir");
        let dst = dir.path().join("dst_dir");

        fs::create_dir(&src).unwrap();
        fs::write(src.join("file.txt"), "content").unwrap();

        copy(&src, &dst).unwrap();
        assert_eq!(fs::read_to_string(dst.join("file.txt")).unwrap(), "content");
    }

    #[test]
    fn test_move_file() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("source.txt");
        let dst = dir.path().join("dest.txt");

        fs::write(&src, "hello").unwrap();
        move_path(&src, &dst).unwrap();

        assert!(!src.exists());
        assert_eq!(fs::read_to_string(&dst).unwrap(), "hello");
    }

    #[test]
    fn test_mkdir_recursive() {
        let dir = TempDir::new().unwrap();
        let nested = dir.path().join("a/b/c/d");

        mkdir(&nested).unwrap();
        assert!(nested.is_dir());
    }

    #[test]
    fn test_remove_file() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("file.txt");

        fs::write(&file, "hello").unwrap();
        remove(&file).unwrap();
        assert!(!file.exists());
    }

    #[test]
    fn test_remove_dir() {
        let dir = TempDir::new().unwrap();
        let subdir = dir.path().join("subdir");

        fs::create_dir(&subdir).unwrap();
        fs::write(subdir.join("file.txt"), "content").unwrap();

        remove(&subdir).unwrap();
        assert!(!subdir.exists());
    }
}
