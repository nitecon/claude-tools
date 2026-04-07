//! Consolidated self-updater for the agent-tools suite.
//!
//! Provides both async and blocking entry points for checking GitHub releases
//! and atomically replacing installed binaries. The update flow:
//!
//! 1. Check the rate-limit marker (`~/.agentic/.agent-tools-update-check`).
//! 2. Fetch the latest GitHub release from `nitecon/agent-tools`.
//! 3. Compare the remote version against the compile-time version.
//! 4. Download the platform-specific archive to a temp directory.
//! 5. Extract matching binaries and atomically replace them (write `.new`, chmod, rename).
//! 6. Clean up temp files.
//!
//! Set `AGENT_TOOLS_NO_UPDATE=1` to disable all automatic update checks.

use anyhow::{bail, Context, Result};
use semver::Version;
use serde::Deserialize;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const GITHUB_REPO: &str = "nitecon/agent-tools";

/// Compile-time version injected by the workspace build script.
/// Falls back to `CARGO_PKG_VERSION` when `AGENT_TOOLS_VERSION` is unset (e.g. during
/// `cargo test` of this crate in isolation).
const CURRENT_VERSION: &str = match option_env!("AGENT_TOOLS_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};

/// Rate-limit interval — at most one automatic check per hour.
const CHECK_INTERVAL_SECS: u64 = 3600;

/// Binary names we look for inside the release archive.
#[cfg(unix)]
const BINARY_NAMES: &[&str] = &["agent-tools", "agent-tools-mcp", "agent-sync"];

#[cfg(windows)]
const BINARY_NAMES: &[&str] = &["agent-tools.exe", "agent-tools-mcp.exe", "agent-sync.exe"];

// ---------------------------------------------------------------------------
// GitHub API types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

// ---------------------------------------------------------------------------
// Public API — async
// ---------------------------------------------------------------------------

/// Spawn a background update check (async, rate-limited).
///
/// Intended for long-running processes such as the MCP server. The check runs
/// on a separate tokio task and logs errors to stderr without propagating them.
/// Respects `AGENT_TOOLS_NO_UPDATE=1`.
pub fn spawn_update_check() {
    if std::env::var("AGENT_TOOLS_NO_UPDATE").is_ok() {
        return;
    }

    tokio::spawn(async {
        if let Err(e) = check_and_update(true).await {
            eprintln!("[agent-tools] update check: {e}");
        }
    });
}

/// Run a manual update with no rate limiting (async).
///
/// Prints current version and update status to stderr. Returns an error if
/// network or filesystem operations fail.
pub async fn manual_update() -> Result<()> {
    do_manual_update().await
}

// ---------------------------------------------------------------------------
// Public API — blocking
// ---------------------------------------------------------------------------

/// Rate-limited auto-update check (blocking).
///
/// Intended for short-lived CLI invocations. Creates a minimal single-threaded
/// tokio runtime to drive the async update logic. Respects `AGENT_TOOLS_NO_UPDATE=1`.
pub fn auto_update_blocking() {
    if std::env::var("AGENT_TOOLS_NO_UPDATE").is_ok() {
        return;
    }

    if let Err(e) = run_blocking(check_and_update(true)) {
        eprintln!("[agent-tools] update check: {e}");
    }
}

/// Run a manual update with no rate limiting (blocking).
///
/// Creates a minimal single-threaded tokio runtime to drive the async update
/// logic. Prints current version and update status to stderr.
pub fn manual_update_blocking() -> Result<()> {
    run_blocking(do_manual_update())
}

/// Clean up leftover `.old` binaries from a previous Windows update.
///
/// On Unix this is a no-op because atomic rename leaves no artifacts.
#[cfg(windows)]
pub fn cleanup_old_binaries() {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for name in BINARY_NAMES {
                let old = dir.join(format!("{name}.old"));
                let _ = std::fs::remove_file(&old);
            }
        }
    }
}

/// Clean up leftover `.old` binaries from a previous Windows update.
///
/// No-op on Unix — atomic rename leaves no artifacts.
#[cfg(not(windows))]
pub fn cleanup_old_binaries() {
    // Nothing to do on Unix.
}

// ---------------------------------------------------------------------------
// Blocking runtime helper
// ---------------------------------------------------------------------------

/// Create a minimal current-thread tokio runtime and block on the given future.
fn run_blocking<F: std::future::Future<Output = Result<()>>>(f: F) -> Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime for blocking update")?
        .block_on(f)
}

// ---------------------------------------------------------------------------
// Core async logic
// ---------------------------------------------------------------------------

/// Main update flow. When `rate_limited` is true the marker file is checked
/// first and the update is skipped if the last check was within the interval.
async fn check_and_update(rate_limited: bool) -> Result<()> {
    let marker = marker_path()?;

    if rate_limited && !should_check(&marker) {
        return Ok(());
    }

    let current = Version::parse(CURRENT_VERSION.trim_start_matches('v'))
        .context("invalid current version")?;

    let client = build_client()?;
    let release = fetch_latest_release(&client).await?;

    // Always update the marker so we don't re-check immediately on failure.
    touch_marker(&marker);

    let latest = Version::parse(release.tag_name.trim_start_matches('v'))
        .context("invalid release version")?;

    if latest <= current {
        return Ok(());
    }

    eprintln!("[agent-tools] update available: v{current} -> v{latest}");

    download_and_install(&client, &release).await?;

    eprintln!("[agent-tools] updated to v{latest} — will take effect on next restart");
    Ok(())
}

/// Manual update — no rate limiting, prints version info and status to stderr.
async fn do_manual_update() -> Result<()> {
    let current = Version::parse(CURRENT_VERSION.trim_start_matches('v'))
        .context("invalid current version")?;
    eprintln!("[agent-tools] current version: v{current}");

    let client = build_client()?;
    let release = fetch_latest_release(&client).await?;

    let latest = Version::parse(release.tag_name.trim_start_matches('v'))
        .context("invalid release version")?;

    if latest <= current {
        eprintln!("[agent-tools] already up to date (v{current})");
        touch_marker(&marker_path()?);
        return Ok(());
    }

    eprintln!("[agent-tools] updating: v{current} -> v{latest}");

    download_and_install(&client, &release).await?;

    touch_marker(&marker_path()?);
    eprintln!("[agent-tools] updated to v{latest} — will take effect on next invocation");
    Ok(())
}

// ---------------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------------

/// Build a reqwest client with a sensible user-agent and timeout.
fn build_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent("agent-tools-updater")
        .timeout(std::time::Duration::from_secs(15))
        .build()?)
}

/// Fetch the latest release metadata from GitHub.
async fn fetch_latest_release(client: &reqwest::Client) -> Result<GitHubRelease> {
    let url = format!("https://api.github.com/repos/{GITHUB_REPO}/releases/latest");
    Ok(client
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

/// Download the release archive and extract/replace binaries in the exe directory.
async fn download_and_install(client: &reqwest::Client, release: &GitHubRelease) -> Result<()> {
    let target = current_target()?;
    let archive_prefix = format!("agent-tools-{}-{target}", release.tag_name);

    let asset = release
        .assets
        .iter()
        .find(|a| a.name.starts_with(&archive_prefix))
        .context("no release asset for this platform")?;

    // Download to OS temp directory.
    let temp_dir = std::env::temp_dir().join(format!("agent-tools-update-{}", release.tag_name));
    std::fs::create_dir_all(&temp_dir)?;

    let archive_path = temp_dir.join(&asset.name);
    download_file(client, &asset.browser_download_url, &archive_path).await?;

    // Resolve actual binary location (follows symlinks).
    let exe_dir = std::env::current_exe()?
        .canonicalize()?
        .parent()
        .context("current exe has no parent directory")?
        .to_path_buf();

    extract_and_replace(&archive_path, &exe_dir)?;

    // Cleanup temp.
    let _ = std::fs::remove_dir_all(&temp_dir);
    Ok(())
}

/// Download a URL to a local file path.
async fn download_file(client: &reqwest::Client, url: &str, path: &Path) -> Result<()> {
    let bytes = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    std::fs::write(path, &bytes)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Platform detection
// ---------------------------------------------------------------------------

/// Map the current OS/arch to a Rust target triple.
fn current_target() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-gnu"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("windows", "x86_64") => Ok("x86_64-pc-windows-msvc"),
        ("windows", "aarch64") => Ok("aarch64-pc-windows-msvc"),
        (os, arch) => bail!("unsupported platform: {os}/{arch}"),
    }
}

// ---------------------------------------------------------------------------
// Rate-limit marker
// ---------------------------------------------------------------------------

/// Return the path to the rate-limit marker file (`~/.agentic/.agent-tools-update-check`).
fn marker_path() -> Result<PathBuf> {
    #[cfg(unix)]
    let dir = {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(home).join(".agentic")
    };

    #[cfg(windows)]
    let dir = {
        let home = std::env::var("USERPROFILE").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".agentic")
    };

    std::fs::create_dir_all(&dir)?;
    Ok(dir.join(".agent-tools-update-check"))
}

/// Return `true` if enough time has elapsed since the marker was last touched.
fn should_check(marker: &Path) -> bool {
    marker
        .metadata()
        .and_then(|m| m.modified())
        .map(|t| t.elapsed().unwrap_or_default().as_secs() > CHECK_INTERVAL_SECS)
        .unwrap_or(true) // no marker = never checked
}

/// Touch the marker file (write empty content, updating its mtime).
fn touch_marker(marker: &Path) {
    let _ = std::fs::write(marker, "");
}

// ---------------------------------------------------------------------------
// Archive extraction — Unix (tar.gz)
// ---------------------------------------------------------------------------

/// Extract matching binaries from a `.tar.gz` archive and atomically replace them.
///
/// Strategy: unpack each matching entry to `<name>.new`, set permissions to 0o755,
/// then rename over the existing binary. Only binaries that are already installed
/// (i.e. the target path exists) are replaced.
#[cfg(unix)]
fn extract_and_replace(archive: &Path, exe_dir: &Path) -> Result<()> {
    use flate2::read::GzDecoder;
    use std::os::unix::fs::PermissionsExt;
    use tar::Archive;

    let file = std::fs::File::open(archive)?;
    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        if !BINARY_NAMES.contains(&file_name.as_str()) {
            continue;
        }

        let target = exe_dir.join(&file_name);
        if !target.exists() {
            continue; // only update binaries already installed
        }

        // Write to .new, then atomically rename.
        let staging = exe_dir.join(format!("{file_name}.new"));
        entry.unpack(&staging)?;

        let mut perms = std::fs::metadata(&staging)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&staging, perms)?;

        std::fs::rename(&staging, &target)
            .with_context(|| format!("failed to replace {}", target.display()))?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Archive extraction — Windows (zip)
// ---------------------------------------------------------------------------

/// Extract matching binaries from a `.zip` archive and replace them.
///
/// On Windows we cannot rename over a running executable, so the strategy is:
/// 1. Write the new binary to `<name>.new`.
/// 2. Rename the running binary to `<name>.old`.
/// 3. Rename `<name>.new` to `<name>`.
///
/// Leftover `.old` files are cleaned up on the next invocation via
/// [`cleanup_old_binaries`].
#[cfg(windows)]
fn extract_and_replace(archive: &Path, exe_dir: &Path) -> Result<()> {
    use std::io::Read;

    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)?;

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let path = match entry.enclosed_name() {
            Some(p) => p.to_path_buf(),
            None => continue,
        };
        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        if !BINARY_NAMES.contains(&file_name.as_str()) {
            continue;
        }

        let target = exe_dir.join(&file_name);
        if !target.exists() {
            continue;
        }

        let staging = exe_dir.join(format!("{file_name}.new"));
        let old = exe_dir.join(format!("{file_name}.old"));

        // Write new binary to staging path.
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        std::fs::write(&staging, &buf)?;

        // Swap: running binary -> .old, new -> target.
        let _ = std::fs::remove_file(&old); // clean up previous .old
        std::fs::rename(&target, &old)
            .with_context(|| format!("failed to move {} to .old", target.display()))?;
        std::fs::rename(&staging, &target)
            .with_context(|| format!("failed to move .new to {}", target.display()))?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use semver::Version;

    #[test]
    fn test_version_comparison() {
        let older = Version::parse("0.1.0").unwrap();
        let newer = Version::parse("0.2.0").unwrap();
        assert!(newer > older);

        let same = Version::parse("0.1.0").unwrap();
        assert!(same <= older);
        assert!(older <= same);
    }

    #[test]
    fn test_version_with_v_prefix() {
        let tag = "v1.2.3";
        let version = Version::parse(tag.trim_start_matches('v')).unwrap();
        assert_eq!(version, Version::new(1, 2, 3));
    }

    #[test]
    fn test_prerelease_is_less_than_release() {
        let pre = Version::parse("1.0.0-rc.1").unwrap();
        let release = Version::parse("1.0.0").unwrap();
        assert!(pre < release);
    }

    #[test]
    fn test_current_version_parses() {
        // Validates that the compile-time CURRENT_VERSION constant is valid semver
        // (with optional 'v' prefix stripped).
        let v = Version::parse(super::CURRENT_VERSION.trim_start_matches('v'));
        assert!(
            v.is_ok(),
            "CURRENT_VERSION '{}' is not valid semver",
            super::CURRENT_VERSION
        );
    }
}
