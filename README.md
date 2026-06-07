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
   - Scheme: `jrg://host/path` for network locations.
   - Query strings and fragments are supported.
   - `/foo.jrg` is a document, `/foo/` is a folder index, and `/foo` deliberately does not resolve.
   - The first TCP transport is a tiny text protocol for local experimentation before TLS/discovery.

2. **Rendering protocol (`jaringan-core` + `jaringan-render`)**
   - Pages are structured blocks: headings, paragraphs, links, structured inputs, action buttons, images, preformatted blocks, and trailing metadata after `~~~~~`.
   - Blocks render to plain text with stable markers.
   - Ratatui render model can later map the same blocks to widgets.

3. **Browser (`jaringan-browser`)**
   - CLI/TUI entrypoint.
   - `sample` prints a parsed local document.
   - `fetch` exercises the protocol resolver against a local document root.
   - `serve` exposes a local document root over TCP.
   - `get` fetches `jrg://host:port/path` over TCP.
   - `open` launches the modal ratatui browser for either local `.jrg` files or `jrg://` URLs.
   - Later: network transport, forms/actions, history persistence, bookmarks.

## Repository layout

- `crates/jaringan-core`: shared document model and plain-text parser/serializer.
- `crates/jaringan-protocol`: request/response types, `jrg://` URL parsing, status codes, response tags, and local resolver.
- `crates/jaringan-render`: plain-text rendering and future ratatui rendering adapters.
- `crates/jaringan-browser`: CLI/TUI application.
- `docs/`: architecture notes, specs, and implementation plans.

## Quick start

```bash
cargo test
cargo run -p jaringan-browser -- sample docs/examples/hello.jrg
cargo run -p jaringan-browser -- fetch docs/examples jrg://local/
cargo run -p jaringan-browser -- fetch docs/examples 'jrg://local/protocol.jrg?view=ai#top'
cargo run -p jaringan-browser -- serve docs/examples --bind 127.0.0.1:7070
cargo run -p jaringan-browser -- get jrg://127.0.0.1:7070/
cargo run -p jaringan-browser -- get --follow jrg://127.0.0.1:7070/
cargo run -p jaringan-browser -- open jrg://127.0.0.1:7070/
cargo run -p jaringan-browser -- open docs/examples/hello.jrg
```

Use `sample` for plain-text output, `fetch` for local protocol-path resolution, `serve`/`get` for TCP transport, `get --follow` for non-interactive redirect following, and `open` for the interactive ratatui browser over local files or TCP `jrg://` pages. M4 form/action syntax uses `? name ...` inputs and `! id ... method="POST" confirm="..."` buttons; confirmed actions require pressing Enter twice in the browser before they are surfaced as confirmed.

Specs:

- `docs/spec/jrg-page-format.md`
- `docs/spec/jrg-protocol.md`
