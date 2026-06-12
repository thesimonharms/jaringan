use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use std::fs;

pub mod config;
pub mod session;

use config::Config;
use jaringan_protocol::JaringanUrl;

/// Jaringan data directory under XDG_DATA_HOME (~/.local/share/jaringan/).
pub fn data_dir() -> PathBuf {
    std::env::var_os("JARINGAN_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let base = std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    let home = std::env::var_os("HOME")
                        .unwrap_or_else(|| std::ffi::OsString::from("/tmp"));
                    PathBuf::from(home).join(".local/share")
                });
            base.join("jaringan")
        })
}

/// Ensure the data directory exists and return its path.
pub fn ensure_data_dir() -> PathBuf {
    let dir = data_dir();
    if let Err(e) = fs::create_dir_all(&dir) {
        eprintln!("[jaringan] warning: failed to create data directory {}: {e}", dir.display());
    }
    dir
}

// ── History ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HistoryEntry {
    pub url: String,
    pub title: String,
    pub visited_at: u64, // unix seconds
}

pub fn history_path() -> PathBuf {
    ensure_data_dir().join("history.json")
}

pub fn save_history(entries: &[HistoryEntry]) {
    if let Ok(json) = serde_json::to_string(entries) {
        if let Err(e) = fs::write(history_path(), json) {
            eprintln!("[jaringan] warning: failed to save history: {e}");
        }
    }
}

pub fn load_history() -> Vec<HistoryEntry> {
    let path = history_path();
    if !path.exists() {
        return Vec::new();
    }
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Append a visit to history, deduping by URL (most recent stays).
pub fn record_history(entries: &mut Vec<HistoryEntry>, url: &str, title: &str) {
    // Remove existing entry for the same URL
    entries.retain(|e| e.url != url);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    entries.push(HistoryEntry {
        url: url.to_owned(),
        title: title.to_owned(),
        visited_at: now,
    });
    // Keep last 200 entries
    if entries.len() > 200 {
        entries.drain(..entries.len() - 200);
    }
    save_history(entries);
}

// ── Bookmarks ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Bookmark {
    pub name: String,
    pub url: String,
    pub created_at: u64,
}

pub fn bookmarks_path() -> PathBuf {
    ensure_data_dir().join("bookmarks.json")
}

pub fn save_bookmarks(entries: &[Bookmark]) {
    if let Ok(json) = serde_json::to_string(entries) {
        if let Err(e) = fs::write(bookmarks_path(), json) {
            eprintln!("[jaringan] warning: failed to save bookmarks: {e}");
        }
    }
}

pub fn load_bookmarks() -> Vec<Bookmark> {
    let path = bookmarks_path();
    if !path.exists() {
        return Vec::new();
    }
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn add_bookmark(entries: &mut Vec<Bookmark>, name: String, url: String) {
    entries.retain(|b| b.url != url);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    entries.push(Bookmark {
        name,
        url,
        created_at: now,
    });
    save_bookmarks(entries);
}

pub fn remove_bookmark(entries: &mut Vec<Bookmark>, url: &str) {
    entries.retain(|b| b.url != url);
    save_bookmarks(entries);
}

// ── Location ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageLocation {
    File(PathBuf),
    Network(JaringanUrl),
    /// A web URL (http:// or https://).
    Web(String),
    Unsupported(String),
}

impl PageLocation {
    pub fn display_url(&self) -> String {
        match self {
            PageLocation::File(path) => path.display().to_string(),
            PageLocation::Network(url) => url.to_string(),
            PageLocation::Web(url) => url.clone(),
            PageLocation::Unsupported(target) => target.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserMode {
    Selection,
    Scroll,
}

/// Overlay panels that can be shown on top of the page content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    Help,
    History,
    Bookmarks,
    Find,
    PageInfo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindState {
    pub query: String,
    pub matches: Vec<usize>,   // line indices of matches
    pub match_idx: usize,      // current match (index into matches)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionConfirmation {
    pub id: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserState {
    pub current: PageLocation,
    pub mode: BrowserMode,
    pub selected: usize,
    pub scroll_offset: u16,
    pub back_stack: Vec<PageLocation>,
    pub forward_stack: Vec<PageLocation>,
    pub status: String,
    pub pending_confirmation: Option<ActionConfirmation>,
    pub overlay: Option<Overlay>,
    pub overlay_selected: usize,
    pub history: Vec<HistoryEntry>,
    pub bookmarks: Vec<Bookmark>,
    pub find_state: FindState,
    pub config: Config,
}

impl BrowserState {
    pub fn new(current: PageLocation, config: Config) -> Self {
        let history = load_history();
        let bookmarks = load_bookmarks();
        Self {
            current,
            mode: BrowserMode::Selection,
            selected: 0,
            scroll_offset: 0,
            back_stack: Vec::new(),
            forward_stack: Vec::new(),
            status: String::from("Ready"),
            pending_confirmation: None,
            overlay: None,
            overlay_selected: 0,
            history,
            bookmarks,
            find_state: FindState {
                query: String::new(),
                matches: Vec::new(),
                match_idx: 0,
            },
            config,
        }
    }

    pub fn record_current(&mut self, title: &str) {
        let url = self.current.display_url();
        record_history(&mut self.history, &url, title);
    }

    pub fn toggle_bookmark_current(&mut self, title: &str) {
        let url = self.current.display_url();
        if self.bookmarks.iter().any(|b| b.url == url) {
            remove_bookmark(&mut self.bookmarks, &url);
            self.status = format!("Bookmark removed: {url}");
        } else {
            add_bookmark(&mut self.bookmarks, title.to_owned(), url.clone());
            self.status = format!("Bookmarked: {title}");
        }
    }
}

// ── URL helpers ──────────────────────────────────────────────────────

/// Convert a web URL to a JrgToHttpResolver-compatible JRG URL.
pub fn web_to_jrg_url(web_url: &str) -> String {
    if let Some(rest) = web_url.strip_prefix("https://") {
        format!("jrg://https.{rest}")
    } else if let Some(rest) = web_url.strip_prefix("http://") {
        format!("jrg://http/{rest}")
    } else {
        format!("jrg://https.{web_url}")
    }
}

pub fn resolve_target(current: &PageLocation, target: &str) -> PageLocation {
    if target.starts_with("jrg://") {
        return JaringanUrl::parse(target)
            .map(PageLocation::Network)
            .unwrap_or_else(|_| PageLocation::Unsupported(target.to_owned()));
    }

    if looks_like_web_url(target) {
        return PageLocation::Web(target.to_owned());
    }

    match current {
        PageLocation::File(current_file) => resolve_file_target(current_file, target),
        PageLocation::Network(base) => base
            .resolve(target)
            .map(PageLocation::Network)
            .unwrap_or_else(|_| PageLocation::Unsupported(target.to_owned())),
        PageLocation::Web(_) | PageLocation::Unsupported(_) => {
            PageLocation::Unsupported(target.to_owned())
        }
    }
}

fn resolve_file_target(current_file: &Path, target: &str) -> PageLocation {
    let path = PathBuf::from(target);
    if path.is_absolute() {
        return PageLocation::File(path);
    }
    let base = current_file.parent().unwrap_or_else(|| Path::new("."));
    PageLocation::File(base.join(path))
}

pub fn navigate_to(state: &mut BrowserState, next: PageLocation) {
    let previous = state.current.clone();
    state.back_stack.push(previous);
    state.forward_stack.clear();
    state.current = next;
    state.selected = 0;
    state.scroll_offset = 0;
    state.overlay = None;
    state.status = String::from("Loaded");
}

pub fn go_back(state: &mut BrowserState) -> bool {
    let Some(previous) = state.back_stack.pop() else {
        state.status = String::from("No back history");
        return false;
    };
    let current = state.current.clone();
    state.forward_stack.push(current);
    state.current = previous;
    state.selected = 0;
    state.scroll_offset = 0;
    state.overlay = None;
    state.status = String::from("Back");
    true
}

pub fn go_forward(state: &mut BrowserState) -> bool {
    let Some(next) = state.forward_stack.pop() else {
        state.status = String::from("No forward history");
        return false;
    };
    let current = state.current.clone();
    state.back_stack.push(current);
    state.current = next;
    state.selected = 0;
    state.scroll_offset = 0;
    state.overlay = None;
    state.status = String::from("Forward");
    true
}

pub fn switch_mode(state: &mut BrowserState, mode: BrowserMode) {
    state.mode = mode;
    state.status = match mode {
        BrowserMode::Selection => String::from("Selection mode"),
        BrowserMode::Scroll => String::from("Scroll mode"),
    };
}

pub fn toggle_mode(state: &mut BrowserState) {
    let next = match state.mode {
        BrowserMode::Selection => BrowserMode::Scroll,
        BrowserMode::Scroll => BrowserMode::Selection,
    };
    switch_mode(state, next);
}

pub fn selection_down(state: &mut BrowserState, item_count: usize) {
    if item_count > 0 {
        state.selected = (state.selected + 1).min(item_count - 1);
    }
}

pub fn selection_up(state: &mut BrowserState) {
    state.selected = state.selected.saturating_sub(1);
}

pub fn selection_first(state: &mut BrowserState) {
    state.selected = 0;
}

pub fn selection_last(state: &mut BrowserState, item_count: usize) {
    state.selected = item_count.saturating_sub(1);
}

pub fn scroll_down(state: &mut BrowserState, line_count: usize, viewport_height: u16) {
    let max_offset = line_count.saturating_sub(viewport_height as usize) as u16;
    state.scroll_offset = (state.scroll_offset + 1).min(max_offset);
}

pub fn scroll_up(state: &mut BrowserState) {
    state.scroll_offset = state.scroll_offset.saturating_sub(1);
}

pub fn scroll_page_down(state: &mut BrowserState, line_count: usize, viewport_height: u16) {
    let max_offset = max_scroll_offset(line_count, viewport_height);
    state.scroll_offset = state
        .scroll_offset
        .saturating_add(viewport_height)
        .min(max_offset);
}

pub fn scroll_page_up(state: &mut BrowserState, _line_count: usize, viewport_height: u16) {
    state.scroll_offset = state.scroll_offset.saturating_sub(viewport_height);
}

pub fn scroll_to_top(state: &mut BrowserState) {
    state.scroll_offset = 0;
}

pub fn scroll_to_bottom(state: &mut BrowserState, line_count: usize, viewport_height: u16) {
    state.scroll_offset = max_scroll_offset(line_count, viewport_height);
}

fn max_scroll_offset(line_count: usize, viewport_height: u16) -> u16 {
    line_count.saturating_sub(viewport_height as usize) as u16
}

pub fn toggle_overlay(state: &mut BrowserState, overlay: Overlay) {
    if state.overlay == Some(overlay) {
        state.overlay = None;
    } else {
        state.overlay = Some(overlay);
        state.overlay_selected = 0;
    }
}

pub fn cache_filename_for_url(url: &str) -> String {
    let mut output = String::new();
    let mut last_was_separator = false;
    for ch in url.chars() {
        if ch.is_ascii_alphanumeric() {
            output.push(ch.to_ascii_lowercase());
            last_was_separator = false;
        } else if !last_was_separator {
            output.push('_');
            last_was_separator = true;
        }
    }
    output.trim_matches('_').to_owned()
}

fn looks_like_web_url(target: &str) -> bool {
    target.starts_with("http://")
        || target.starts_with("https://")
        || target.starts_with("www.")
}

// ── Tab Persistence ────────────────────────────────────────────────────

/// A serializable record of a saved tab for persistence.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SavedTab {
    pub url: String,
    pub title: String,
}

pub fn tabs_path() -> PathBuf {
    ensure_data_dir().join("tabs.json")
}

/// Save open tabs to disk for restoration on the next launch.
pub fn save_tabs(entries: &[SavedTab]) {
    if let Ok(json) = serde_json::to_string(entries) {
        if let Err(e) = fs::write(tabs_path(), json) {
            eprintln!("[jaringan] warning: failed to save tabs: {e}");
        }
    }
}

/// Load previously saved tabs from disk.
pub fn load_tabs() -> Vec<SavedTab> {
    let path = tabs_path();
    if !path.exists() {
        return Vec::new();
    }
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_jrg_links_against_current_file() {
        let current = PageLocation::File(PathBuf::from("/tmp/site/index.jrg"));
        assert_eq!(
            resolve_target(&current, "about/team.jrg"),
            PageLocation::File(PathBuf::from("/tmp/site/about/team.jrg"))
        );
    }

    #[test]
    fn resolves_jrg_protocol_links_as_network_locations() {
        let current =
            PageLocation::Network(JaringanUrl::parse("jrg://example.org/docs/start.jrg").unwrap());
        assert_eq!(
            resolve_target(&current, "guide/intro.jrg?mode=ai#install"),
            PageLocation::Network(
                JaringanUrl::parse("jrg://example.org/docs/guide/intro.jrg?mode=ai#install")
                    .unwrap()
            )
        );
        assert_eq!(
            resolve_target(&current, "jrg://other.example/home.jrg"),
            PageLocation::Network(JaringanUrl::parse("jrg://other.example/home.jrg").unwrap())
        );
    }

    #[test]
    fn resolves_web_urls_as_web_locations() {
        let current = PageLocation::File(PathBuf::from("/tmp/site/index.jrg"));
        assert_eq!(
            resolve_target(&current, "https://example.com/page"),
            PageLocation::Web("https://example.com/page".to_owned())
        );
    }

    #[test]
    fn converts_web_to_jrg_urls() {
        assert_eq!(
            web_to_jrg_url("https://example.com/page"),
            "jrg://https.example.com/page"
        );
        assert_eq!(
            web_to_jrg_url("http://example.com"),
            "jrg://http/example.com"
        );
    }

    #[test]
    fn records_back_history_and_returns_to_previous_page() {
        let home = PageLocation::File(PathBuf::from("/tmp/site/index.jrg"));
        let about = PageLocation::File(PathBuf::from("/tmp/site/about.jrg"));
        let mut state = BrowserState::new(home.clone(), Config::default());
        navigate_to(&mut state, about.clone());
        assert_eq!(state.current, about);
        assert_eq!(state.back_stack, vec![home.clone()]);
        assert!(go_back(&mut state));
        assert_eq!(state.current, home);
    }

    #[test]
    fn records_forward_history_when_going_back_and_forward() {
        let home = PageLocation::File(PathBuf::from("/tmp/site/index.jrg"));
        let about = PageLocation::File(PathBuf::from("/tmp/site/about.jrg"));
        let mut state = BrowserState::new(home.clone(), Config::default());
        navigate_to(&mut state, about.clone());
        assert!(go_back(&mut state));
        assert_eq!(state.current, home);
        assert_eq!(state.forward_stack, vec![about.clone()]);
        assert!(go_forward(&mut state));
        assert_eq!(state.current, about);
        assert_eq!(state.back_stack, vec![home]);
    }

    #[test]
    fn history_and_bookmarks_persistence() {
        let dir = std::env::temp_dir().join(format!("jrg-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        // SAFETY: single-threaded test, no other code reads JARINGAN_DATA
        unsafe { std::env::set_var("JARINGAN_DATA", &dir); }

        let mut entries = Vec::new();
        record_history(&mut entries, "jrg://example.org/page", "Test Page");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].url, "jrg://example.org/page");

        // Reload from disk
        let loaded = load_history();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].title, "Test Page");

        // Bookmark
        let mut bookmarks = Vec::new();
        add_bookmark(&mut bookmarks, "My Page".to_owned(), "jrg://example.org/page".to_owned());
        assert_eq!(bookmarks.len(), 1);

        let loaded_bm = load_bookmarks();
        assert_eq!(loaded_bm.len(), 1);
        assert_eq!(loaded_bm[0].name, "My Page");

        let _ = fs::remove_dir_all(&dir);
        // SAFETY: single-threaded test cleanup
        unsafe { std::env::remove_var("JARINGAN_DATA"); }
    }
}
