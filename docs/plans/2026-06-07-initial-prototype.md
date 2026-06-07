# Initial Prototype Implementation Plan

> **For Hermes:** Use subagent-driven-development skill to implement this plan task-by-task.

**Goal:** Build a working offline prototype for Jaringan: parse a terminal-native page format, render it as stable plain text, and expose it through a small CLI command.

**Architecture:** Keep the core document model separate from protocol and browser concerns. Start offline with files/samples so the page format and renderer can be tested before network transport. The browser crate depends on the parser and renderer but networking remains behind `jaringan-protocol`.

**Tech Stack:** Rust 2024 workspace, `thiserror`, `url`, `clap`, future `ratatui`.

---

### Task 1: Create workspace scaffold

**Objective:** Set up the repository as a Rust workspace with four crates.

**Files:**
- Create: `Cargo.toml`
- Create: `.gitignore`
- Create: `crates/jaringan-core/Cargo.toml`
- Create: `crates/jaringan-protocol/Cargo.toml`
- Create: `crates/jaringan-render/Cargo.toml`
- Create: `crates/jaringan-browser/Cargo.toml`

**Verification:**

Run:

```bash
cargo metadata --no-deps
```

Expected: JSON metadata containing all four packages.

### Task 2: Implement the core page model

**Objective:** Define the minimal blocks necessary for a useful plain-text page.

**Files:**
- Modify: `crates/jaringan-core/src/lib.rs`

**Implementation:**

Add `Document`, `Block`, and `Link` types. Implement a line-oriented parser that supports headings (`#`), links (`=>`), paragraphs, and fenced preformatted blocks.

**Verification:**

Run:

```bash
cargo test -p jaringan-core
```

Expected: parser tests pass for headings, links, paragraphs, and preformatted blocks.

### Task 3: Implement protocol URL and response types

**Objective:** Add typed protocol primitives without starting networking yet.

**Files:**
- Modify: `crates/jaringan-protocol/src/lib.rs`

**Implementation:**

Add `JaringanUrl`, `Request`, `Response`, and `StatusCode`.

**Verification:**

Run:

```bash
cargo test -p jaringan-protocol
```

Expected: URL parser accepts `jar://example.org/path` and rejects non-`jar` schemes.

### Task 4: Implement plain renderer

**Objective:** Convert parsed documents into deterministic plain text suitable for humans and agents.

**Files:**
- Modify: `crates/jaringan-render/src/lib.rs`

**Implementation:**

Render headings, paragraphs, links with numeric labels, and preformatted content.

**Verification:**

Run:

```bash
cargo test -p jaringan-render
```

Expected: renderer output includes title, paragraph text, and `[1] label <target>` link markers.

### Task 5: Implement browser sample command

**Objective:** Provide a concrete executable that proves the pieces compose.

**Files:**
- Modify: `crates/jaringan-browser/src/main.rs`
- Create: `docs/examples/hello.jar`

**Implementation:**

Add `jaringan-browser sample <path>` that reads a file, parses it, renders it, and prints output.

**Verification:**

Run:

```bash
cargo run -p jaringan-browser -- sample docs/examples/hello.jar
```

Expected: terminal output shows a rendered title, paragraph, and numbered link.

### Task 6: Format, test, and commit

**Objective:** Verify the scaffold is healthy and create the first commit.

**Verification:**

Run:

```bash
cargo fmt --all --check
cargo test
cargo run -p jaringan-browser -- sample docs/examples/hello.jar
```

Expected: all commands succeed.

Commit:

```bash
git add .
git commit -m "feat: scaffold jaringan prototype"
```
