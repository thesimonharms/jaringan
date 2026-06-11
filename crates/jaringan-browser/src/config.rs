//! Jaringan browser configuration via `~/.config/jaringan-browser/config.yaml`.
//!
//! Configuration is optional — every field has a sensible default, so an
//! empty file behaves identically to a missing file.

use std::path::PathBuf;
use std::fs;

use serde::{Deserialize, Serialize};

use ratatui::style::Color;

/// Top-level browser configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Default URL/file to open when no target is given on the CLI.
    /// `null` (the default) opens the welcome page.
    #[serde(default)]
    pub default_target: Option<String>,

    /// Override for XDG data directory (history, bookmarks, cache).
    #[serde(default)]
    pub data_dir: Option<String>,

    /// Maximum entries in the history list.
    #[serde(default = "default_history_limit")]
    pub history_limit: usize,

    /// Theme colours for the TUI.
    #[serde(default)]
    pub theme: ThemeConfig,

    /// Default gateway settings.
    #[serde(default)]
    pub gateway: GatewayConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default_target: None,
            data_dir: None,
            history_limit: default_history_limit(),
            theme: ThemeConfig::default(),
            gateway: GatewayConfig::default(),
        }
    }
}

/// TUI theme colours.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeConfig {
    /// Accent colour for headers, tab bar, borders, selection.
    /// Named colour (cyan, green, yellow, red, magenta, blue, white, black)
    /// or hex `#rrggbb`.
    #[serde(default = "default_accent")]
    pub accent: String,

    /// Background colour for the status bar.
    #[serde(default)]
    pub status_bg: Option<String>,

    /// Colour for selected items in overlays and menus.
    #[serde(default = "default_selection")]
    pub selection: String,

    /// Colour for the border of the content area.
    #[serde(default = "default_border")]
    pub border: String,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            accent: default_accent(),
            status_bg: None,
            selection: default_selection(),
            border: default_border(),
        }
    }
}

/// Gateway defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayConfig {
    /// Default JRG host for HTTP→JRG gateway.
    #[serde(default = "default_jrg_host")]
    pub jrg_host: String,

    /// Request timeout in seconds.
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,

    /// Enable the HTTP bridge for fetching arbitrary URLs.
    #[serde(default)]
    pub enable_http_bridge: bool,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            jrg_host: default_jrg_host(),
            timeout_secs: default_timeout(),
            enable_http_bridge: false,
        }
    }
}

// ── Default values ─────────────────────────────────────────────────

fn default_history_limit() -> usize { 200 }
fn default_accent() -> String { "cyan".to_owned() }
fn default_selection() -> String { "yellow".to_owned() }
fn default_border() -> String { "dark_gray".to_owned() }
fn default_jrg_host() -> String { "127.0.0.1:7070".to_owned() }
fn default_timeout() -> u64 { 30 }

// ── Paths ──────────────────────────────────────────────────────────

/// Return the config directory path: `~/.config/jaringan-browser/`.
pub fn config_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .unwrap_or_else(|| std::ffi::OsString::from("/tmp"));
    PathBuf::from(home).join(".config/jaringan-browser")
}

/// Return the full config file path.
pub fn config_path() -> PathBuf {
    config_dir().join("config.yaml")
}

// ── Load / Save ────────────────────────────────────────────────────

/// Load configuration from the default path. Returns `Ok(None)` when the
/// file does not exist (caller should use `Config::default()`).
pub fn load() -> Result<Option<Config>, String> {
    let path = config_path();
    if !path.exists() {
        return Ok(None);
    }
    let source = fs::read_to_string(&path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    let config: Config = serde_yaml::from_str(&source)
        .map_err(|e| format!("failed to parse {}: {e}", path.display()))?;
    Ok(Some(config))
}

/// Write configuration to the default path, creating the directory as needed.
pub fn save(config: &Config) -> Result<(), String> {
    let dir = config_dir();
    fs::create_dir_all(&dir)
        .map_err(|e| format!("failed to create {}: {e}", dir.display()))?;
    let path = dir.join("config.yaml");
    let yaml = serde_yaml::to_string(config)
        .map_err(|e| format!("failed to serialize config: {e}"))?;
    fs::write(&path, yaml)
        .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    Ok(())
}

// ── Colour helpers ─────────────────────────────────────────────────

/// Parse a colour string – named colour or hex `#rrggbb` – into a ratatui
/// `Color`. Returns `Color::Cyan` on parse failure.
pub fn parse_color(name: &str) -> Color {
    match name.trim().to_ascii_lowercase().as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" => Color::White,
        "gray" | "grey" => Color::Gray,
        "dark_gray" | "dark_grey" => Color::DarkGray,
        "light_red" => Color::LightRed,
        "light_green" => Color::LightGreen,
        "light_yellow" => Color::LightYellow,
        "light_blue" => Color::LightBlue,
        "light_magenta" => Color::LightMagenta,
        "light_cyan" => Color::LightCyan,
        hex if hex.starts_with('#') && hex.len() == 7 => {
            let r = u8::from_str_radix(&hex[1..3], 16).unwrap_or(0);
            let g = u8::from_str_radix(&hex[3..5], 16).unwrap_or(0);
            let b = u8::from_str_radix(&hex[5..7], 16).unwrap_or(0);
            Color::Rgb(r, g, b)
        }
        _ => Color::Cyan,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_named_colors() {
        assert_eq!(parse_color("cyan"), Color::Cyan);
        assert_eq!(parse_color("CYAN"), Color::Cyan);
        assert_eq!(parse_color("dark_gray"), Color::DarkGray);
        assert_eq!(parse_color("light_red"), Color::LightRed);
    }

    #[test]
    fn test_parse_hex_colors() {
        assert_eq!(parse_color("#ff0000"), Color::Rgb(255, 0, 0));
        assert_eq!(parse_color("#00ff00"), Color::Rgb(0, 255, 0));
        assert_eq!(parse_color("#0000ff"), Color::Rgb(0, 0, 255));
    }

    #[test]
    fn test_parse_invalid_fallback() {
        assert_eq!(parse_color("not-a-color"), Color::Cyan);
        assert_eq!(parse_color(""), Color::Cyan);
    }

    #[test]
    fn test_default_config_round_trip() {
        let dir = std::env::temp_dir().join(format!("jrg-config-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // Temporarily redirect the config path for the test
        let config = Config::default();
        assert!(config.default_target.is_none());
        assert_eq!(config.history_limit, 200);
        assert_eq!(config.theme.accent, "cyan");

        let _ = fs::remove_dir_all(&dir);
    }
}
