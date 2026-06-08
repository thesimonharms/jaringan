use std::path::{Path, PathBuf};

use jaringan_protocol::JaringanUrl;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageLocation {
    File(PathBuf),
    Network(JaringanUrl),
    Unsupported(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserMode {
    Selection,
    Scroll,
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
    pub show_help: bool,
}

impl BrowserState {
    pub fn new(current: PageLocation) -> Self {
        Self {
            current,
            mode: BrowserMode::Selection,
            selected: 0,
            scroll_offset: 0,
            back_stack: Vec::new(),
            forward_stack: Vec::new(),
            status: String::from("Ready"),
            pending_confirmation: None,
            show_help: false,
        }
    }
}

pub fn resolve_target(current: &PageLocation, target: &str) -> PageLocation {
    if target.starts_with("jrg://") {
        return JaringanUrl::parse(target)
            .map(PageLocation::Network)
            .unwrap_or_else(|_| PageLocation::Unsupported(target.to_owned()));
    }

    if looks_like_url(target) {
        return PageLocation::Unsupported(target.to_owned());
    }

    match current {
        PageLocation::File(current_file) => resolve_file_target(current_file, target),
        PageLocation::Network(base) => base
            .resolve(target)
            .map(PageLocation::Network)
            .unwrap_or_else(|_| PageLocation::Unsupported(target.to_owned())),
        PageLocation::Unsupported(_) => PageLocation::Unsupported(target.to_owned()),
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

pub fn toggle_help(state: &mut BrowserState) {
    state.show_help = !state.show_help;
    state.status = if state.show_help {
        String::from("Help open")
    } else {
        String::from("Help closed")
    };
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

fn looks_like_url(target: &str) -> bool {
    target.contains("://") || target.starts_with("mailto:")
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
    fn records_back_history_and_returns_to_previous_page() {
        let home = PageLocation::File(PathBuf::from("/tmp/site/index.jrg"));
        let about = PageLocation::File(PathBuf::from("/tmp/site/about.jrg"));
        let mut state = BrowserState::new(home.clone());

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
        let mut state = BrowserState::new(home.clone());

        navigate_to(&mut state, about.clone());
        assert!(go_back(&mut state));
        assert_eq!(state.current, home);
        assert_eq!(state.forward_stack, vec![about.clone()]);

        assert!(go_forward(&mut state));
        assert_eq!(state.current, about);
        assert_eq!(state.back_stack, vec![home]);
        assert!(state.forward_stack.is_empty());
    }

    #[test]
    fn page_scrolling_and_home_end_clamp_to_bounds() {
        let mut state = BrowserState::new(PageLocation::File(PathBuf::from("/tmp/site/index.jrg")));

        scroll_page_down(&mut state, 100, 20);
        assert_eq!(state.scroll_offset, 20);
        scroll_page_down(&mut state, 100, 20);
        assert_eq!(state.scroll_offset, 40);
        scroll_to_bottom(&mut state, 45, 20);
        assert_eq!(state.scroll_offset, 25);
        scroll_page_down(&mut state, 45, 20);
        assert_eq!(state.scroll_offset, 25);
        scroll_page_up(&mut state, 45, 20);
        assert_eq!(state.scroll_offset, 5);
        scroll_to_top(&mut state);
        assert_eq!(state.scroll_offset, 0);
    }

    #[test]
    fn selection_shortcuts_jump_to_first_and_last_items() {
        let mut state = BrowserState::new(PageLocation::File(PathBuf::from("/tmp/site/index.jrg")));
        state.selected = 2;

        selection_first(&mut state);
        assert_eq!(state.selected, 0);
        selection_last(&mut state, 5);
        assert_eq!(state.selected, 4);
        selection_last(&mut state, 0);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn help_overlay_toggle_updates_state_and_status() {
        let mut state = BrowserState::new(PageLocation::File(PathBuf::from("/tmp/site/index.jrg")));

        assert!(!state.show_help);
        toggle_help(&mut state);
        assert!(state.show_help);
        assert_eq!(state.status, "Help open");
        toggle_help(&mut state);
        assert!(!state.show_help);
        assert_eq!(state.status, "Help closed");
    }

    #[test]
    fn creates_safe_cache_names_for_remote_images() {
        assert_eq!(
            cache_filename_for_url("https://example.com/assets/cover image.png?size=large"),
            "https_example_com_assets_cover_image_png_size_large"
        );
    }

    #[test]
    fn starts_in_selection_mode_and_toggles_to_scroll_mode() {
        let mut state = BrowserState::new(PageLocation::File(PathBuf::from("/tmp/site/index.jrg")));

        assert_eq!(state.mode, BrowserMode::Selection);
        toggle_mode(&mut state);
        assert_eq!(state.mode, BrowserMode::Scroll);
        toggle_mode(&mut state);
        assert_eq!(state.mode, BrowserMode::Selection);
    }

    #[test]
    fn selection_mode_movement_changes_selected_item_only() {
        let mut state = BrowserState::new(PageLocation::File(PathBuf::from("/tmp/site/index.jrg")));

        selection_down(&mut state, 3);
        selection_down(&mut state, 3);
        selection_down(&mut state, 3);
        assert_eq!(state.selected, 2);
        assert_eq!(state.scroll_offset, 0);
        selection_up(&mut state);
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn scroll_mode_movement_changes_scroll_offset_only() {
        let mut state = BrowserState::new(PageLocation::File(PathBuf::from("/tmp/site/index.jrg")));
        switch_mode(&mut state, BrowserMode::Scroll);

        scroll_down(&mut state, 20, 5);
        scroll_down(&mut state, 20, 5);
        assert_eq!(state.scroll_offset, 2);
        assert_eq!(state.selected, 0);
        scroll_up(&mut state);
        assert_eq!(state.scroll_offset, 1);
    }
}
