# AI Features + Keybinding Chords Implementation Plan

> **Execution:** Implement wave-by-wave, commit each wave before starting the next.

**Goal:** Add baochuan-powered AI features to Jaringan Browser using a chord-style keybinding system (leader key `A` + second key) to prevent accidental triggers.

**Architecture:** (1) Introduce a chord mode state in `BrowserState`. The leader key enters chord mode; the next key press matches against chord sub-bindings. (2) Add `AiConfig` to the YAML config. (3) Add baochuan as a path dependency. (4) Each AI action calls baochuan asynchronously and displays results in an overlay or status line.

**Tech Stack:** Rust, baochuan (path dep `~/baochuan`), tokio (already present), crossterm, serde_yaml (already present).

---

## Wave 0 — Chord Mode Infrastructure

**Objective:** Add a chord-mode state machine so pressing the leader key (`A`) enters a "waiting for second key" mode, with a status bar indicator.

### Files
- Modify: `crates/jaringan-browser/src/lib.rs` — add `ChordMode` enum, add `chord_mode` field to `BrowserState`
- Modify: `crates/jaringan-browser/src/config.rs` — add `chord_leader`, `chord_ai_summarize`, `chord_ai_ask`, `chord_ai_find`, `chord_ai_tag_bookmark`, `chord_ai_tab_suggest` fields to `Keybindings`
- Modify: `crates/jaringan-browser/src/main.rs` — detect leader key and second key in `handle_key_event`, set status indicator
- Modify: `crates/jaringan-browser/src/main.rs` — render chord indicator in status bar

### What to implement

**lib.rs:**
- Add `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum ChordMode { None, AwaitingAi }`
- Add `pub chord_mode: ChordMode` field to `BrowserState`
- Initialize as `ChordMode::None` in `BrowserState::new`

**config.rs:**
- Add to `Keybindings`:
  - `#[serde(default = "default_kb_chord_leader")] pub chord_leader: String`
  - `#[serde(default = "default_kb_chord_ai_summarize")] pub chord_ai_summarize: String`
  - `#[serde(default = "default_kb_chord_ai_ask")] pub chord_ai_ask: String`
  - `#[serde(default = "default_kb_chord_ai_find")] pub chord_ai_find: String`
  - `#[serde(default = "default_kb_chord_ai_tag_bookmark")] pub chord_ai_tag_bookmark: String`
  - `#[serde(default = "default_kb_chord_ai_tab_suggest")] pub chord_ai_tab_suggest: String`
- Default functions: `fn default_kb_chord_leader() -> String { default_kb("A") }`, similarly for sub-bindings with `"s"`, `"a"`, `"f"`, `"t"`, `"T"`

**main.rs** — in `handle_key_event`, at the TOP (before tab switching):
1. If `state.chord_mode != ChordMode::None`, the next key press handles the chord action (match against chord sub-bindings), then resets chord_mode to None and returns.
   - For now, match each sub-binding and write to `state.status` like `"AI: Summarize (not yet implemented)"` — these are stubs.
2. If `state.chord_mode == ChordMode::None`, check if the current key matches `chord_leader`. If so, set `state.chord_mode = ChordMode::AwaitingAi`, set `state.status = "[A] » Waiting for action…"`, and return.

**Status bar (main.rs render area):**
- Where `&state.status` is rendered, if `state.chord_mode != ChordMode::None`, prefix the status text with `"[A] "` style indicator. The status field already says "Waiting for action…", so the prefix is for visual reinforcement.

### Verification
1. `cargo check` passes in `crates/jaringan-browser`
2. Start the browser, press `A` → status shows chord indicator
3. Press `A s` → status shows "AI: Summarize stub"
4. Press `A a` → status shows "AI: Ask stub"

---

## Wave 1 — AI Config + Baochuan Dependency

**Objective:** Add AI provider configuration and wire baochuan as a dependency.

### Files
- Modify: `crates/jaringan-browser/Cargo.toml` — add baochuan path dep
- Modify: `crates/jaringan-browser/src/config.rs` — add `AiConfig` struct
- Create: `crates/jaringan-browser/src/ai.rs`

### What to implement

**Cargo.toml:**
```toml
baochuan = { path = "../../../baochuan" }
```

**config.rs:**
- Add `AiConfig` struct with fields:
  - `provider: String` (default: `"openai"`) — e.g. openai, anthropic, deepseek, openrouter
  - `model: String` (default: `"gpt-4o-mini"`)
  - `api_key_env: String` (default: `"OPENAI_API_KEY"`) — env var name to read at runtime
  - `timeout_secs: u64` (default: 30)
  - `summary_prompt: String` (default: `"Summarize this page in 3-5 concise bullet points."`)
- Add `#[serde(default)] pub ai: AiConfig` field to `Config`

**ai.rs:**
- Module with `#[derive(Debug, Clone)] pub struct AiClient { provider: String, model: String, api_key: String, timeout: Duration }`
- Constructor: `pub fn from_config(config: &AiConfig) -> Option<Self>` — reads `api_key_env` from env, returns None if missing
- Stub methods (return `Err("not implemented")` for now):
  - `pub async fn summarize(&self, page_text: &str) -> Result<String, String>`
  - `pub async fn ask(&self, page_text: &str, question: &str) -> Result<String, String>`
  - `pub async fn semantic_find(&self, page_text: &str, query: &str) -> Result<Vec<String>, String>`
- Have `from_config` construct a baochuan provider based on `self.provider` string
  - Use match on provider name: `"openai" => OpenAIProvider::new(api_key)`, `"anthropic" => AnthropicProvider::new(api_key)`, etc.
- Re-export from lib.rs: `pub mod ai;`

### Verification
1. `cargo check` passes
2. `AiClient::from_config` reads env var correctly

---

## Wave 2 — AI Page Summarisation

**Objective:** Wire `A s` to actually call baochuan and display a summary.

### Files
- Modify: `crates/jaringan-browser/src/ai.rs` — implement `summarize()`
- Modify: `crates/jaringan-browser/src/main.rs` — wire `A s` to call summarise
- Modify: `crates/jaringan-browser/src/lib.rs` — add `Overlay::AiResult` variant

### What to implement

**ai.rs:**
- In `summarize()`: construct a baochuan `ChatRequestBuilder` with the system prompt and user message containing the page text (truncate to ~100k chars), send, return the response content

**lib.rs:**
- Add `AiResult(String)` to the `Overlay` enum — stores the AI response text
- Add `ai_result: String` field to `BrowserState` (or just use overlay content)
- Add `ai_question_buffer: String` field for ask-mode prompt input

**main.rs:**
- When `A s` is pressed in chord mode: gather rendered page text from `page.items`, call `ai_client.summarize()`, show result in an `Overlay::AiResult` overlay
- Wrap the AI call in a tokio `spawn_blocking` or direct async — the event loop is already tokio-based, so just await
- In the render function, add a case for `Overlay::AiResult` that shows the stored string in a scrollable overlay

### Verification
1. `cargo check` passes
2. With an env var set, `A s` shows a summary of the current page

---

## Wave 3 — Ask About Page / Semantic Find / Bookmark Tagging

**Objective:** Wire `A a` (ask), `A f` (semantic find), `A t` (tag bookmark), `A T` (tab suggestions).

### Files
- Modify: `crates/jaringan-browser/src/ai.rs` — implement remaining methods
- Modify: `crates/jaringan-browser/src/main.rs` — wire chord handlers
- Modify: `crates/jaringan-browser/src/lib.rs` — add overlay variants as needed

### What to implement

**A a (Ask about page):**
- Enters a mode similar to GoTo: `state.overlay = Some(Overlay::AiAsk)`
- User types a question into `state.ai_question_buffer`, presses Enter
- Calls `ai_client.ask()` with page text + question
- Result shown in `Overlay::AiResult`

**A f (Semantic find):**
- Similar to Ask: prompt for query, call `ai_client.semantic_find()`, show matching passages

**A t (Tag bookmark):**
- If current page is bookmarked, call ai to suggest tags
- Display tags in status line
- Save tags to bookmark entry

**A T (Tab suggestions):**
- Cycle through all open tabs, call AI to suggest which to close
- Show suggestions in overlay

### Verification
1. `cargo check` passes
2. Each chord action works end-to-end with a real API call
