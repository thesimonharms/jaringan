use jaringan_core::{Block, Document};
use jaringan_protocol::{JaringanUrl, fetch_tcp};

/// Manages state across multiple interactive commands within a session.
pub struct SessionState {
    pub url: String,
    pub blocks: Vec<Block>,
    pub metadata: Option<String>,
}

impl SessionState {
    /// Start a new session by fetching the given URL and parsing it.
    pub fn start(url: &str) -> Result<Self, String> {
        let parsed = JaringanUrl::parse(url).map_err(|e| format!("Invalid URL: {e}"))?;
        let response = fetch_tcp(&parsed).map_err(|e| format!("Fetch failed: {e}"))?;
        let doc = jaringan_core::parse_document(&response.body)
            .map_err(|e| format!("Parse failed: {e}"))?;
        Ok(SessionState {
            url: url.to_string(),
            blocks: doc.blocks,
            metadata: doc.metadata,
        })
    }

    /// Re-fetch the current URL and update state.
    pub fn refresh(&mut self) -> Result<(), String> {
        let parsed = JaringanUrl::parse(&self.url).map_err(|e| format!("Invalid URL: {e}"))?;
        let response = fetch_tcp(&parsed).map_err(|e| format!("Fetch failed: {e}"))?;
        let doc = jaringan_core::parse_document(&response.body)
            .map_err(|e| format!("Parse failed: {e}"))?;
        self.blocks = doc.blocks;
        self.metadata = doc.metadata;
        Ok(())
    }

    /// Get the document for the current state.
    pub fn document(&self) -> Document {
        Document::with_metadata(self.blocks.clone(), self.metadata.clone())
    }
}