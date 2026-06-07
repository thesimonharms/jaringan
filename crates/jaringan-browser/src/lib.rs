use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageLocation {
    File(PathBuf),
    Unsupported(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserState {
    pub current: PageLocation,
    pub selected: usize,
    pub back_stack: Vec<PageLocation>,
    pub forward_stack: Vec<PageLocation>,
    pub status: String,
}

impl BrowserState {
    pub fn new(current: PageLocation) -> Self {
        Self {
            current,
            selected: 0,
            back_stack: Vec::new(),
            forward_stack: Vec::new(),
            status: String::from("Ready"),
        }
    }
}

pub fn resolve_target(current_file: &Path, target: &str) -> PageLocation {
    if looks_like_url(target) {
        return PageLocation::Unsupported(target.to_owned());
    }

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
    state.status = String::from("Back");
    true
}

fn looks_like_url(target: &str) -> bool {
    target.contains("://") || target.starts_with("mailto:")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_jrg_links_against_current_file() {
        let current = Path::new("/tmp/site/index.jrg");

        assert_eq!(
            resolve_target(current, "about/team.jrg"),
            PageLocation::File(PathBuf::from("/tmp/site/about/team.jrg"))
        );
    }

    #[test]
    fn treats_jrg_protocol_links_as_unsupported_until_network_transport_exists() {
        let current = Path::new("/tmp/site/index.jrg");

        assert_eq!(
            resolve_target(current, "jrg://example.org/home"),
            PageLocation::Unsupported("jrg://example.org/home".into())
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
}
