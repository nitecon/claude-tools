//! Unified configuration system for agent-tools.
//!
//! Config is loaded from a three-layer hierarchy (lowest to highest priority):
//!
//! 1. `/opt/agentic/agent-tools/gateway.conf` -- system-wide global (KEY=VALUE)
//! 2. `~/.agentic/agent-tools/gateway.conf` -- per-user override (KEY=VALUE)
//! 3. Environment variables (`GATEWAY_URL`, `GATEWAY_API_KEY`, etc.)

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

// -- Public types -------------------------------------------------------------

/// Top-level configuration container.
#[derive(Debug, Default, Clone)]
pub struct Config {
    pub gateway: GatewayConfig,
}

/// Gateway connection settings.
#[derive(Debug, Default, Clone)]
pub struct GatewayConfig {
    pub url: Option<String>,
    pub api_key: Option<String>,
    pub timeout_ms: Option<u64>,
    pub default_project: Option<String>,
}

// -- Path helpers -------------------------------------------------------------

/// Return the user's home directory via `HOME` (unix) or `USERPROFILE` (windows).
///
/// # Panics
/// Panics if neither environment variable is set.
pub fn home_dir() -> PathBuf {
    if let Ok(h) = std::env::var("HOME") {
        return PathBuf::from(h);
    }
    if let Ok(h) = std::env::var("USERPROFILE") {
        return PathBuf::from(h);
    }
    panic!("neither HOME nor USERPROFILE is set");
}

/// Path to the per-user gateway config: `~/.agentic/agent-tools/gateway.conf`.
pub fn user_gateway_conf_path() -> PathBuf {
    home_dir()
        .join(".agentic")
        .join("agent-tools")
        .join("gateway.conf")
}

/// Path to the system-wide gateway config: `/opt/agentic/agent-tools/gateway.conf`.
pub fn global_gateway_conf_path() -> PathBuf {
    PathBuf::from("/opt/agentic/agent-tools/gateway.conf")
}

// -- Config loading -----------------------------------------------------------

/// Load configuration from all layers and return the merged result.
///
/// Resolution order (later wins):
/// 1. Global gateway.conf (`/opt/agentic/agent-tools/gateway.conf`)
/// 2. User gateway.conf (`~/.agentic/agent-tools/gateway.conf`)
/// 3. Environment variables -- override everything
pub fn load_config() -> Config {
    let mut cfg = Config::default();

    // Layer 1: system-wide global gateway.conf
    if let Some(pairs) = read_key_value_file(&global_gateway_conf_path()) {
        apply_key_value_pairs(&mut cfg, &pairs);
    }

    // Layer 2: per-user override gateway.conf (overwrites any global values)
    if let Some(pairs) = read_key_value_file(&user_gateway_conf_path()) {
        apply_key_value_pairs(&mut cfg, &pairs);
    }

    // Layer 3: environment variables (highest priority)
    apply_env_overrides(&mut cfg);

    cfg
}

/// Apply values from a KEY=VALUE map onto the config. Any key present in the
/// map overwrites the corresponding config field.
fn apply_key_value_pairs(cfg: &mut Config, pairs: &HashMap<String, String>) {
    if let Some(v) = pairs.get("GATEWAY_URL") {
        cfg.gateway.url = Some(v.clone());
    }
    if let Some(v) = pairs.get("GATEWAY_API_KEY") {
        cfg.gateway.api_key = Some(v.clone());
    }
    if let Some(v) = pairs.get("GATEWAY_TIMEOUT_MS") {
        if let Ok(ms) = v.parse::<u64>() {
            cfg.gateway.timeout_ms = Some(ms);
        }
    }
    if let Some(v) = pairs.get("DEFAULT_PROJECT_IDENT") {
        cfg.gateway.default_project = Some(v.clone());
    }
}

/// Apply environment variable overrides (highest priority layer).
fn apply_env_overrides(cfg: &mut Config) {
    if let Ok(v) = std::env::var("GATEWAY_URL") {
        cfg.gateway.url = Some(v);
    }
    if let Ok(v) = std::env::var("GATEWAY_API_KEY") {
        cfg.gateway.api_key = Some(v);
    }
    if let Ok(v) = std::env::var("GATEWAY_TIMEOUT_MS") {
        if let Ok(ms) = v.parse::<u64>() {
            cfg.gateway.timeout_ms = Some(ms);
        }
    }
    if let Ok(v) = std::env::var("DEFAULT_PROJECT_IDENT") {
        cfg.gateway.default_project = Some(v);
    }
}

/// Parse a simple KEY=VALUE file (lines starting with `#` are comments,
/// blank lines are skipped, values may be optionally quoted).
fn read_key_value_file(path: &PathBuf) -> Option<HashMap<String, String>> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut map = HashMap::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((key, val)) = trimmed.split_once('=') {
            let key = key.trim().to_string();
            let val = val.trim().trim_matches('"').trim_matches('\'').to_string();
            map.insert(key, val);
        }
    }
    Some(map)
}

// -- Interactive setup --------------------------------------------------------

/// Run an interactive setup wizard that writes `~/.agentic/agent-tools/gateway.conf`.
///
/// Prompts the user for gateway URL, API key, and timeout, then writes the
/// resulting KEY=VALUE file.
///
/// # Errors
/// Returns an error if stdin/stdout interaction fails or the config file cannot
/// be written.
pub fn run_setup_gateway() -> Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let mut reader = stdin.lock();

    // Gateway URL
    write!(out, "Gateway URL [http://localhost:7913]: ")?;
    out.flush()?;
    let mut url_input = String::new();
    reader.read_line(&mut url_input)?;
    let url = url_input.trim();
    let url = if url.is_empty() {
        "http://localhost:7913"
    } else {
        url
    };

    // API key (masked input)
    let api_key =
        rpassword::prompt_password("Gateway API key: ").context("failed to read API key")?;
    if api_key.trim().is_empty() {
        anyhow::bail!("API key cannot be empty");
    }
    let api_key = api_key.trim();

    // Timeout
    write!(out, "Request timeout in ms [5000]: ")?;
    out.flush()?;
    let mut timeout_input = String::new();
    reader.read_line(&mut timeout_input)?;
    let timeout: u64 = timeout_input.trim().parse().unwrap_or(5000);

    // Build KEY=VALUE content
    let mut content = String::new();
    content.push_str(&format!("GATEWAY_URL={url}\n"));
    content.push_str(&format!("GATEWAY_API_KEY={api_key}\n"));
    content.push_str(&format!("GATEWAY_TIMEOUT_MS={timeout}\n"));

    // Write the file
    let config_path = user_gateway_conf_path();
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create directory {}", parent.display()))?;
    }
    std::fs::write(&config_path, &content)
        .with_context(|| format!("write config to {}", config_path.display()))?;

    writeln!(out)?;
    writeln!(out, "Gateway config written to {}", config_path.display())?;
    writeln!(out)?;
    writeln!(
        out,
        "To register the MCP server, add to your Claude config:"
    )?;
    writeln!(out, "  {{")?;
    writeln!(out, "    \"mcpServers\": {{")?;
    writeln!(out, "      \"agent-tools\": {{")?;
    writeln!(
        out,
        "        \"command\": \"/opt/agentic/bin/agent-tools-mcp\""
    )?;
    writeln!(out, "      }}")?;
    writeln!(out, "    }}")?;
    writeln!(out, "  }}")?;

    Ok(())
}

/// Backwards-compatible alias for [`run_setup_gateway`].
///
/// # Errors
/// Delegates to `run_setup_gateway`; see its documentation for error conditions.
pub fn run_init() -> Result<()> {
    run_setup_gateway()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn home_dir_returns_path() {
        // Should not panic in a normal environment.
        let h = home_dir();
        assert!(!h.as_os_str().is_empty());
    }

    #[test]
    fn user_gateway_conf_path_ends_correctly() {
        let p = user_gateway_conf_path();
        assert!(p.ends_with(".agentic/agent-tools/gateway.conf"));
    }

    #[test]
    fn global_gateway_conf_path_is_absolute() {
        let p = global_gateway_conf_path();
        assert_eq!(p, PathBuf::from("/opt/agentic/agent-tools/gateway.conf"));
    }

    #[test]
    fn env_overrides_take_precedence() {
        // Set env vars, load config, verify they appear.
        env::set_var("GATEWAY_URL", "http://test:9999");
        env::set_var("GATEWAY_TIMEOUT_MS", "1234");
        let cfg = load_config();
        assert_eq!(cfg.gateway.url.as_deref(), Some("http://test:9999"));
        assert_eq!(cfg.gateway.timeout_ms, Some(1234));
        env::remove_var("GATEWAY_URL");
        env::remove_var("GATEWAY_TIMEOUT_MS");
    }

    #[test]
    fn apply_key_value_pairs_overwrites() {
        let mut cfg = Config {
            gateway: GatewayConfig {
                url: Some("http://base".into()),
                api_key: Some("key-base".into()),
                timeout_ms: Some(1000),
                default_project: None,
            },
        };
        let mut pairs = HashMap::new();
        pairs.insert("GATEWAY_URL".into(), "http://overlay".into());
        pairs.insert("DEFAULT_PROJECT_IDENT".into(), "proj".into());
        apply_key_value_pairs(&mut cfg, &pairs);
        assert_eq!(cfg.gateway.url.as_deref(), Some("http://overlay"));
        assert_eq!(cfg.gateway.api_key.as_deref(), Some("key-base"));
        assert_eq!(cfg.gateway.timeout_ms, Some(1000));
        assert_eq!(cfg.gateway.default_project.as_deref(), Some("proj"));
    }

    #[test]
    fn read_key_value_parses_correctly() {
        let dir = std::env::temp_dir().join("agent-comms-test-kv");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("test.conf");
        std::fs::write(
            &file,
            "# comment\nGATEWAY_URL=http://localhost:7913\nAPI_KEY=\"secret\"\n\nTIMEOUT=5000\n",
        )
        .unwrap();
        let map = read_key_value_file(&file).unwrap();
        assert_eq!(map.get("GATEWAY_URL").unwrap(), "http://localhost:7913");
        assert_eq!(map.get("API_KEY").unwrap(), "secret");
        assert_eq!(map.get("TIMEOUT").unwrap(), "5000");
        std::fs::remove_dir_all(&dir).ok();
    }
}
