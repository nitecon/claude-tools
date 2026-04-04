use anyhow::{bail, Context, Result};
use semver::Version;
use serde::Deserialize;
use std::path::{Path, PathBuf};

const GITHUB_REPO: &str = "nitecon/claude-tools";
const CURRENT_VERSION: &str = env!("CLAUDE_TOOLS_VERSION");
const CHECK_INTERVAL_SECS: u64 = 3600; // 1 hour

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

/// Spawn a background update check. Non-blocking, never panics.
pub fn spawn_update_check() {
    tokio::spawn(async {
        if let Err(e) = check_and_update().await {
            eprintln!("[claude-tools] update check: {e}");
        }
    });
}

async fn check_and_update() -> Result<()> {
    // Rate limit: at most once per hour
    let marker = marker_path()?;
    if !should_check(&marker) {
        return Ok(());
    }

    let current = Version::parse(CURRENT_VERSION).context("invalid current version")?;

    let client = reqwest::Client::builder()
        .user_agent("claude-tools-updater")
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let url = format!("https://api.github.com/repos/{GITHUB_REPO}/releases/latest");
    let release: GitHubRelease = client.get(&url).send().await?.error_for_status()?.json().await?;

    // Always update the marker so we don't re-check immediately on failure
    touch_marker(&marker);

    let latest = Version::parse(release.tag_name.trim_start_matches('v'))
        .context("invalid release version")?;

    if latest <= current {
        return Ok(());
    }

    eprintln!(
        "[claude-tools] update available: v{current} -> v{latest}"
    );

    let target = current_target()?;
    let archive_prefix = format!("claude-tools-{}-{target}", release.tag_name);

    let asset = release
        .assets
        .iter()
        .find(|a| a.name.starts_with(&archive_prefix))
        .context("no release asset for this platform")?;

    // Download to OS temp directory
    let temp_dir = std::env::temp_dir().join(format!("claude-tools-update-{}", release.tag_name));
    std::fs::create_dir_all(&temp_dir)?;

    let archive_path = temp_dir.join(&asset.name);
    download_file(&client, &asset.browser_download_url, &archive_path).await?;

    // Extract and replace binaries
    let exe_dir = std::env::current_exe()?
        .canonicalize()?
        .parent()
        .context("current exe has no parent directory")?
        .to_path_buf();

    extract_and_replace(&archive_path, &exe_dir)?;

    // Cleanup temp
    let _ = std::fs::remove_dir_all(&temp_dir);

    eprintln!("[claude-tools] updated to v{latest} — will take effect on next restart");
    Ok(())
}

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

fn marker_path() -> Result<PathBuf> {
    let dir = std::env::temp_dir().join("claude-tools");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("last-update-check"))
}

fn should_check(marker: &Path) -> bool {
    marker
        .metadata()
        .and_then(|m| m.modified())
        .map(|t| {
            t.elapsed()
                .unwrap_or_default()
                .as_secs()
                > CHECK_INTERVAL_SECS
        })
        .unwrap_or(true) // no marker = never checked
}

fn touch_marker(marker: &Path) {
    let _ = std::fs::write(marker, "");
}

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

/// Binary names we look for inside the release archive.
#[cfg(unix)]
const BINARY_NAMES: &[&str] = &["claude-tools-mcp", "claude-tools"];

#[cfg(windows)]
const BINARY_NAMES: &[&str] = &["claude-tools-mcp.exe", "claude-tools.exe"];

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

        // Write to .new, then atomically rename
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

        // Write new binary to staging path
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        std::fs::write(&staging, &buf)?;

        // Swap: running binary -> .old, new -> target
        let _ = std::fs::remove_file(&old); // clean up previous .old
        std::fs::rename(&target, &old)
            .with_context(|| format!("failed to move {} to .old", target.display()))?;
        std::fs::rename(&staging, &target)
            .with_context(|| format!("failed to move .new to {}", target.display()))?;
    }

    Ok(())
}

/// Clean up leftover .old binaries from a previous Windows update.
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

#[cfg(not(windows))]
pub fn cleanup_old_binaries() {
    // No-op on Unix — atomic rename leaves no artifacts
}
