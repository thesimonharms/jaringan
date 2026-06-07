# jaringan

`jaringan` is an experiment in an internet that is cheap for AI agents and pleasant for humans in terminals.

The current web is optimized for graphical browsers. AI browser-use workflows have to drive those browsers, parse screenshots, and spend a lot of tokens on pages that were never designed for them. `jaringan` flips the default: pages are structured terminal-native documents that remain useful as plain text, can be rendered in a TUI, and can be fetched over a simple protocol.

## Product thesis

- **AI-native:** content exposes structure, links, actions, metadata, and semantic sections without screenshots or DOM spelunking.
- **Human terminal-native:** a ratatui browser makes documents feel first-class in the terminal.
- **Plain-text resilient:** every page should still be mostly useful over `curl`, `nc`, logs, or an LLM context window.
- **Cheap to index:** a later search engine can crawl compact structured pages instead of JS-heavy sites.

## First three parts

1. **Sharing protocol (`jaringan-protocol`)**
   - Scheme: `jar://host/path` for network locations.
   - For the first prototype, support local files and plain response parsing before implementing sockets/TLS.
   - Response format is line-oriented UTF-8, easy to stream and inspect.

2. **Rendering protocol (`jaringan-core` + `jaringan-render`)**
   - Pages are structured blocks: headings, paragraphs, links, preformatted blocks, and actions.
   - Blocks render to plain text with stable markers.
   - Ratatui render model can later map the same blocks to widgets.

3. **Browser (`jaringan-browser`)**
   - CLI/TUI entrypoint.
   - Prototype command prints a parsed document from a file or URL-like input.
   - Later: navigation stack, keybindings, forms/actions, history, bookmarks.

## Repository layout

- `crates/jaringan-core`: shared document model and plain-text parser/serializer.
- `crates/jaringan-protocol`: request/response types and `jar://` URL parsing.
- `crates/jaringan-render`: plain-text rendering and future ratatui rendering adapters.
- `crates/jaringan-browser`: CLI/TUI application.
- `docs/`: architecture notes and implementation plans.

## Quick start

```bash
cargo test
cargo run -p jaringan-browser -- sample docs/examples/hello.jar
```
