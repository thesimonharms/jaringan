//! Jaringan browser configuration via `~/.config/jaringan/browser.yaml`.
//!
//! Configuration is optional — every field has a sensible default, so an
//! empty file behaves identically to a missing file. On first run a default
//! config file is written automatically.

use std::path::PathBuf;
use std::fs;

use serde::{Deserialize, Serialize};

use ratatui::style::Color;
use crossterm::event::{KeyCode, KeyModifiers};

/// A parsed keybinding — the action + its key code and modifiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub key: KeyCode,
    pub modifiers: KeyModifiers,
    pub label: String, // human-readable label e.g. "Ctrl+Shift+R"
}

/// Parse a keybinding string like `"Ctrl+Shift+R"`, `"q"`, `"Esc"`, `"Alt+1"`.
fn parse_binding(s: &str) -> (KeyCode, KeyModifiers, String) {
    let parts: Vec<&str> = s.split('+').collect();
    let mut modifiers = KeyModifiers::empty();
    let mut key_part = "";
    let mut label_parts: Vec<&str> = Vec::new();

    for &part in &parts {
        let lower = part.trim().to_lowercase();
        match lower.as_str() {
            "ctrl" | "control" => { modifiers.insert(KeyModifiers::CONTROL); label_parts.push("Ctrl"); }
            "alt" | "option" => { modifiers.insert(KeyModifiers::ALT); label_parts.push("Alt"); }
            "shift" => { modifiers.insert(KeyModifiers::SHIFT); label_parts.push("Shift"); }
            "super" | "win" | "cmd" => { modifiers.insert(KeyModifiers::SUPER); label_parts.push("Super"); }
            _ => key_part = part,
        }
    }

    let key = match key_part.trim().to_lowercase().as_str() {
        "esc" => KeyCode::Esc,
        "enter" | "return" => KeyCode::Enter,
        "tab" => KeyCode::Tab,
        "backtab" => KeyCode::BackTab,
        "backspace" | "bs" => KeyCode::Backspace,
        "space" | " " => KeyCode::Char(' '),
        "delete" | "del" => KeyCode::Delete,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" | "pgup" => KeyCode::PageUp,
        "pagedown" | "pgdn" | "pgdown" => KeyCode::PageDown,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "insert" | "ins" => KeyCode::Insert,
        "f1" => KeyCode::F(1),
        "f2" => KeyCode::F(2),
        "f3" => KeyCode::F(3),
        "f4" => KeyCode::F(4),
        "f5" => KeyCode::F(5),
        "f6" => KeyCode::F(6),
        "f7" => KeyCode::F(7),
        "f8" => KeyCode::F(8),
        "f9" => KeyCode::F(9),
        "f10" => KeyCode::F(10),
        "f11" => KeyCode::F(11),
        "f12" => KeyCode::F(12),
        _ if key_part.chars().count() == 1 => {
            let c = key_part.chars().next().unwrap_or('?');
            KeyCode::Char(c)
        }
        _ => KeyCode::Char('?'),
    };

    label_parts.push(key_part);
    let label = label_parts.join("+");
    (key, modifiers, label)
}

/// Check if a crossterm KeyEvent matches a binding string.
pub fn key_matches_binding(key: &crossterm::event::KeyEvent, binding_str: &str) -> bool {
    let (binding_key, binding_mod, _) = parse_binding(binding_str);
    key.code == binding_key && key.modifiers == binding_mod
}

/// All user-configurable keybindings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keybindings {
    #[serde(default = "default_kb_quit")] pub quit: String,
    #[serde(default = "default_kb_quit_esc")] pub quit_esc: String,
    #[serde(default = "default_kb_scroll_down")] pub scroll_down: String,
    #[serde(default = "default_kb_scroll_down_alt")] pub scroll_down_alt: String,
    #[serde(default = "default_kb_scroll_up")] pub scroll_up: String,
    #[serde(default = "default_kb_scroll_up_alt")] pub scroll_up_alt: String,
    #[serde(default = "default_kb_page_down")] pub page_down: String,
    #[serde(default = "default_kb_page_down_alt")] pub page_down_alt: String,
    #[serde(default = "default_kb_page_up")] pub page_up: String,
    #[serde(default = "default_kb_selection_mode")] pub selection_mode: String,
    #[serde(default = "default_kb_scroll_mode")] pub scroll_mode: String,
    #[serde(default = "default_kb_help")] pub help: String,
    #[serde(default = "default_kb_help_alt")] pub help_alt: String,
    #[serde(default = "default_kb_history")] pub history: String,
    #[serde(default = "default_kb_bookmarks")] pub bookmarks: String,
    #[serde(default = "default_kb_copy_url")] pub copy_url: String,
    #[serde(default = "default_kb_goto")] pub goto: String,
    #[serde(default = "default_kb_scroll_bottom")] pub scroll_bottom: String,
    #[serde(default = "default_kb_bookmark_toggle")] pub bookmark_toggle: String,
    #[serde(default = "default_kb_page_info")] pub page_info: String,
    #[serde(default = "default_kb_clear_screen")] pub clear_screen: String,
    #[serde(default = "default_kb_back")] pub back: String,
    #[serde(default = "default_kb_back_alt")] pub back_alt: String,
    #[serde(default = "default_kb_forward")] pub forward: String,
    #[serde(default = "default_kb_reload")] pub reload: String,
    #[serde(default = "default_kb_find")] pub find: String,
    #[serde(default = "default_kb_find_next")] pub find_next: String,
    #[serde(default = "default_kb_find_prev")] pub find_prev: String,
    #[serde(default = "default_kb_new_tab")] pub new_tab: String,
    #[serde(default = "default_kb_close_tab")] pub close_tab: String,
    #[serde(default = "default_kb_next_tab")] pub next_tab: String,
    #[serde(default = "default_kb_prev_tab")] pub prev_tab: String,
    #[serde(default = "default_kb_tab_move_left")] pub tab_move_left: String,
    #[serde(default = "default_kb_tab_move_right")] pub tab_move_right: String,
    #[serde(default = "default_kb_open_new_tab")] pub open_new_tab: String,
    #[serde(default = "default_kb_source_view")] pub source_view: String,
    #[serde(default = "default_kb_text_selection")] pub text_selection: String,
    #[serde(default = "default_kb_text_select_exit")] pub text_select_exit: String,
    #[serde(default = "default_kb_plugin_reload")] pub plugin_reload: String,
    #[serde(default = "default_kb_chord_leader")] pub chord_leader: String,
    #[serde(default = "default_kb_chord_ai_summarize")] pub chord_ai_summarize: String,
    #[serde(default = "default_kb_chord_ai_ask")] pub chord_ai_ask: String,
    #[serde(default = "default_kb_chord_ai_find")] pub chord_ai_find: String,
    #[serde(default = "default_kb_chord_ai_tag_bookmark")] pub chord_ai_tag_bookmark: String,
    #[serde(default = "default_kb_chord_ai_tab_suggest")] pub chord_ai_tab_suggest: String,
}

impl Default for Keybindings {
    fn default() -> Self {
        Self {
            quit: default_kb_quit(), quit_esc: default_kb_quit_esc(),
            scroll_down: default_kb_scroll_down(), scroll_down_alt: default_kb_scroll_down_alt(),
            scroll_up: default_kb_scroll_up(), scroll_up_alt: default_kb_scroll_up_alt(),
            page_down: default_kb_page_down(), page_down_alt: default_kb_page_down_alt(),
            page_up: default_kb_page_up(),
            selection_mode: default_kb_selection_mode(),
            scroll_mode: default_kb_scroll_mode(),
            help: default_kb_help(), help_alt: default_kb_help_alt(),
            history: default_kb_history(),
            bookmarks: default_kb_bookmarks(),
            copy_url: default_kb_copy_url(),
            goto: default_kb_goto(),
            scroll_bottom: default_kb_scroll_bottom(),
            bookmark_toggle: default_kb_bookmark_toggle(),
            page_info: default_kb_page_info(),
            clear_screen: default_kb_clear_screen(),
            back: default_kb_back(), back_alt: default_kb_back_alt(),
            forward: default_kb_forward(),
            reload: default_kb_reload(),
            find: default_kb_find(),
            find_next: default_kb_find_next(),
            find_prev: default_kb_find_prev(),
            new_tab: default_kb_new_tab(),
            close_tab: default_kb_close_tab(),
            next_tab: default_kb_next_tab(),
            prev_tab: default_kb_prev_tab(),
            tab_move_left: default_kb_tab_move_left(),
            tab_move_right: default_kb_tab_move_right(),
            open_new_tab: default_kb_open_new_tab(),
            source_view: default_kb_source_view(),
            text_selection: default_kb_text_selection(),
            text_select_exit: default_kb_text_select_exit(),
            plugin_reload: default_kb_plugin_reload(),
            chord_leader: default_kb_chord_leader(),
            chord_ai_summarize: default_kb_chord_ai_summarize(),
            chord_ai_ask: default_kb_chord_ai_ask(),
            chord_ai_find: default_kb_chord_ai_find(),
            chord_ai_tag_bookmark: default_kb_chord_ai_tag_bookmark(),
            chord_ai_tab_suggest: default_kb_chord_ai_tab_suggest(),
        }
    }
}

/// AI provider configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiConfig {
    /// Which baochuan provider to use: "openai", "anthropic", "deepseek",
    /// "openrouter", "gemini", "grok", "mistral", etc.
    #[serde(default = "default_ai_provider")]
    pub provider: String,
    /// Model name to use (e.g. "gpt-4o-mini", "claude-3-haiku")
    #[serde(default = "default_ai_model")]
    pub model: String,
    /// Environment variable name that holds the API key (e.g. "OPENAI_API_KEY")
    #[serde(default = "default_ai_api_key_env")]
    pub api_key_env: String,
    /// Timeout in seconds for AI requests.
    #[serde(default = "default_ai_timeout")]
    pub timeout_secs: u64,
    /// Custom system prompt for page summarisation.
    #[serde(default = "default_ai_summary_prompt")]
    pub summary_prompt: String,
    /// Custom system prompt for ask-about-page queries.
    #[serde(default = "default_ai_ask_prompt")]
    pub ask_prompt: String,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: default_ai_provider(),
            model: default_ai_model(),
            api_key_env: default_ai_api_key_env(),
            timeout_secs: default_ai_timeout(),
            summary_prompt: default_ai_summary_prompt(),
            ask_prompt: default_ai_ask_prompt(),
        }
    }
}

/// Top-level browser configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)] pub default_target: Option<String>,
    #[serde(default)] pub data_dir: Option<String>,
    #[serde(default = "default_history_limit")] pub history_limit: usize,
    #[serde(default)] pub render_images: bool,
    #[serde(default)] pub tab_persistence: bool,
    #[serde(default = "default_live_reload")] pub live_reload: bool,
    #[serde(default = "default_mouse")] pub enable_mouse: bool,
    #[serde(default)] pub theme: ThemeConfig,
    #[serde(default)] pub gateway: GatewayConfig,
    #[serde(default)] pub keybindings: Keybindings,
    #[serde(default)] pub ai: AiConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default_target: None, data_dir: None,
            history_limit: default_history_limit(),
            render_images: false, tab_persistence: false,
            live_reload: true, enable_mouse: true,
            theme: ThemeConfig::default(),
            gateway: GatewayConfig::default(),
            keybindings: Keybindings::default(),
            ai: AiConfig::default(),
        }
    }
}

/// TUI theme colours.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeConfig {
    #[serde(default = "default_accent")] pub accent: String,
    #[serde(default)] pub status_bg: Option<String>,
    #[serde(default = "default_selection")] pub selection: String,
    #[serde(default = "default_border")] pub border: String,
    #[serde(default = "default_find_highlight")] pub find_highlight: String,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            accent: default_accent(), status_bg: None,
            selection: default_selection(),
            border: default_border(),
            find_highlight: default_find_highlight(),
        }
    }
}

/// Gateway defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayConfig {
    #[serde(default = "default_jrg_host")] pub jrg_host: String,
    #[serde(default = "default_timeout")] pub timeout_secs: u64,
    #[serde(default)] pub enable_http_bridge: bool,
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
fn default_find_highlight() -> String { "light_yellow".to_owned() }
fn default_live_reload() -> bool { true }
fn default_mouse() -> bool { true }
fn default_jrg_host() -> String { "127.0.0.1:7070".to_owned() }
fn default_timeout() -> u64 { 30 }

// ── Default keybinding functions ────────────────────────────────────

fn default_kb(s: &str) -> String { s.to_owned() }

fn default_kb_quit() -> String { default_kb("q") }
fn default_kb_quit_esc() -> String { default_kb("Esc") }
fn default_kb_scroll_down() -> String { default_kb("j") }
fn default_kb_scroll_down_alt() -> String { default_kb("Down") }
fn default_kb_scroll_up() -> String { default_kb("k") }
fn default_kb_scroll_up_alt() -> String { default_kb("Up") }
fn default_kb_page_down() -> String { default_kb("Space") }
fn default_kb_page_down_alt() -> String { default_kb("PageDown") }
fn default_kb_page_up() -> String { default_kb("PageUp") }
fn default_kb_selection_mode() -> String { default_kb("v") }
fn default_kb_scroll_mode() -> String { default_kb("s") }
fn default_kb_help() -> String { default_kb("?") }
fn default_kb_help_alt() -> String { default_kb("h") }
fn default_kb_history() -> String { default_kb("H") }
fn default_kb_bookmarks() -> String { default_kb("B") }
fn default_kb_copy_url() -> String { default_kb("y") }
fn default_kb_goto() -> String { default_kb("g") }
fn default_kb_scroll_bottom() -> String { default_kb("G") }
fn default_kb_bookmark_toggle() -> String { default_kb("Ctrl+d") }
fn default_kb_page_info() -> String { default_kb("Ctrl+i") }
fn default_kb_clear_screen() -> String { default_kb("Ctrl+l") }
fn default_kb_back() -> String { default_kb("b") }
fn default_kb_back_alt() -> String { default_kb("Backspace") }
fn default_kb_forward() -> String { default_kb("f") }
fn default_kb_reload() -> String { default_kb("r") }
fn default_kb_find() -> String { default_kb("Ctrl+f") }
fn default_kb_find_next() -> String { default_kb("Ctrl+n") }
fn default_kb_find_prev() -> String { default_kb("Ctrl+p") }
fn default_kb_new_tab() -> String { default_kb("Ctrl+t") }
fn default_kb_close_tab() -> String { default_kb("Ctrl+w") }
fn default_kb_next_tab() -> String { default_kb("Ctrl+Tab") }
fn default_kb_prev_tab() -> String { default_kb("Ctrl+BackTab") }
fn default_kb_tab_move_left() -> String { default_kb("Ctrl+Shift+Left") }
fn default_kb_tab_move_right() -> String { default_kb("Ctrl+Shift+Right") }
fn default_kb_open_new_tab() -> String { default_kb("Ctrl+Enter") }
fn default_kb_source_view() -> String { default_kb("Ctrl+\\") }
fn default_kb_text_selection() -> String { default_kb("V") }
fn default_kb_text_select_exit() -> String { default_kb("Esc") }
fn default_kb_plugin_reload() -> String { default_kb("Ctrl+Shift+R") }
fn default_ai_provider() -> String { "openai".to_owned() }
fn default_ai_model() -> String { "gpt-4o-mini".to_owned() }
fn default_ai_api_key_env() -> String { "OPENAI_API_KEY".to_owned() }
fn default_ai_timeout() -> u64 { 30 }
fn default_ai_summary_prompt() -> String {
    "Summarise this page in 3-5 concise bullet points. Focus on the key facts and main arguments.".to_owned()
}
fn default_ai_ask_prompt() -> String {
    "You are a helpful assistant. Answer the user's question based on the page content provided.".to_owned()
}
fn default_kb_chord_leader() -> String { default_kb("A") }
fn default_kb_chord_ai_summarize() -> String { default_kb("s") }
fn default_kb_chord_ai_ask() -> String { default_kb("a") }
fn default_kb_chord_ai_find() -> String { default_kb("f") }
fn default_kb_chord_ai_tag_bookmark() -> String { default_kb("t") }
fn default_kb_chord_ai_tab_suggest() -> String { default_kb("T") }

// ── Paths ──────────────────────────────────────────────────────────

/// Return the config directory path: `~/.config/jaringan/`.
pub fn config_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .unwrap_or_else(|| std::ffi::OsString::from("/tmp"));
    PathBuf::from(home).join(".config/jaringan")
}

/// Return the full config file path.
pub fn config_path() -> PathBuf {
    config_dir().join("browser.yaml")
}

/// Legacy config path for migration.
fn legacy_config_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .unwrap_or_else(|| std::ffi::OsString::from("/tmp"));
    PathBuf::from(home).join(".config/jaringan-browser/config.yaml")
}

// ── Load / Save / Ensure ────────────────────────────────────────────

/// Load configuration. Falls back to legacy path if new doesn't exist.
pub fn load() -> Result<Option<Config>, String> {
    let path = config_path();
    if path.exists() {
        let source = fs::read_to_string(&path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        let config: Config = serde_yaml::from_str(&source)
            .map_err(|e| format!("failed to parse {}: {e}", path.display()))?;
        return Ok(Some(config));
    }
    let legacy = legacy_config_path();
    if legacy.exists() {
        let source = fs::read_to_string(&legacy)
            .map_err(|e| format!("failed to read {}: {e}", legacy.display()))?;
        let config: Config = serde_yaml::from_str(&source)
            .map_err(|e| format!("failed to parse {}: {e}", legacy.display()))?;
        eprintln!("[jaringan] migrated config from {}", legacy.display());
        let _ = save(&config);
        return Ok(Some(config));
    }
    Ok(None)
}

/// Write configuration to the default path.
pub fn save(config: &Config) -> Result<(), String> {
    let dir = config_dir();
    fs::create_dir_all(&dir)
        .map_err(|e| format!("failed to create {}: {e}", dir.display()))?;
    let path = dir.join("browser.yaml");
    let yaml = serde_yaml::to_string(config)
        .map_err(|e| format!("failed to serialize config: {e}"))?;
    fs::write(&path, yaml)
        .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    Ok(())
}

/// Ensure the default config file exists. Returns the loaded/default config.
/// Call at browser startup.
pub fn ensure_defaults() -> Config {
    let path = config_path();

    match load() {
        Ok(Some(cfg)) => {
            // Config loaded successfully
            return cfg;
        }
        Ok(None) => {
            // No config exists at either path — write defaults
            eprintln!("[jaringan] writing default config to {}", path.display());
            let cfg = Config::default();
            let _ = save(&cfg);
            return cfg;
        }
        Err(e) => {
            // Config exists but is corrupt or unreadable
            eprintln!("[jaringan] WARNING: {e}");
            eprintln!(
                "[jaringan] using default settings; fix or delete {} to suppress this warning",
                path.display()
            );
        }
    }

    Config::default()
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
    fn test_parse_binding_simple_char() {
        let (code, mods, label) = parse_binding("q");
        assert_eq!(code, KeyCode::Char('q'));
        assert_eq!(mods, KeyModifiers::empty());
        assert_eq!(label, "q");
    }

    #[test]
    fn test_parse_binding_ctrl_shift_r() {
        let (code, mods, label) = parse_binding("Ctrl+Shift+R");
        assert_eq!(code, KeyCode::Char('R'));
        assert!(mods.contains(KeyModifiers::CONTROL));
        assert!(mods.contains(KeyModifiers::SHIFT));
        assert_eq!(label, "Ctrl+Shift+R");
    }

    #[test]
    fn test_parse_binding_alt_number() {
        let (code, mods, _label) = parse_binding("Alt+1");
        assert_eq!(code, KeyCode::Char('1'));
        assert!(mods.contains(KeyModifiers::ALT));
    }

    #[test]
    fn test_parse_binding_special_keys() {
        assert_eq!(parse_binding("Esc").0, KeyCode::Esc);
        assert_eq!(parse_binding("Enter").0, KeyCode::Enter);
        assert_eq!(parse_binding("Tab").0, KeyCode::Tab);
        assert_eq!(parse_binding("BackTab").0, KeyCode::BackTab);
        assert_eq!(parse_binding("PageDown").0, KeyCode::PageDown);
        assert_eq!(parse_binding("F5").0, KeyCode::F(5));
    }

    #[test]
    fn test_parse_binding_space() {
        assert_eq!(parse_binding("Space").0, KeyCode::Char(' '));
        assert_eq!(parse_binding(" ").0, KeyCode::Char(' '));
    }

    #[test]
    fn test_key_matches_binding() {
        let event = crossterm::event::KeyEvent::new(KeyCode::Char('R'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
        assert!(key_matches_binding(&event, "Ctrl+Shift+R"));
    }

    #[test]
    fn test_default_config_round_trip() {
        let dir = std::env::temp_dir().join(format!("jrg-config-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let config = Config::default();
        assert!(config.default_target.is_none());
        assert_eq!(config.history_limit, 200);
        assert_eq!(config.theme.accent, "cyan");
        assert_eq!(config.keybindings.quit, "q");
        assert_eq!(config.keybindings.new_tab, "Ctrl+t");

        let _ = fs::remove_dir_all(&dir);
    }
}
