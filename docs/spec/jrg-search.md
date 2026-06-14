# Jaringan Search 1.0

The public JRG search engine is a submission-based indexer, not a crawler.
Anyone can list their domain; the engine verifies ownership via DNS, then pulls
the site's pages from a well-known `jrgidx` file. Every indexed page must be
Ed25519-signed and declare an encryption capability.

This is the milestone that turns M5's local `index`/`search` prototype into a
federated, host-controlled catalog.

## Concepts

| Term | Meaning |
|------|---------|
| **Domain owner** | The operator of `example.com` who wants their JRG site indexed |
| **jrgidx** | A small text file served at `/.well-known/jrgidx` listing the site's pages |
| **Search engine** | The `jaringan-search` service that accepts submissions and serves queries |
| **Submission** | A domain pending or verified by the engine |
| **Index entry** | One row in the aggregate jrgidx (a single indexed page) |

The engine never crawls. It only reads jrgidx files that domain owners
explicitly publish.

## The jrgidx exchange format

A jrgidx file is the canonical, portable way to list a JRG site's pages. The
engine consumes it from every verified domain; sites can also host their own
copy for other tools.

```text
JRG-SEARCH/1.0
jrg://example.com/intro.jrg	Introduction	ed25519:ABCD...	xchacha20poly1305;key-id=key1	ed25519:SIG...	2026-06-14T12:00:00Z
jrg://example.com/setup.jrg	Setup Guide	ed25519:ABCD...	xchacha20poly1305;key-id=key1	ed25519:SIG...	2026-06-14T12:00:00Z
```

### Fields

Each line after the header is a tab-separated row with exactly 6 fields:

| # | Field | Format | Purpose |
|---|-------|--------|---------|
| 1 | URL | `jrg://host/path.jrg` | Canonical page URL |
| 2 | Title | free text | Human-readable title |
| 3 | Public key | `ed25519:<base64>` | Ed25519 verifying key for the page signature |
| 4 | Encryption | `<suite>;key-id=<id>` | Encryption capability the page supports |
| 5 | Signature | `ed25519:<base64>` | Page-level Ed25519 signature |
| 6 | Last modified | ISO 8601 timestamp | When the page was last updated |

### Hard requirements for indexed pages

The engine rejects any entry that fails format-level validation:

- The `URL` must start with `jrg://` and contain a path
- The `Title` must be non-empty
- The `Public key` must be a valid `ed25519:<base64>` string (32 bytes decoded)
- The `Signature` must be a valid `ed25519:<base64>` string (64 bytes decoded)
- The `Encryption` field must begin with a known suite name
  (`xchacha20poly1305` or `chacha20poly1305`)

Pages whose signatures fail cryptographic verification (the engine can fetch
the page source and check `signed-by:` + `signature:` metadata against the
public key in the jrgidx row) are also rejected. The v1.0 engine only does
format-level checks; cryptographic verification is the next milestone.

## Submission protocol

Domain owners submit their site by interacting with the search engine's JRG
pages. There is no API key, no account, and no payment. Ownership is proven
by DNS.

### 1. Submit the domain

```text
POST /actions/submit
domain=example.com
```

The engine generates a random 32-character hex token and returns a page
telling the owner to publish a DNS TXT record:

```text
# Domain Submitted: example.com

## Next Step: DNS Verification

Create a TXT record for `_jrg-verify.example.com` with the following value:

> a3b9c1d8e7f2... (the token)

? domain value="example.com"
! check label="Verify DNS" method="POST" target="/actions/check-verify"
```

The token and pending submission persist to `<data-dir>/submissions.json` so
the engine can survive restarts.

### 2. Publish the DNS TXT record

The owner creates one DNS record:

```text
_jrg-verify.example.com  IN  TXT  "a3b9c1d8e7f2..."
```

The underscore prefix is conventional and avoids collisions with regular
subdomains. The TXT value is the raw token, no quoting, no key=value.

### 3. Verify and index

```text
POST /actions/check-verify
domain=example.com
```

The engine:

1. Resolves TXT records for `_jrg-verify.example.com` via system DNS
2. If any record matches the pending token exactly, marks the domain verified
3. Tries to fetch `https://example.com/.well-known/jrgidx` (then HTTP)
4. Parses the response with `SearchIndexV1::from_index_text_v1`
5. Runs `validate_entry` over every row
6. Adds valid entries to the aggregate index
7. Persists `<data-dir>/index.jrgidx`

The owner sees a confirmation page with the count of indexed pages and the
count of skipped (invalid) entries.

### 4. Periodic re-indexing

The engine spawns a background thread (default every 6 hours, configurable
via `--reindex-hours N`, or disabled with `0`) that:

- Reads the current set of verified domains from `submissions.json`
- Re-fetches each domain's jrgidx
- Validates and re-adds entries
- Overwrites the on-disk `index.jrgidx` with the latest state

The thread is best-effort: failures are logged to stderr and the existing
index is kept if a refresh fails.

## Service endpoints

| Route | What it serves |
|-------|----------------|
| `GET /` or `/search.jrg` | Search form (`?q` input + `!do-search` button) |
| `GET /search.jrg?q=...` | Results page for the query |
| `GET /submit.jrg` | Domain submission form |
| `GET /status.jrg` | Engine stats: total pages, verified domains |
| `POST /actions/submit` | Submit a domain, receive a verification token |
| `POST /actions/check-verify` | Check DNS, fetch jrgidx, index pages |
| `POST /actions/search` | Search the index (form-driven) |
| `GET /actions/verify` | Show verify form or pending instructions |

The engine is a standard JRG protocol server — connect with the
`jaringan-browser` CLI, the HTTP→JRG gateway, or any HTTP/1.1 client.

## Persistence

| File | Contents |
|------|----------|
| `<data-dir>/index.jrgidx` | v1.0 jrgidx of all validated entries from all domains |
| `<data-dir>/submissions.json` | Map of domain → `{ token, created_at, verified }` |

The engine loads both on startup, writes both on every state change, and the
background re-index thread rewrites `index.jrgidx` on each cycle.

## Data model

In Rust, the engine consumes `jaringan_core::IndexEntryV1` and
`jaringan_core::SearchIndexV1`. These are exposed by the core crate so other
tools can read or write jrgidx files without depending on `jaringan-search`.

```rust
pub struct IndexEntryV1 {
    pub url: String,
    pub title: String,
    pub public_key: String,         // "ed25519:<base64>"
    pub encryption: String,         // "xchacha20poly1305;key-id=key1"
    pub signature: String,          // "ed25519:<base64>"
    pub last_modified: String,      // ISO 8601
}

pub struct SearchIndexV1 { /* entries: Vec<IndexEntryV1> */ }
```

`SearchIndexV1::search(query)` scores entries by title (×10) and URL (×5) and
returns sorted results. The search engine uses the same scorer; richer
field-level scoring is left for a future version that has access to full
page bodies.

## Self-hosting

The engine is a single Rust binary. Run it on any host and point a
hostname at it:

```bash
jaringan-search serve \
    --bind 127.0.0.1:7071 \
    --data-dir /var/lib/jaringan-search \
    --domain search.example.com \
    --reindex-hours 6
```

Place a reverse proxy (such as `jaringan-proxy`) in front of it to route
the public hostname to this backend, or bind directly if the port is
public. Native JRG clients can connect directly; the engine's
dual-protocol mode handles both raw JRG and HTTP request formats.

## Limitations and future work

- **No page-level signature verification** — v1.0 trusts the jrgidx; v1.1 will
  fetch each page and verify `signed-by:` + `signature:` against the
  declared public key.
- **No revocation** — a domain's pages stay in the index until the next
  re-index cycle excludes them (if the jrgidx is removed) or until the
  submission is manually purged.
- **Title and URL only** — full-text scoring is not possible without fetching
  every page, which costs too much for the current architecture.
- **No ranking signals beyond presence** — no PageRank, no click tracking, no
  freshness weighting. The order is title-match-score, then title, then URL.
- **One jrgidx URL convention** — only `/.well-known/jrgidx` is checked. Other
  paths are not honored.

## See also

- `docs/spec/jrg-security.md` — Ed25519 page signatures
- `docs/spec/jrg-encryption.md` — XChaCha20-Poly1305 capability metadata
- `docs/spec/jrg-proxy.md` — the vhost reverse proxy for routing
  traffic to this engine and other services on a single public port
