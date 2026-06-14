//! JRG Search Engine — JRG-native search with DNS-verified submissions.

pub mod engine;
pub mod pages;

pub use engine::{validate_entry, verify_page_signature, SearchEngine};
