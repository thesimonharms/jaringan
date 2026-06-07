# Jaringan Protocol 0.1

Jaringan's protocol exists to fetch terminal-native `.jrg` pages for both humans and AI agents. Version 0.1 defines URL semantics, status concepts, response tags, local resolver behavior, and a tiny TCP wire transport.

## URL scheme

Jaringan URLs use the `jrg://` scheme:

```text
jrg://example.org/
jrg://example.org/about.jrg
jrg://example.org/folder/
jrg://example.org/search.jrg?q=terminal#results
```

- `jrg://` is the only supported scheme.
- `jar://` is intentionally rejected because `.jar` is the Java archive format.
- A host is required.
- Query strings are allowed.
- Fragments are allowed.

## Path semantics

Paths are intentionally strict:

- `/` resolves to the origin root folder and may serve `/index.jrg`.
- `/foo.jrg` resolves to a page document.
- `/foo/` resolves to a folder and may serve `/foo/index.jrg`.
- `/foo` does **not** resolve as a page because it is not a `.jrg` document and is not a folder path.

This avoids extension guessing and keeps document identity explicit.

## Relative links

Relative targets resolve like HTML anchors using URL joining rules:

Base:

```text
jrg://example.org/docs/start.jrg?old=1#top
```

Targets:

```text
guide/intro.jrg?mode=ai#install
../about.jrg
#section-two
```

Resolve to:

```text
jrg://example.org/docs/guide/intro.jrg?mode=ai#install
jrg://example.org/about.jrg
jrg://example.org/docs/start.jrg?old=1#section-two
```

## TCP wire transport

The first transport is intentionally tiny and line-oriented. It is for local experimentation, not final security or discovery.

Client request:

```text
GET jrg://127.0.0.1:7070/protocol.jrg?view=ai#top JRG/0.1
Host: 127.0.0.1:7070

```

The request target may be either a full `jrg://` URL or an absolute path when a `Host:` header is present.

Server response:

```text
JRG/0.1 200 OK
Content-Type: text/jrg; charset=utf-8

# Page body
```

Redirect tags are represented as headers:

```text
Tag-Redirect: jrg://example.org/new.jrg
```

Prototype commands:

```bash
cargo run -p jaringan-browser -- serve docs/examples --bind 127.0.0.1:7070
cargo run -p jaringan-browser -- get jrg://127.0.0.1:7070/
```

## Request

The in-process request model is currently:

```rust
Request { url: JaringanUrl }
```

Future network transports can add method, agent hints, accepted render capabilities, cache validators, and authentication without changing page syntax.

## Response

The in-process response model is currently:

```rust
Response {
    status: StatusCode,
    content_type: ContentType,
    tags: Vec<ResponseTag>,
    body: String,
}
```

Content types:

- `text/jrg; charset=utf-8`
- `text/plain; charset=utf-8`

## Status codes

0.1 copies the useful shape of HTTP status codes so clients and agents get familiar semantics:

- `200 OK`
- `301 Moved Permanently`
- `302 Found`
- `303 See Other`
- `304 Not Modified`
- `400 Bad Request`
- `403 Forbidden`
- `404 Not Found`
- `409 Conflict`
- `410 Gone`
- `422 Unprocessable Content`
- `429 Too Many Requests`
- `500 Internal Server Error`
- `501 Not Implemented`
- `502 Bad Gateway`
- `503 Service Unavailable`

## Response tags

Redirects are represented as tags instead of magic browser behavior:

```rust
ResponseTag::Redirect { target }
```

The prototype terminal browser follows redirect tags automatically for `jrg://` pages, resolving relative redirect targets against the current page URL and stopping after a small redirect limit. The lower-level `get` command prints the response and does not follow redirects.

## Local resolver

`LocalFileResolver` maps a `jrg://host/path` to a filesystem root for tests and local serving:

- `/` -> `<root>/index.jrg`
- `/foo/` -> `<root>/foo/index.jrg`
- `/foo.jrg` -> `<root>/foo.jrg`
- `/foo` -> `404 Not Found`

Query strings and fragments are accepted by the URL parser but ignored by the local file mapping.

## Not in 0.1

- No search/discovery.
- No identity or signatures.
- No redirect safety UI yet; the prototype browser follows redirects automatically.
- No content negotiation beyond basic content type enums.
- No TLS yet.
