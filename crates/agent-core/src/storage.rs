use std::path::{Path, PathBuf};

/// Returns the centralized data directory for a project.
///
/// Resolution order (first existing directory with the project hash wins):
/// 1. **User-level:** `~/.agent-tools/<hash>/` — overrides global
/// 2. **Global:** `/opt/agentic/tools/<hash>/` (Unix) or `%USERPROFILE%\.agentic\tools\<hash>\` (Windows)
///
/// If neither exists yet, the user-level directory is chosen when the global
/// base directory does not exist or is not writable, otherwise the global directory is used.
///
/// The hash is a blake3 digest of a normalized project identifier:
/// - If the project is a git repo with a remote origin, the identifier is the
///   normalized origin URL (e.g., `github.com/nitecon/agent-tools.git`).
/// - Otherwise, the identifier is the canonical absolute path of the project root.
pub fn project_data_dir(project_root: &Path) -> PathBuf {
    let identifier = resolve_project_identifier(project_root);
    let hash = blake3::hash(identifier.as_bytes()).to_hex().to_string();

    let user_dir = user_tools_dir().join(&hash);
    let global_dir = global_tools_dir().join(&hash);

    // If the user-level dir already has data for this project, use it (override)
    if user_dir.exists() {
        return user_dir;
    }

    // If the global dir already has data for this project, use it
    if global_dir.exists() {
        return global_dir;
    }

    // Neither exists yet — pick the best location for new data
    let global_base = global_tools_dir();
    if global_base.exists() && is_writable(&global_base) {
        global_dir
    } else {
        user_dir
    }
}

/// Returns the user-level tools storage directory.
///
/// `~/.agent-tools` on all platforms.
pub fn user_tools_dir() -> PathBuf {
    home_dir().join(".agent-tools")
}

/// Returns the global tools storage directory.
///
/// On Unix: `/opt/agentic/tools`
/// On Windows: `%USERPROFILE%\.agentic\tools`
pub fn global_tools_dir() -> PathBuf {
    #[cfg(unix)]
    {
        PathBuf::from("/opt/agentic/tools")
    }

    #[cfg(windows)]
    {
        let home = std::env::var("USERPROFILE").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".agentic").join("tools")
    }
}

/// Resolve the user's home directory.
fn home_dir() -> PathBuf {
    #[cfg(unix)]
    {
        std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp"))
    }

    #[cfg(windows)]
    {
        std::env::var("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
    }
}

/// Check if a directory is writable by attempting to create a temp file.
fn is_writable(dir: &Path) -> bool {
    let probe = dir.join(".write-probe");
    match std::fs::write(&probe, b"") {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Resolve a stable project identifier from the project root.
///
/// Prefers the git remote origin URL (normalized). Falls back to the
/// canonicalized absolute path of the project root.
fn resolve_project_identifier(project_root: &Path) -> String {
    // Try git remote origin URL
    if let Ok(output) = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(project_root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
    {
        if output.status.success() {
            let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !url.is_empty() {
                return normalize_git_url(&url);
            }
        }
    }

    // Fallback: canonical absolute path
    project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf())
        .to_string_lossy()
        .to_string()
}

/// Normalize a git remote URL so that SSH and HTTPS URLs for the same repo
/// produce the same identifier.
///
/// Examples:
///   `git@github.com:nitecon/agent-tools.git`       -> `github.com/nitecon/agent-tools.git`
///   `https://github.com/nitecon/agent-tools.git`    -> `github.com/nitecon/agent-tools.git`
///   `ssh://git@github.com/nitecon/agent-tools.git`  -> `github.com/nitecon/agent-tools.git`
///   `https://user@github.com/nitecon/agent-tools.git` -> `github.com/nitecon/agent-tools.git`
fn normalize_git_url(url: &str) -> String {
    let mut s = url.to_string();

    // Strip protocol prefix
    for proto in &["https://", "http://", "ssh://"] {
        if let Some(rest) = s.strip_prefix(proto) {
            s = rest.to_string();
            break;
        }
    }

    // Strip user@ prefix (e.g., git@, user@)
    if let Some(at_pos) = s.find('@') {
        // Only strip if '@' comes before the first '/' or ':'
        let slash_pos = s.find('/').unwrap_or(usize::MAX);
        let colon_pos = s.find(':').unwrap_or(usize::MAX);
        if at_pos < slash_pos && at_pos < colon_pos {
            s = s[at_pos + 1..].to_string();
        }
    }

    // Convert SSH shorthand colon to slash (github.com:user/repo -> github.com/user/repo)
    // Only convert the first colon that appears before any slash (SSH shorthand pattern)
    if let Some(colon_pos) = s.find(':') {
        let slash_pos = s.find('/').unwrap_or(usize::MAX);
        if colon_pos < slash_pos {
            s = format!("{}/{}", &s[..colon_pos], &s[colon_pos + 1..]);
        }
    }

    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_git_url_ssh() {
        assert_eq!(
            normalize_git_url("git@github.com:nitecon/agent-tools.git"),
            "github.com/nitecon/agent-tools.git"
        );
    }

    #[test]
    fn test_normalize_git_url_https() {
        assert_eq!(
            normalize_git_url("https://github.com/nitecon/agent-tools.git"),
            "github.com/nitecon/agent-tools.git"
        );
    }

    #[test]
    fn test_normalize_git_url_ssh_explicit() {
        assert_eq!(
            normalize_git_url("ssh://git@github.com/nitecon/agent-tools.git"),
            "github.com/nitecon/agent-tools.git"
        );
    }

    #[test]
    fn test_normalize_git_url_https_with_user() {
        assert_eq!(
            normalize_git_url("https://user@github.com/nitecon/agent-tools.git"),
            "github.com/nitecon/agent-tools.git"
        );
    }

    #[test]
    fn test_normalize_git_url_http() {
        assert_eq!(
            normalize_git_url("http://github.com/nitecon/agent-tools.git"),
            "github.com/nitecon/agent-tools.git"
        );
    }

    #[test]
    fn test_ssh_and_https_produce_same_hash() {
        let ssh = normalize_git_url("git@github.com:nitecon/agent-tools.git");
        let https = normalize_git_url("https://github.com/nitecon/agent-tools.git");
        assert_eq!(ssh, https);
        assert_eq!(
            blake3::hash(ssh.as_bytes()).to_hex().to_string(),
            blake3::hash(https.as_bytes()).to_hex().to_string()
        );
    }

    #[test]
    fn test_project_data_dir_returns_hashed_path() {
        // Just verify the function runs and returns a path with a hash component
        let dir = std::env::temp_dir();
        let result = project_data_dir(&dir);
        let file_name = result.file_name().unwrap().to_str().unwrap();
        // blake3 hex is 64 chars
        assert_eq!(file_name.len(), 64);
    }

    #[test]
    #[cfg(unix)]
    fn test_user_dir_overrides_global() {
        // On Unix, user (~/.agent-tools) and global (/opt/agentic/tools) are distinct
        let user = user_tools_dir();
        let global = global_tools_dir();
        assert_ne!(user, global);
    }

    #[test]
    fn test_is_writable_temp() {
        // Temp dir should be writable
        assert!(is_writable(&std::env::temp_dir()));
    }
}
