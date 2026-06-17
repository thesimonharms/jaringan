//! AI-powered features using the baochuan crate.
//!
//! Provides methods for summarisation, Q&A, and semantic find using the
//! configured AI provider.

use std::time::Duration;

use baochuan::{
    providers::{
        AnthropicProvider, DeepSeekProvider, GeminiProvider, GrokProvider,
        MistralProvider, OpenAIProvider, OpenRouterProvider,
    },
    ChatMessage, ChatRequestBuilder, Provider as BaochuanProvider,
};

use crate::config::AiConfig;

/// An AI client configured from `AiConfig`.
#[derive(Debug, Clone)]
pub struct AiClient {
    provider: String,
    model: String,
    api_key: String,
    timeout: Duration,
    summary_prompt: String,
    ask_prompt: String,
}

impl AiClient {
    /// Construct an `AiClient` from config. Returns `None` if the API key env
    /// var is not set or empty.
    pub fn from_config(config: &AiConfig) -> Option<Self> {
        let api_key = std::env::var(&config.api_key_env).unwrap_or_default();
        if api_key.is_empty() {
            return None;
        }
        Some(Self {
            provider: config.provider.clone(),
            model: config.model.clone(),
            api_key,
            timeout: Duration::from_secs(config.timeout_secs),
            summary_prompt: config.summary_prompt.clone(),
            ask_prompt: config.ask_prompt.clone(),
        })
    }

    /// Create the appropriate baochuan provider from the provider name string.
    fn create_provider(&self) -> Result<Box<dyn BaochuanProvider>, String> {
        let p: Box<dyn BaochuanProvider> = match self.provider.as_str() {
            "openai" => Box::new(OpenAIProvider::new(&self.api_key)),
            "anthropic" => Box::new(AnthropicProvider::new(&self.api_key)),
            "deepseek" => Box::new(DeepSeekProvider::new(&self.api_key)),
            "openrouter" => Box::new(OpenRouterProvider::new(&self.api_key)),
            "gemini" => Box::new(GeminiProvider::new(&self.api_key)),
            "grok" | "xai" => Box::new(GrokProvider::new(&self.api_key)),
            "mistral" => Box::new(MistralProvider::new(&self.api_key)),
            other => return Err(format!("unsupported AI provider: {other}")),
        };
        Ok(p)
    }

    /// Send a chat request and return the response text.
    async fn chat(&self, system: &str, user: &str) -> Result<String, String> {
        let provider = self.create_provider()?;
        let request = ChatRequestBuilder::new(&self.model)
            .message(ChatMessage::system(system))
            .message(ChatMessage::user(user))
            .max_tokens(1024)
            .build()
            .map_err(|e| format!("failed to build request: {e}"))?;

        let response = tokio::time::timeout(self.timeout, provider.chat(&request))
            .await
            .map_err(|_| "AI request timed out".to_owned())?
            .map_err(|e| format!("AI request failed: {e}"))?;

        Ok(response.content().unwrap_or("(no response)").to_owned())
    }

    /// Summarise the given page text.
    pub async fn summarize(&self, page_text: &str) -> Result<String, String> {
        let truncated = Self::truncate(page_text, 80_000);
        self.chat(&self.summary_prompt, &truncated).await
    }

    /// Ask a question about the page content.
    pub async fn ask(&self, page_text: &str, question: &str) -> Result<String, String> {
        let truncated = Self::truncate(page_text, 80_000);
        let user = format!(
            "Page content:\n\n{truncated}\n\n---\n\nQuestion: {question}"
        );
        self.chat(&self.ask_prompt, &user).await
    }

    /// Semantic find — find passages matching a query.
    pub async fn semantic_find(&self, page_text: &str, query: &str) -> Result<Vec<String>, String> {
        let truncated = Self::truncate(page_text, 80_000);
        let system = "You are a semantic search assistant. Given a page and a query, return the most relevant passages from the page that answer or relate to the query. Return each passage on a separate line. If nothing is relevant, return 'No relevant passages found.'";
        let user = format!(
            "Page content:\n\n{truncated}\n\n---\n\nSearch query: {query}\n\nRelevant passages:"
        );
        let response = self.chat(system, &user).await?;
        let lines: Vec<String> = response
            .lines()
            .map(|l| l.trim().to_owned())
            .filter(|l| !l.is_empty())
            .collect();
        Ok(lines)
    }

    /// Suggest tags for a bookmarked page.
    pub async fn suggest_tags(&self, page_text: &str) -> Result<String, String> {
        let truncated = Self::truncate(page_text, 20_000);
        let system = "You are a bookmark tagging assistant. Suggest 3-5 short, relevant tags for this page. Return them as a comma-separated list.";
        self.chat(system, &truncated).await
    }

    /// Suggest which open tabs to close based on content.
    pub async fn tab_suggestions(&self, page_texts: &[&str]) -> Result<String, String> {
        let combined: String = page_texts.iter().enumerate().map(|(i, t)| {
            let truncated = Self::truncate(t, 10_000);
            format!("Tab {}:\n{truncated}\n\n---\n", i + 1)
        }).collect();
        let system = "You are a tab management assistant. Review the content of each open tab and suggest which ones could be closed (stale, redundant, or less important). Explain your reasoning briefly.";
        self.chat(system, &combined).await
    }

    /// Truncate text to approximately `max_chars` (character-based, not byte).
    fn truncate(text: &str, max_chars: usize) -> String {
        let chars: Vec<char> = text.chars().collect();
        if chars.len() <= max_chars {
            text.to_owned()
        } else {
            chars[..max_chars].iter().collect::<String>()
                + "\n\n[... content truncated ...]"
        }
    }
}
