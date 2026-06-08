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

Current parser support includes headings, paragraphs, links (`=> target label`), structured inputs (`? name ...`), action buttons (`! id ...` with `method`/`confirm`), images, preformatted blocks, and trailing metadata after `~~~~~`. M4 renders editable inputs/actions, enforces explicit two-step confirmation for confirmed actions, collects URL-encoded form payloads, and executes prototype POST actions over network or local demo resolvers.

## Crate responsibilities

### `jaringan-core`

Owns the stable data model:

- `Document`
- `Block`
- `Link`
- `Button`
- `Input`
- `Image`
- parser/serializer for the text-first page format

### `jaringan-protocol`

Owns transport-facing types:

- `JaringanUrl`
- `Request`
- `Response`
- status codes and content type declarations
- `ResponseTag` redirect tags
- `PageResolver` and `LocalFileResolver`
- TCP `serve`/`fetch_tcp` transport helpers

### `jaringan-render`

Converts the model into presentation output:

- plain text for AI/log/curl usage
- ratatui `Text`/`Line` adapters later

### `jaringan-browser`

Application shell:

- CLI arguments
- sample rendering command
- local protocol fetch command
- TCP protocol serve/get commands
- modal ratatui event loop for local files and TCP `jrg://` URLs
- browser-side redirect following and network error pages
- bounded TCP client timeouts and `get --follow`
- selection/scroll interaction state
- editable form inputs and confirmed POST action submission

## Specs

- `docs/spec/jrg-page-format.md`: `.jrg` block grammar, metadata delimiter, plain-text fallback rules.
- `docs/spec/jrg-protocol.md`: `jrg://` URL semantics, strict `.jrg` path rules, status codes, response tags, local resolver behavior, TCP wire format.

## Milestones

1. **M0 scaffold:** workspace, docs, core parser, plain renderer, browser sample command.
2. **M1 file browser:** open local `.jrg` pages, navigate links between local files, maintain history.
3. **M2 protocol contract:** define `jrg://` URL/path semantics, page metadata, status codes, response tags, and resolver abstraction.
4. **M3 protocol server/client:** serve, fetch, browse `jrg://` pages over TCP, follow redirect tags in the browser/CLI, display network error pages, and use bounded client timeouts, then harden toward TLS/discovery.
5. **M4 actions/forms:** structured/editable inputs, action buttons with explicit two-step confirmation, URL-encoded payload collection, TCP POST action submission, and a local demo action handler.
6. **M5 crawler/search:** index page titles, headings, links, and metadata.
