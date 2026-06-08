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
   - `jrg://` is the single secure-capable scheme: signed pages use public keyrings and browsers show secure/not secure instead of inventing a second scheme.
   - The first TCP transport is a tiny text protocol for local experimentation before encrypted transport/discovery.

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
cargo run -p jaringan-browser -- index docs/examples
cargo run -p jaringan-browser -- index docs/examples --output /tmp/docs.jrgidx
cargo run -p jaringan-browser -- search docs/examples action
cargo run -p jaringan-browser -- search docs/examples action --index /tmp/docs.jrgidx
cargo run -p jaringan-browser -- open jrg://127.0.0.1:7070/
cargo run -p jaringan-browser -- open docs/examples/hello.jrg
cargo run -p jaringan-browser -- open docs/examples/search-form.jrg
```

Use `sample` for plain-text output, `fetch` for local protocol-path resolution, `serve`/`get` for TCP transport, `get --follow` for non-interactive redirect following, `index`/`search` for local M5 crawl/search experiments, and `open` for the interactive ratatui browser over local files or TCP `jrg://` pages. `index --output` persists a reusable `.jrgidx` search index, and `search --index` queries that index instead of crawling. M4/M5 form syntax uses `? name ...` inputs and `! id ...` buttons. Inputs can be edited in the browser; confirmed POST actions submit URL-encoded values, and local GET `/search` actions render selectable search results in the TUI.

For signed pages, put trusted Ed25519 public keys in `~/.config/jaringan/keyring`:

```text
# signer-name ed25519:<base64-public-key>
alice ed25519:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=
```

Set `JARINGAN_KEYRING=/path/to/keyring` to point the browser at another keyring file.

Specs:

- `docs/spec/jrg-page-format.md`
- `docs/spec/jrg-protocol.md`
- `docs/spec/jrg-security.md`
- `docs/spec/jrg-search.md`
