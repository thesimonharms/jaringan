# Jaringan Search 0.1

M5 starts with a local crawler/search prototype for `.jrg` document roots.

## Indexed fields

Each page contributes a `SearchEntry` with:

- URL: `jrg://local/<relative-path>` for local roots.
- Title: metadata `title:` when present, otherwise the first level-1 heading, otherwise `Untitled`.
- Headings: every heading block in document order.
- Links: link labels and targets.
- Metadata: trailing text after `~~~~~`.

The index deliberately ignores non-`.jrg` files.

## Ranking

Search is case-insensitive and field-weighted:

- title match: 10 points
- heading match: 5 points
- link label/target match: 3 points
- metadata match: 1 point

Results sort by score descending, then title, then URL for deterministic output.

## Prototype CLI

```bash
cargo run -p jaringan-browser -- index docs/examples
cargo run -p jaringan-browser -- search docs/examples laksa
```

`index` prints one tab-separated line per indexed page:

```text
jrg://local/action-form.jrg	M4 action form example
```

`search` prints tab-separated result lines:

```text
<score>	<url>	<title>	<snippet>
```

## Future work

- Crawl remote `jrg://` graphs with redirect limits and robots/rate-limit policy.
- Persist index files instead of rebuilding each query.
- Tokenize and normalize queries beyond substring matching.
- Expose search results inside the TUI.
