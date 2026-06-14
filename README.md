# jaringan

`jaringan` is an experiment in an internet that is cheap for AI agents and pleasant for humans in terminals.

The current web is optimized for graphical browsers. AI browser-use workflows have to drive those browsers, parse screenshots, and spend a lot of tokens on pages that were never designed for them. `jaringan` flips the default: pages are structured terminal-native documents that remain useful as plain text, can be rendered in a TUI, and can be fetched over a simple protocol.

## Product thesis

- **AI-native:** content exposes structure, links, actions, metadata, and semantic sections without screenshots or DOM spelunking.
- **Human terminal-native:** a ratatui browser makes documents feel first-class in the terminal.
- **Plain-text resilient:** every page should still be mostly useful over `curl`, `nc`, logs, or an LLM context window.
- **Cheap to index:** a search engine can crawl compact structured pages instead of JS-heavy sites.

## The six parts

1. **Sharing protocol (`jaringan-protocol`)**
   - Scheme: `jrg://host/path` for network locations.
   - Query strings and fragments are supported.
   - `/foo.jrg` is a document, `/foo/` is a folder index, and `/foo` deliberately does not resolve.
   - `jrg://` is the single secure-capable scheme: signed pages use public keyrings, encrypted payload capabilities live under the protocol layer, and browsers show secure/not secure instead of inventing a second scheme.
   - The first TCP transport is a tiny text protocol for local experimentation, with optional encrypted framing for pre-shared-key deployments.

2. **Rendering protocol (`jaringan-core` + `jaringan-render`)**
   - Pages are structured blocks: headings, paragraphs, links, structured inputs, action buttons, images, quotes, lists, rules, tables, preformatted blocks, and trailing metadata after `~~~~~`.
   - Blocks render to plain text with stable markers.
   - Ratatui render model can later map the same blocks to widgets.

3. **Browser (`jaringan-browser`)**
   - CLI/TUI entrypoint.
   - `sample` prints a parsed local document.
   - `fetch` exercises the protocol resolver against a local document root.
   - `serve` exposes a local document root over TCP.
   - `get` fetches `jrg://host:port/path` over TCP.
   - `open` launches the modal ratatui browser for either local `.jrg` files or `jrg://` URLs.
   - Forms/actions support editable inputs, explicit confirmation, and capability-token-gated demo side effects.
   - `index` / `search` for local M5 crawl/search experiments with persisted `.jrgidx`.

4. **Gateways (`jaringan-gateway`)**
   - `serve-http` — HTTP server that proxies each request to a JRG backend.
   - `jrg-to-http` — JRG server that fetches an HTTP URL per request and returns the body as JRG, letting the Jaringan browser reach regular web pages.
   - Renders JRG pages as semantic HTML so any browser can read them.

5. **Search engine (`jaringan-search`, M11)**
   - JRG server with a search form, submit form, status page, and verify flow.
   - Domain owners prove ownership with a DNS TXT record at `_jrg-verify.<domain>`.
   - The engine fetches the site's `/.well-known/jrgidx` (a small text index file), validates that every listed page is Ed25519-signed and encryption-capable, and adds it to the master index.
   - Background re-indexing keeps the catalog fresh without operator intervention.
   - See `docs/spec/jrg-search.md`.

6. **Vhost reverse proxy (`jaringan-proxy`)**
   - Byte-forwarding TCP proxy that fans one public port out to many backends via `Host:` header routing.
   - Sits in front of the search engine, the content server, and any future services so external clients can reach each one on the same host.
   - Protocol-agnostic: forwards both raw JRG and HTTP unchanged. Backends do their own protocol detection.
   - See `docs/spec/jrg-proxy.md`.

## Repository layout

- `crates/jaringan-core`: shared document model, plain-text parser/serializer, `IndexEntryV1`/`SearchIndexV1` for the public jrgidx format, `PublicKeyring`, signature verification.
- `crates/jaringan-protocol`: request/response types, `jrg://` URL parsing, status codes, response tags, local resolver, encrypted TCP framing.
- `crates/jaringan-render`: plain-text rendering and future ratatui rendering adapters.
- `crates/jaringan-browser`: CLI/TUI application.
- `crates/jaringan-gateway`: HTTP↔JRG gateways.
- `crates/jaringan-search`: M11 public search engine (DNS-verified submission, jrgidx validation, periodic re-index).
- `crates/jaringan-proxy`: vhost reverse proxy that routes by `Host:` header.
- `docs/`: architecture notes, specs, and implementation plans.

## Quick start

```bash
cargo test

# Render a local page as plain text
cargo run -p jaringan-browser -- sample docs/examples/hello.jrg

# Serve and fetch over the JRG TCP protocol
cargo run -p jaringan-browser -- serve docs/examples --bind 127.0.0.1:7070
cargo run -p jaringan-browser -- get jrg://127.0.0.1:7070/

# Open in the interactive ratatui browser
cargo run -p jaringan-browser -- open jrg://127.0.0.1:7070/

# Encrypted TCP framing (pre-shared key)
JARINGAN_ENCRYPTION_KEY_HEX=0000000000000000000000000000000000000000000000000000000000000000 \
  cargo run -p jaringan-browser -- serve docs/examples --bind 127.0.0.1:7070 --encrypted-key-id local-dev

# Local M5 search experiments
cargo run -p jaringan-browser -- index docs/examples --output /tmp/docs.jrgidx
cargo run -p jaringan-browser -- search docs/examples action --index /tmp/docs.jrgidx

# Public M11 search engine (separate binary)
cargo run -p jaringan-search -- serve --bind 127.0.0.1:7071 --data-dir /tmp/jrg-search

# Vhost reverse proxy (separate binary)
cargo run -p jaringan-proxy -- \
    --bind 0.0.0.0:7070 \
    --routes search.example.com=127.0.0.1:7071,jrg.example.com=127.0.0.1:7072

# HTTP↔JRG gateway (lets regular browsers read JRG content)
cargo run -p jaringan-gateway -- serve-http --http-listen 127.0.0.1:18080 --jrg-host 127.0.0.1:7070
```

Use `sample` for plain-text output, `fetch` for local protocol-path resolution, `serve`/`get` for TCP transport, `get --follow` for non-interactive redirect following, `--encrypted-key-id` plus `JARINGAN_ENCRYPTION_KEY_HEX` for encrypted TCP framing, `index`/`search` for local M5 crawl/search experiments, and `open` for the interactive ratatui browser over local files or TCP `jrg://` pages. `index --output` persists a reusable `.jrgidx` search index, and `search --index` queries that index instead of crawling. M4/M5 form syntax uses `? name ...` inputs and `! id ...` buttons. Inputs can be edited in the browser; confirmed POST actions submit URL-encoded values, and local GET `/search` actions render selectable search results in the TUI.

For signed pages, put trusted Ed25519 public keys in `~/.config/jaringan/keyring`:

```text
# signer-name ed25519:<base64-public-key>
alice ed25519:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=
```

Set `JARINGAN_KEYRING=/path/to/keyring` to point the browser at another keyring file.

## Browser controls

The interactive `open` command uses modal keyboard navigation:

| Mode | Keys | Action |
|------|------|--------|
| Both | `Tab` / `s` / `v` | Toggle / Scroll / Selection mode |
| Selection | `j` / `k` / `↓` / `↑` | Move selection |
| Selection | `Enter` | Open link / Press button / Edit input |
| Selection | `g` / `G` | Jump to top / bottom of page |
| Selection | `Home` / `End` | First / Last selectable item |
| Selection | `PgDn` / `Space` / `PgUp` | Page down / Page up |
| Both | `b` | Go back |
| Both | `f` | Go forward |
| Both | `r` | Reload page |
| Both | `?` / `h` | Toggle help overlay |
| Both | `q` / `Esc` | Quit |

## Live demo

The documentation and example pages are hosted at these addresses:

| Endpoint | What you can do |
|----------|-----------------|
| [http-gateway.simonharms.xyz](https://http-gateway.simonharms.xyz) | Browse JRG docs/demos in any browser (rendered as HTML) |
| [jrg://jrg.simonharms.xyz](jrg://jrg.simonharms.xyz) | Access the same content natively via `jaringan-browser` |
| [jrg://search.simonharms.xyz](jrg://search.simonharms.xyz) | The public JRG search engine (M11) |

To browse via JRG natively:
```bash
jaringan-browser get jrg://jrg.simonharms.xyz/
jaringan-browser open jrg://jrg.simonharms.xyz/
jaringan-browser open jrg://search.simonharms.xyz/
```

To submit your own site to the search engine:
```bash
jaringan-browser open jrg://search.simonharms.xyz/submit.jrg
```

## Specs

- `docs/spec/jrg-page-format.md` — block grammar, metadata delimiter, plain-text fallback
- `docs/spec/jrg-protocol.md` — `jrg://` URL semantics, TCP wire format, encryption capabilities
- `docs/spec/jrg-security.md` — Ed25519 page signatures, keyrings, browser indicators
- `docs/spec/jrg-encryption.md` — XChaCha20-Poly1305 payload encryption and encrypted TCP framing
- `docs/spec/jrg-search.md` — M11 search engine: DNS-verified submission, jrgidx v1.0 format, Ed25519 + encryption requirements
- `docs/spec/jrg-proxy.md` — vhost reverse proxy: Host-header routing, security model, deployment
