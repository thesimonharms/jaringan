# Jaringan Page Format 0.1

Jaringan pages are UTF-8 plain text documents with the `.jrg` extension. A valid page should be useful when opened in a normal text editor and more structured when parsed by an AI agent or a terminal-native browser.

## File extension

- Pages MUST use `.jrg`.
- A path that does not end in `.jrg` is not a page document.
- A folder path ending in `/` MAY serve that folder's `index.jrg`.

## Blocks

Blocks are separated by blank lines where needed. The parser currently recognizes these line-oriented blocks.

### Headings

```text
# Heading 1
## Heading 2
###### Heading 6
```

- One to six leading `#` characters followed by a space.
- The first level-1 heading is the visible title fallback.
- Metadata `title:` overrides the visible title for browser chrome.

### Paragraphs

```text
Paragraph text can wrap over
multiple source lines.
```

- Consecutive non-empty lines become one paragraph.
- Source line breaks inside a paragraph collapse to spaces.

### Links

```text
=> target Label text
```

- `target` is the first token after `=>`.
- Label is the rest of the line.
- If label is omitted, the target is used as the label.
- Relative links resolve like HTML anchors against the current page URL/path.

Examples:

```text
=> about.jrg About
=> ../index.jrg Parent home
=> #section-two Same page fragment
=> jrg://example.org/docs/start.jrg Remote page
```

### Inputs

```text
? name label="Input label" value="current value" placeholder="Hint text"
```

- Inputs are structured form fields.
- `name` is required and is the machine-readable field key.
- `label` is optional and defaults to `name`.
- `value` is optional and defaults to an empty string.
- `placeholder` is optional hint text for empty inputs.
- The prototype browser renders inputs as selectable fields.
- When an input is selected, printable key presses append to the value and Backspace deletes the previous character.

### Buttons / actions

```text
! id label="Button label" method="POST" target="action-or-target" confirm="Run this action?" auth="capability-token"
```

- Buttons are terminal-native controls and may represent side-effectful actions.
- `id` is required.
- `label` is optional and defaults to `id`.
- `target` is optional and defaults to `id`.
- `method` is optional and defaults to `GET`; supported values are `GET` and `POST`.
- `confirm` is optional explicit confirmation copy.
- `auth` is an optional action capability token. When present, the browser includes it in the action request so the resolver can authorize the side effect.
- When `confirm` is present, the prototype browser requires pressing Enter twice: first to stage the action and display the confirmation prompt, then again to confirm it.
- Confirmed `GET` actions are surfaced in browser status text with the collected form payload appended as query parameters.
- Confirmed `POST` actions collect all current input values into a URL-encoded payload.
- Network `POST` actions send that payload over the TCP transport and render the returned page.
- Local absolute `POST` targets such as `/actions/search` are handled by the prototype local resolver for demo side effects.

### Images

```text
@ ./cover.png alt="Cover image"
```

- Image source is required.
- `alt` is optional and defaults to the source.
- Terminal browsers may render inline when supported, download/cache remote images, or expose the alt text and source as selectable items.

### Preformatted blocks

````text
```plain
spacing is preserved
```
````

- Start with a line beginning with triple backticks.
- End with a line beginning with triple backticks.
- Contents preserve line breaks and spacing.

## Metadata

Metadata starts after a line containing exactly `~~~~~` and continues to the end of the file. Metadata is not rendered as page body.

Security metadata is optional and uses the same trailing block:

```text
signed-by: alice
signature: ed25519:<base64-signature>
```

Unsigned pages are valid. Browsers use keyrings to verify signatures when present and display secure/not-secure state.

```text
~~~~~
title: Simon's page
date: 2026-06-07
redirect: jrg://example.org/new.jrg
```

- Everything after the first line that is exactly `~~~~~` is metadata.
- Metadata is unstructured text in 0.1.
- The recommended convention is `key: value` lines.
- `title:` is recognized by the parser and browser title logic.
- `redirect:` is only a tag/convention for now. Browsers should not auto-follow without a future redirect-safety policy.
- Metadata is not rendered as body content.

## Plain-text fallback

A `.jrg` page should remain understandable without a Jaringan browser:

- Headings are readable Markdown-like text.
- Links expose both target and label.
- Images include alt text.
- Buttons include a label and target.
- Metadata is visibly separated at the bottom.
