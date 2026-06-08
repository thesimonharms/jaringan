# Jaringan Search 0.1

M5 provides a local crawler/search prototype for `.jrg` document roots.

## Indexed fields

Each page contributes a `SearchEntry` with:

- URL: `jrg://local/<relative-path>` for local roots.
- Title: metadata `title:` when present, otherwise the first level-1 heading, otherwise `Untitled`.
- Headings: every heading block in document order.
- Links: link labels and targets.
- Metadata: trailing text after `~~~~~`.
- Body: paragraphs, preformatted text, input labels/values, button labels, and image alt text.

The index deliberately ignores non-`.jrg` files.

## Ranking and matching

Search is case-insensitive and tokenized on non-alphanumeric separators. A field/sentence must contain every query token to match.

Weights:

- title match: 10 points per token
- heading match: 5 points per token
- link label/target match: 3 points per token
- metadata/body sentence match: 1 point per token

Results sort by score descending, then title, then URL for deterministic output. The snippet is the first matching title, heading, link text, metadata line, or body sentence.

## Persisted index files

`SearchIndex::to_index_text()` writes a compact text format headed by:

```text
JRG-SEARCH/0.1
```

The format is meant for local prototype reuse, not long-term compatibility yet. Use `.jrgidx` for persisted files.

## Prototype CLI

```bash
cargo run -p jaringan-browser -- index docs/examples
cargo run -p jaringan-browser -- index docs/examples --output /tmp/docs.jrgidx
cargo run -p jaringan-browser -- search docs/examples action
cargo run -p jaringan-browser -- search docs/examples action --index /tmp/docs.jrgidx
```

`index` prints one tab-separated line per indexed page:

```text
jrg://local/action-form.jrg	M4 action form example
```

`search` prints tab-separated result lines:

```text
<score>	<url>	<title>	<snippet>
```

## Local TUI search actions

A local page can expose a search form with structured input and a GET action:

```text
? q label="Search query" value="action" placeholder="Search docs"
! find label="Find" method="GET" target="/search"
```

When opened from a local file root, the browser handles `GET /search` by crawling the current page directory, searching for the current `q` value, and replacing the page with selectable result links.

See `docs/examples/search-form.jrg`.

## Future work

- Crawl remote `jrg://` graphs with redirect limits and robots/rate-limit policy.
- Promote the index format when compatibility matters.
- Add richer tokenization/stemming and highlighted snippets.
- Add a dedicated TUI search prompt/keybinding beyond page-authored `/search` forms.
