# Browser Ergonomics Wave 2 Plan

**Goal:** Add 10 UI/UX features to the `jaringan-browser` TUI across 6 waves, each self-contained and testable.

**Architecture:** Each feature builds on the existing ratatui/crossterm event loop, `BrowserState`, `LoadedPage`, tab infrastructure, and render pipeline. No refactoring of core types needed.

---

## Wave 1: Text Selection + Copy, Mouse Support

### Feature: Text Selection Mode

Add a **selection mode** (separate from existing `BrowserMode::Selection` which selects interactive items) that lets the user highlight and copy arbitrary text from the rendered page content.

**State additions (`BrowserState`):**
```rust
pub text_select_active: bool,
pub text_select_start: (u16, u16),   // (line, col)
pub text_select_end: (u16, u16),
```

File: `crates/jaringan-browser/src/lib.rs` — add fields to `BrowserState` struct.

**Keybinding (`handle_key_event`):**
- `Ctrl+Space` — toggle text selection mode on/off
- Arrow keys (`j`/`k`/`h`/`l`) move the cursor when text select is active
- `y` or `Enter` — copy selection to clipboard via `copy_to_clipboard`
- `Esc` — cancel text selection

File: `crates/jaringan-browser/src/main.rs` — new case in main keybinding match.

**Rendering (`draw_frame` / `render_lines`):**
- When `text_select_active`, render a cursor highlight at the selection region using reverse-video or a background color on the selected span of characters
- Update `draw_frame` to re-render the selection overlay each frame

**Clipboard integration:**
Update `copy_to_clipboard` (already exists) to accept arbitrary text, not just URLs.

### Feature: Mouse Support

Enable crossterm mouse capture on startup, dispatch mouse events to link/button activation, scrolling, and tab switching.

**Startup (`run_tui`):**
```rust
execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
```

**Shutdown:**
```rust
execute!(terminal.backend_mut(), DisableMouseCapture, LeaveAlternateScreen)?;
```

**Event handling (`run_app` loop):**
Add `Event::Mouse(mouse)` handler:
- `MouseEventKind::Down(Left)` on a link/button → activate it (find which rendered line the Y coordinate maps to)
- `MouseEventKind::ScrollDown` → scroll down
- `MouseEventKind::ScrollUp` → scroll up
- Click on tab bar area → switch to that tab

**Coordinate mapping:**
The renderer already produces a list of `Line` objects with associated `item_index`. Store the screen Y ↔ item mapping alongside rendered lines so a mouse click on Y=10 can find which interactive item (if any) is at that position.

**Config gate (add to `Config`):**
```rust
#[serde(default = "default_mouse")]
pub enable_mouse: bool,
```
Default: `true` (mouse is harmless to disable; keyboard still works).

File: `crates/jaringan-browser/src/config.rs` — add field.

---

## Wave 2: Open Link in New Tab, Tab Reordering

### Feature: Open Link in New Tab

Allow opening a link in a new background tab without navigating away from the current page.

**Keybinding:** `Ctrl+Enter` when a link/button is selected.

**Implementation:** In `handle_key_event` / `activate_selected`, when `Ctrl+Enter` is detected:
1. Clone the current tab's state creation logic (reuse `load_location`)
2. Push the new `Tab` to `tabs`
3. Keep `active_tab` unchanged (background open)
4. Set status to `"Opened {label} in new tab"`

File: `crates/jaringan-browser/src/main.rs` — new keybinding in `handle_key_event`, before the main `Enter` handler.

### Feature: Tab Reordering

Allow moving tabs left/right in the tab bar.

**Keybinding:**
- `Ctrl+Shift+Left` — move current tab left
- `Ctrl+Shift+Right` — move current tab right

**Implementation:**
```rust
KeyCode::Left if ctrl && alt => {
    // Actually use Ctrl+Shift, which is already in the modifier system
    if *active_tab > 0 {
        tabs.swap(*active_tab, *active_tab - 1);
        *active_tab -= 1;
    }
}
KeyCode::Right if ctrl && alt => {
    if *active_tab + 1 < tabs.len() {
        tabs.swap(*active_tab, *active_tab + 1);
        *active_tab += 1;
    }
}
```

Note: Use `Ctrl+Shift+Left/Right` where `ctrl` and `Shift` are both detected. Since the current modifier check uses `key.modifiers.contains(KeyModifiers::CONTROL)`, also check for `KeyModifiers::SHIFT`.

File: `crates/jaringan-browser/src/main.rs` — new cases in tab management keybindings section.

---

## Wave 3: GoTo Input History, URL Completion from History

### Feature: GoTo Input History

Remember previously entered URLs in the `GoTo` overlay and allow cycling through them with arrow keys.

**State additions (`BrowserState`):**
```rust
pub goto_history: Vec<String>,  // previously entered URLs, newest first
pub goto_history_idx: usize,    // current position in history when browsing
```

File: `crates/jaringan-browser/src/lib.rs`.

**Persistence:** Save/load `goto_history` as a JSON list alongside history/bookmarks.

**Keybinding changes (GoTo overlay handler):**
- `Up` / `Ctrl+p` — cycle backward through goto history
- `Down` / `Ctrl+n` — cycle forward

**On Enter (submit URL):** Push the entered URL to `goto_history` (dedup by position — move existing entry to front), cap at 50 entries, save to disk.

### Feature: URL Completion from History

When typing in the GoTo buffer, show a dropdown of matching history entries.

**Implementation:** In the GoTo overlay handler, after every `Char(ch)` append, filter `state.history` and `state.goto_history` for entries containing the buffer text as a prefix or substring.

**Rendering:** Add a completion dropdown below the GoTo input line showing up to 5 matching entries. The user can:
- Continue typing to narrow results
- Press `Tab` to cycle through completions
- Press `Enter` to accept the highlighted completion
- Press `Esc` to dismiss without completing

**State additions:**
```rust
pub goto_completions: Vec<String>,   // filtered completions
pub goto_completion_idx: usize,      // highlighted completion
```

---

## Wave 4: Improved Table Rendering

Replace the current simple table renderer with a richer one that handles alignment, multiline cells, and column width constraints.

**Current (`render_browser_table`):**
- Simple vertical bars, header bold, no alignment awareness
- No multiline cell support

**Improvements:**
1. **Alignment:** Parse column alignment from `|:---|---:|` separator rows (already implemented in `jaringan-render` crate's inline markup parser — pass `alignments: Vec<Alignment>` through `Table` struct)
2. **Multiline cells:** Split cells containing `\n` into multiple rendered rows, repeating the row border between them
3. **Column width clamping:** Cap columns to a max width (e.g., 40 chars) and truncate with `…` to prevent tables from pushing page content off-screen
4. **Alternating row colors:** Subtle background tint on every other row for readability
5. **Better borders:** Use box-drawing characters (`│`, `─`, `├`, `┤`, `┼`, `┌`, `┐`, `└`, `┘`) for a polished look — already partially done but header/body separation is missing

File: `crates/jaringan-browser/src/main.rs` — update `render_browser_table` and helpers.

**`Table` struct in `jaringan-core`:**
Check if `alignments` field exists — if not, add it to `Table` struct in `jaringan-core/src/lib.rs`:
```rust
pub struct Table {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub alignments: Vec<Alignment>,  // NEW
}
```

Update the JRG parser in `jaringan-core` to capture alignments from separator lines (already partially done per the skill doc). Add a test.

---

## Wave 5: Auth Status Indicator

**Feature:** Show whether the current page or service has an associated auth token in the footer/status bar.

**State addition (`BrowserState`):**
```rust
pub has_auth_token: bool,         // set when loading a page that has auth token
pub auth_service_name: Option<String>,  // the service name
```

**Implementation:**
1. After `load_location`, check if the page's document has any `Block::Auth { service, .. }` blocks
2. Look up `lookup_stored_token(service)` to see if a token exists
3. Set `has_auth_token` and `auth_service_name` accordingly
4. In `draw_frame` / `draw_footer`, show a padlock or status indicator:
   - `🔒 <service>` — if token exists
   - `🔓 <service>` — if auth block exists but no token
   - Nothing if no auth block

File: `crates/jaringan-browser/src/main.rs` — update `load_location` return handling and `draw_footer`.

---

## Wave 6: File Download Prompt

**Feature:** When activating a link/button whose resolved target is a non-renderable binary file (or a download endpoint), prompt the user to download and save the file.

**Detection:** In `activate_selected`, check the response `Content-Type` or file extension after `load_location`:
- If content-type is not `text/*`, `application/json`, or `jrg` → treat as download
- If file extension is `.png`, `.jpg`, `.pdf`, `.zip`, etc. → treat as download

**Prompt flow:**
1. Set `state.pending_download` with the URL/content-type/suggested filename
2. Show `"Download <filename>? (y/N)"` in status
3. On `y` → download to `~/Downloads/` or `~/.cache/jaringan/downloads/`, show "Downloaded to <path>"
4. On `n` → cancel

**State additions:**
```rust
pub pending_download: Option<DownloadPrompt>,
```

**Keybinding:** Handle `y`/`n` only when `pending_download` is set.

---

## Verification

After each wave:

```bash
cargo test -p jaringan-browser
cargo build -p jaringan-browser
# Smoke test: start and quit
echo "q" | cargo run -p jaringan-browser -- open
```

After Wave 4 (table rendering):

```bash
cargo test -p jaringan-core  # parser alignment tests
```

After all waves:

```bash
cargo test --workspace 2>&1 | tail -10
cargo build --workspace 2>&1 | tail -5
```
