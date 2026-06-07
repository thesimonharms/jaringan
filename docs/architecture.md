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

Current parser support is intentionally smaller: headings, paragraphs, links (`=> target label`), and preformatted blocks.

## Crate responsibilities

### `jaringan-core`

Owns the stable data model:

- `Document`
- `Block`
- `Link`
- parser/serializer for the text-first page format

### `jaringan-protocol`

Owns transport-facing types:

- `JaringanUrl`
- `Request`
- `Response`
- status codes and content type declarations

### `jaringan-render`

Converts the model into presentation output:

- plain text for AI/log/curl usage
- ratatui `Text`/`Line` adapters later

### `jaringan-browser`

Application shell:

- CLI arguments
- sample rendering command
- future ratatui event loop

## Milestones

1. **M0 scaffold:** workspace, docs, core parser, plain renderer, browser sample command.
2. **M1 file browser:** open local `.jrg` pages, navigate links between local files, maintain history.
3. **M2 protocol server/client:** serve and fetch `jrg://` pages over a simple TCP/TLS protocol.
4. **M3 terminal browser:** ratatui UI with viewport, selectable links, status bar, back/forward.
5. **M4 actions/forms:** structured inputs and side-effectful actions with explicit confirmation.
6. **M5 crawler/search:** index page titles, headings, links, and metadata.
