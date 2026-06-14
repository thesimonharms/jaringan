# Architecture

## Design constraints

- Documents must be useful as raw UTF-8 text.
- A parser should recover structure without executing code.
- Links and actions must be explicit and machine-readable.
- Rendering should be deterministic so agents can quote line numbers and labels.
- The first milestone should work offline; networking can come after the format stabilizes.

## Draft document format

A Jaringan page is a sequence of line-oriented blocks:

```text
# Title

Paragraph text can span lines until a blank line.

=> jrg://example/about About page
=> https://example.com Outside-web fallback

```plain
Preformatted content keeps spacing.
```

! action-id label="Do thing" method="POST" target="/actions/do-thing"
```

Current parser support includes headings, paragraphs, links (`=> target label`), structured inputs (`? name ...`), action buttons (`! id ...` with `method`/`confirm`/`auth`), images, quotes (`>`), lists (`-`/`*`), rules (`---`), tables (`| cell |`), preformatted blocks, and trailing metadata after `~~~~~`. M4 renders editable inputs/actions, enforces explicit two-step confirmation for confirmed actions, collects URL-encoded form payloads, and executes prototype POST actions over network or local demo resolvers. M8 adds action capability tokens so side-effectful demo POSTs can reject unauthorized submissions before recording side effects. M9 polishes the TUI with richer typography, accent rails, aligned tables, and calmer chrome.

## Crate responsibilities

### `jaringan-core`

Owns the stable data model:

- `Document`
- `Block`
- `Link`
- `Button`
- `Input`
- `Image`
- `SearchEntry` / `SearchIndex` — local M5 prototype format
- `IndexEntryV1` / `SearchIndexV1` — public jrgidx v1.0 exchange format
- `PublicKeyring` / `SignatureStatus`
- parser/serializer for the text-first page format
- title/heading/link/metadata extraction for local search
- tokenized local search, snippets, and prototype index text serialization
- v1.0 jrgidx serialization with Ed25519 + encryption field validation
- same-scheme page signature verification against public keyrings

### `jaringan-protocol`

Owns transport-facing types:

- `JaringanUrl`
- `Request`
- `Response`
- status codes and content type declarations
- `ResponseTag` redirect tags
- `PageResolver` and `LocalFileResolver`
- TCP `serve`/`fetch_tcp` transport helpers
- encrypted TCP `serve_encrypted`/`fetch_tcp_encrypted` helpers using pre-shared-key frames
- XChaCha20-Poly1305 encrypted payload helpers and encryption capability metadata

### `jaringan-render`

Converts the model into presentation output:

- plain text for AI/log/curl usage
- ratatui `Text`/`Line` adapters later

### `jaringan-browser`

Application shell:

- CLI arguments
- sample rendering command
- local protocol fetch command
- TCP protocol serve/get commands, including optional encrypted pre-shared-key TCP mode
- modal ratatui event loop for local files and TCP `jrg://` URLs
- browser-side redirect following and network error pages
- bounded TCP client timeouts and `get --follow`
- selection/scroll interaction state
- editable form inputs and confirmed POST action submission
- local `index`/`search` commands for M5 crawl/search experiments
- persisted `.jrgidx` files and local GET `/search` result pages in the TUI
- secure/not-secure indicator in the browser header

### `jaringan-search` (M11)

The public JRG search engine:

- JRG server (`PageResolver`) with search, submit, status, verify endpoints
- DNS TXT ownership verification at `_jrg-verify.<domain>`
- jrgidx v1.0 fetch and validation (Ed25519 + encryption-capability format)
- Aggregate `SearchIndexV1` of all submitted domains' pages
- Background periodic re-index thread (default 6h, configurable)
- On-disk persistence of submissions and index in `--data-dir`
- Reads from the same `IndexEntryV1` / `SearchIndexV1` types in `jaringan-core`

See `docs/spec/jrg-search.md` for the full submission protocol and jrgidx
v1.0 format.

### `jaringan-proxy`

Tiny byte-forwarding reverse proxy that routes by `Host:` header. Sits in
front of multiple Jaringan services on a single public port so external
clients can reach each one on the same host. Protocol-agnostic: forwards
both raw JRG and HTTP traffic unchanged. Backends do dual-protocol
detection themselves.

**Security model:** Only explicitly routed hosts can reach backends. Unknown
hostnames are rejected by default. Error responses are generic — no backend
addresses, ports, or diagnostics leak to the client. Request head sizes are
capped; body forwarding is bounded.

See `docs/spec/jrg-proxy.md` for the routing model and hardening details.

### `jaringan-gateway`

HTTP↔JRG bidirectional bridge:

- `serve-http` — HTTP server that proxies each request to a JRG backend
- `jrg-to-http` — JRG server that fetches an HTTP URL per request and
  returns the body as JRG (used for reaching regular web pages through the
  Jaringan browser)
- renders JRG pages as semantic HTML for regular browsers

## Specs

- `docs/spec/jrg-page-format.md`: `.jrg` block grammar, metadata delimiter, plain-text fallback rules.
- `docs/spec/jrg-protocol.md`: `jrg://` URL semantics, strict `.jrg` path rules, status codes, response tags, local resolver behavior, TCP wire format, and encryption capability values.
- `docs/spec/jrg-security.md`: same-scheme security model, public-keyring signatures, browser indicators.
- `docs/spec/jrg-encryption.md`: XChaCha20-Poly1305 encrypted payload model, capability metadata, and encrypted TCP frame shape.
- `docs/spec/jrg-search.md`: M11 public search engine — DNS-verified submission, jrgidx v1.0 exchange format, Ed25519 + encryption requirements, periodic re-indexing.
- `docs/spec/jrg-proxy.md`: vhost reverse proxy — Host-header routing, byte forwarding, security model.

## Milestones

1. **M0 scaffold:** workspace, docs, core parser, plain renderer, browser sample command.
2. **M1 file browser:** open local `.jrg` pages, navigate links between local files, maintain history.
3. **M2 protocol contract:** define `jrg://` URL/path semantics, page metadata, status codes, response tags, and resolver abstraction.
4. **M3 protocol server/client:** serve, fetch, browse `jrg://` pages over TCP, follow redirect tags in the browser/CLI, display network error pages, and use bounded client timeouts, then harden toward TLS/discovery.
5. **M4 actions/forms:** structured/editable inputs, action buttons with explicit two-step confirmation, URL-encoded payload collection, TCP POST action submission, and a local demo action handler.
6. **M5 crawler/search:** index page titles, headings, links, metadata, and body text; crawl local `.jrg` roots, query/persist the resulting search index, and expose local TUI search result pages.
7. **M6 security indicators:** keep `jrg://` as one secure-capable scheme, load human-editable Ed25519 public keyrings, verify optional page signatures against trusted keys, and display secure/not-secure state without gatekeeping unsigned pages.
8. **M7 encryption capabilities:** keep `jrg://` as one scheme, define XChaCha20-Poly1305 encrypted payload primitives, serialize encryption capability metadata, and support encrypted TCP request/response framing with pre-shared keys.
9. **M8 action auth model:** add explicit `auth` capability tokens to side-effectful action buttons, carry them as `Action-Token` headers, and reject unauthorized demo POSTs before writing side effects.
10. **M9 browser experience:** add rich renderable blocks (quotes, lists, rules, tables), improve plain/TUI rendering, and make the terminal browser more aesthetic with accent styling and aligned layouts.
11. **M10 browser ergonomics:** add help overlay (`?`/`h`), forward history (`f`), page scrolling (`PgUp`/`PgDown`/`g`/`G`), selection shortcuts (`Home`/`End`), and polished key hints in footer and docs.
12. **M11 public search engine:** add the `jaringan-search` and `jaringan-proxy` crates, DNS-verified domain submission, jrgidx v1.0 exchange format, Ed25519 + encryption requirements, background re-indexing, and the vhost reverse proxy that fans one public port to all services.