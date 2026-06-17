# Bug Fix Plan — Jaringan Browser

> **For subagents:** Fix the bugs listed in your task. Read the file before editing. After all fixes, run `cargo build -p jaringan-browser` and `cargo test -p jaringan-browser` to verify. Commit with the message given.

---

## Task A: C1 + M7 — Overlay state write-back + Find typing fix

**File:** `crates/jaringan-browser/src/main.rs`

### C1: Missing overlay write-back (CRITICAL)
In `handle_key_event`, the non-GoTo overlay handling block returns early at ~line 2079 without writing the cloned tab back. The GoTo path (line 2012) and text-select path (line 2142) both do `tabs[*active_tab] = tab;` before returning, but the general overlay path does not.

**Fix:** Find the line `return Ok(());` that is at the end of the `if state.overlay.is_some()` block (after the `match key.code { ... }` for overlay handling, right before `// ── Text selection mode`). Add `tabs[*active_tab] = tab;` before that `return Ok(())`.

### M7: Find overlay can't type h, q, ?, j, k
In the overlay handling `match key.code` block (~lines 2016-2060), the close/navigation arms (`Char('q') | Esc | Char('h') | Char('?')`, `Down | Char('j')`, `Up | Char('k')`) fire before the Find-typing arm (`Char(ch) if matches!(state.overlay, Some(Find))`). 

**Fix:** Add `&& state.overlay != Some(jaringan_browser::Overlay::Find)` guards to:
1. The close arm: `KeyCode::Char('q') | KeyCode::Esc | KeyCode::Char('h') | KeyCode::Char('?')` → add guard `if state.overlay != Some(jaringan_browser::Overlay::Find)`
2. The `Down | Char('j')` arm → add same guard
3. The `Up | Char('k')` arm → add same guard

### H3: q/Esc quits while editing input
The `KeyCode::Char('q') | KeyCode::Esc` quit arm (~line 2148) fires before the input-typing arm. 

**Fix:** Add `&& !is_selected_input(page, state.selected)` guard to the quit arm. Also add the same guard to `KeyCode::Esc` so Esc exits text selection mode or closes overlays rather than quitting when editing.

### Commit
```bash
git add crates/jaringan-browser/src/main.rs
git commit -m "fix(browser): overlay state write-back (C1), Find typing (M7), q/Esc in inputs (H3)"
```

---

## Task B: H1 + M5 — Download confirm shadowed + error page download prompt

**File:** `crates/jaringan-browser/src/main.rs`

### H1: Download 'y' confirmation unreachable
The `Char('y')` copy-URL arm (~line 2187) has no guard and matches before the `Char('y') if state.pending_download.is_some()` arm (~line 2404).

**Fix:** Add `&& state.pending_download.is_none()` guard to the copy-URL `Char('y')` arm. This lets the download-confirm arm match when a download is pending.

### M5: Network error pages trigger download prompt
`is_renderable_content_type` (~line 5055) doesn't include "error" content type. When `load_location` returns an error page with `content_type: "error"`, the link activator treats it as a download.

**Fix:** Add `|| ct.contains("error")` to the `is_renderable_content_type` function.

### Commit
```bash
git add crates/jaringan-browser/src/main.rs
git commit -m "fix(browser): download confirm reachable (H1), error pages not download prompt (M5)"
```

---

## Task C: H2 — History overlay opens wrong entry

**File:** `crates/jaringan-browser/src/main.rs`

### H2: History overlay index mismatch
`draw_history_overlay` (~line 3812) renders `state.history.iter().rev().enumerate()` (newest first), marking `i == state.overlay_selected`. But the Enter handler (~line 2036) opens `state.history.get(state.overlay_selected)` — indexing the original oldest-first order.

**Fix:** In the overlay Enter handler, compute the correct index. Change the history lookup from:
```rust
state.history.get(state.overlay_selected).map(|e| e.url.clone())
```
to:
```rust
let rev_idx = state.history.len().saturating_sub(1).saturating_sub(state.overlay_selected);
state.history.get(rev_idx).map(|e| e.url.clone())
```

### Commit
```bash
git add crates/jaringan-browser/src/main.rs
git commit -m "fix(browser): history overlay opens correct entry (H2)"
```

---

## Task D: M1, M3, M4 — Stale state after navigation

**File:** `crates/jaringan-browser/src/lib.rs` and `crates/jaringan-browser/src/main.rs`

### M1: Stale find_state after navigation
`navigate_to` in `lib.rs` (~line 366) resets `selected`, `scroll_offset`, `overlay`, `status` but not `find_state`.

**Fix:** In `navigate_to`, add these resets:
```rust
state.find_state = FindState {
    query: String::new(),
    matches: Vec::new(),
    match_idx: 0,
};
state.pending_confirmation = None;
state.pending_download = None;
state.text_select_active = false;
```

### M3: Auth status not updated after link navigation
In `activate_selected` (~line 2818), the Link branch calls `navigate_to` and `record_current` but not `update_auth_status`.

**Fix:** Add `update_auth_status(state, &loaded.document);` after `state.record_current(...)` in the Link branch of `activate_selected`. Also add it in `activate_button` after page content is replaced (in both the Network Post and File Post branches, after `*page = LoadedPage { ... }`).

### M4: pending_confirmation not cleared on link navigation
The Link branch in `activate_selected` doesn't clear `pending_confirmation`.

**Fix:** This is handled by the M1 fix (navigate_to clears it). No separate change needed if M1 is applied.

### Commit
```bash
git add crates/jaringan-browser/src/lib.rs crates/jaringan-browser/src/main.rs
git commit -m "fix(browser): reset stale state on navigation (M1), auth status on link nav (M3/M4)"
```

---

## Task E: M2, M6, M8, M9, M11 — Mouse + scrolling + Home/End fixes

**File:** `crates/jaringan-browser/src/main.rs`

### M2: Mouse click doesn't update file_mtime
In `handle_mouse_event`, after `activate_selected(state, page, ...)` in the Down(Left) branch (~line 2534), the `file_mtime` is not updated.

**Fix:** After `activate_selected(state, page, script_runtime, bridge)?;` in the mouse handler, add:
```rust
*file_mtime = file_mtime_of(&page.location);
```
Note: you need to get a mutable reference to `tab.file_mtime`. The current code has `let file_mtime = &mut tab.file_mtime;` — check if it exists in the mouse handler. If not, add it.

### M6: Home/End change selection in Scroll mode
`Home` and `End` (~lines 2245-2246) always call `selection_first`/`selection_last` regardless of mode.

**Fix:** Gate on mode:
```rust
KeyCode::Home => match state.mode {
    BrowserMode::Selection => selection_first(state),
    BrowserMode::Scroll => scroll_to_top(state),
},
KeyCode::End => match state.mode {
    BrowserMode::Selection => selection_last(state, page.items.len()),
    BrowserMode::Scroll => {
        let line_count = render_lines(page, state.selected, &state.find_state, find_color_for(state), state.show_source).len();
        if let Ok(size) = terminal.size() {
            let viewport_height = size.height.saturating_sub(8);
            scroll_to_bottom(state, line_count, viewport_height);
        }
    }
},
```

### M8: Mouse tab-click wrong tab width
In `handle_mouse_event` tab-bar branch (~line 2481), the width estimate `label.chars().count() + 4` doesn't match `draw_tab_bar`.

**Fix:** Read `draw_tab_bar` to see the exact format. The tab format is `"▸ {title} "` or `"  {title} "` with possible truncation and watch mark. Match the exact formula. At minimum, account for:
- Title truncation at 20 chars (title chars > 20 → 18 chars + "…")
- Watch mark " ◉" when live-reload is active (+2 chars)
- Prefix: 2 chars ("▸ " or "  ")

### M9: Mouse click maps wrong item for inline paragraph links
`block_line_count` and the item-offset counter in the mouse handler only count standalone `Block::Link/Input/Button/Image`, not inline links inside paragraphs.

**Fix:** In the item-offset counting loop inside `handle_mouse_event`, also count inline links from `Block::Paragraph`. For a paragraph, count how many `InlineSpan::Link` entries it contains and add that to `item_offset`. You'll need to call `split_inline_spans` on paragraph text and count links.

### M11: Mouse events processed when enable_mouse is false
`run_tui` always enables mouse capture.

**Fix:** In `run_tui`, only call `EnableMouseCapture` if the config has `enable_mouse: true`. Load the config first (like `run_app` does), or pass a flag. Simplest: move the config load to before `execute!` and conditionally enable. Also in `run_app`, only dispatch `Event::Mouse` if `cfg.enable_mouse`.

### Commit
```bash
git add crates/jaringan-browser/src/main.rs
git commit -m "fix(browser): mouse file_mtime (M2), Home/End mode (M6), tab width (M8), inline links (M9), mouse config gate (M11)"
```

---

## Verification (after all tasks complete)

```bash
cd /home/lekmon/jaringan
cargo build -p jaringan-browser 2>&1 | grep -E "^error" | head -20
cargo test -p jaringan-browser 2>&1 | tail -10
cargo build --workspace 2>&1 | tail -5
```
