# Jaringan Protocol 0.1

Jaringan's protocol exists to fetch terminal-native `.jrg` pages for both humans and AI agents. Version 0.1 defines URL semantics, status concepts, response tags, local resolver behavior, a tiny TCP wire transport, and an optional encrypted TCP framing mode.

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

The first transport is intentionally tiny and line-oriented. `jrg://` remains the single scheme as security features evolve; signing or encryption should not create a second protocol name. Browsers surface whether a page is secure or not secure instead of refusing unsigned content by default.

All examples in this spec use port `7070` — the conventional default for JRG servers. The `jaringan-browser serve` command and all CLI fetch/get examples follow this convention. When running multiple services on the same host, increment the port (e.g. `7071`, `7072`).

Client request:

```text
GET jrg://127.0.0.1:7070/protocol.jrg?view=ai#top JRG/0.1
Host: 127.0.0.1:7070

```

Action POST request:

```text
POST jrg://127.0.0.1:7070/actions/search JRG/0.1
Host: 127.0.0.1:7070
Action-Token: demo-search
Content-Length: 7

q=laksa
```

The `Action-Token` value is resolved from the button's `auth` attribute — the browser treats `auth` as a **service name** and looks up the real token under `~/.config/jaringan/tokens/<service>/token`. If no stored token is found, the action proceeds without the header (assuming the resolver allows unauthenticated requests). The token is issued by a server returning a `Tag-Token` response header (see [Token registration flow](#token-registration-flow) below).

The request target may be either a full `jrg://` URL or an absolute path when a `Host:` header is present. `Action-Token:` is optional at the wire level, but side-effectful resolvers can require it before executing an action.

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
cargo run -p jaringan-browser -- get --follow jrg://127.0.0.1:7070/
```

TCP clients use bounded connect/read/write timeouts so an unresponsive origin cannot hang indefinitely.

## Request

The in-process request model is currently:

```rust
Request {
    method: RequestMethod,
    url: JaringanUrl,
    body: String,
}
```

Supported request methods are `GET` and `POST`. `POST` bodies use URL-encoded form payloads in the prototype. Future network transports can add agent hints, accepted render capabilities, authentication, and richer encrypted handshakes without changing page syntax.

## Encryption capabilities

Jaringan keeps encryption under the same `jrg://` scheme. The protocol crate provides reusable encryption primitives, compact capability metadata, and an optional encrypted TCP frame wrapper for pre-shared-key deployments.

The first supported encryption suite is:

```text
xchacha20poly1305
```

Capability header values use this shape:

```text
xchacha20poly1305; key-id=key-2026
```

Encrypted payloads carry a suite, base64 nonce, and base64 ciphertext. XChaCha20-Poly1305 provides authenticated encryption and accepts associated data so callers can bind ciphertext to context such as the canonical `jrg://` URL.

Encrypted TCP frames wrap a normal Jaringan wire request or response:

```text
JRG-ENC/0.1
Content-Encryption: xchacha20poly1305; key-id=local-dev
Nonce: <base64-24-byte-nonce>
Content-Length: <base64-ciphertext-length>

<base64-ciphertext-and-auth-tag>
```

The browser CLI can serve and fetch encrypted frames when both peers share `JARINGAN_ENCRYPTION_KEY_HEX` and the same `--encrypted-key-id`.

## Security indicators and signatures

Jaringan is secure-capable under the same `jrg://` scheme. Security is a page/browser state, not a separate URL scheme.

- Unsigned pages are valid and render normally.
- Signed pages declare `signed-by:` and `signature:` metadata after `~~~~~`.
- The browser verifies signatures against its configured public keyring and shows `secure: signed by <name>` or `not secure: ...`.
- Public keyrings are the signing authority. Jaringan tooling should not be a centralized security gatekeeper.

Prototype metadata shape:

```text
~~~~~
title: Signed page
signed-by: alice
signature: ed25519:<base64-signature>
```

The Ed25519 signature covers the full source with the `signature:` metadata line omitted. This lets signatures cover visible content and metadata such as title while allowing the signature itself to live inside page metadata.

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

Redirects and tokens are represented as tags instead of magic browser behavior:

```rust
ResponseTag::Redirect { target }
ResponseTag::Token { service, value, expires_at }
```

- `Tag-Redirect` — the prototype terminal browser follows redirect tags automatically for `jrg://` pages, resolving relative redirect targets against the current page URL and stopping after a small redirect limit. The lower-level `get` command prints the response as-is by default; `get --follow` follows redirect tags before printing the final response.
- `Tag-Token` — issued by a server-side registration or login endpoint to grant a capability token. The three fields map to HTTP-style headers:
  - `service` — the service name the token is valid for. Empty means the host is the service.
  - `value` — the opaque token string (e.g., a UUID).
  - `expires_at` — optional expiry (e.g., `2026-12-31T23:59:59Z`). Omitted or empty means never expires.

### Token registration flow

A Jaringan page can require auth by adding the `auth` attribute to its buttons:

```text
! post label="Post" method="POST" target="/actions/post" auth="microblog"
```

The `auth` value is a **service name** — not the token itself. The browser looks up the real token from `~/.config/jaringan/tokens/<service>/token` when the button is activated.

For a user to obtain a token, the server must provide a register or login page. The `jaringan auth register` CLI command:

1. Fetches the register page over JRG TCP
2. Fills any provided form fields (`-f username=Simon`) and POSTs to `/actions/register`
3. Checks the response for a `Tag-Token` header
4. Stores the token value at `~/.config/jaringan/tokens/<service>/token`

**Server authors** implement registration by returning `Tag-Token` from the register endpoint:

```text
JRG/0.1 200 OK
Content-Type: text/jrg; charset=utf-8
Tag-Token: microblog.localhost:7072; value=550e8400e29b41d4a716446655440000
# ✅ Registered!
```

The `service` field in `Tag-Token` determines where the CLI stores the token. When a button has `auth="microblog"`, the browser searches stored token directories for any name starting with `microblog.` or `microblog:`, finding `microblog.localhost:7072`.

**Revocation** — `jaringan auth revoke <service>` removes the stored token directory. Servers may also expire tokens by re-checking them against their own store.

### Reference implementation: microblog demo

The full source is at `crates/jaringan-demo-microblog/src/main.rs`. Key pieces:

**Page templates** — buttons carry `auth` as a service name:

```rust
const REGISTER_JRG: &str = r#"...
!register label="📝 Sign Up" target="/actions/register" method="POST" auth="microblog"
..."#;

const MICROBLOG_JRG: &str = r#"...
!post label="📤 Post" target="/actions/post" method="POST" auth="microblog"
..."#;
```

Both buttons share `auth="microblog"`. The register button doesn't need an existing token — the browser sends no `Action-Token` since none is stored yet, and the server treats a missing token as a registration request. The post button does require a token, so the user must register first.

**Register handler** — generates a random token, stores it, returns `Tag-Token`:

```rust
fn handle_register(&self, body: &str) -> Response {
    let token_bytes: [u8; 16] = rand::random();
    let token = hex::encode(token_bytes);
    let expires = Instant::now() + Duration::from_secs(3600);

    self.tokens.lock().unwrap().insert(
        token.clone(),
        TokenInfo { username, expires_at: expires },
    );

    Response::page(StatusCode::Ok, body)
        .with_tag(ResponseTag::Token {
            service: format!("microblog.localhost:{}", self.port),
            value: token,
            expires_at: None,
        })
}
```

The `service` field — `"microblog.localhost:7072"` — is where the CLI stores the token. When a button has `auth="microblog"`, the browser finds this directory by prefix matching.

**Post handler** — reads `action_token` from the wire-level request, validates against stored tokens:

```rust
fn handle_post(&self, request: &Request) -> Response {
    let token = request.action_token.as_deref().unwrap_or("");
    let tokens = self.tokens.lock().unwrap();
    let token_info = tokens.get(&token).cloned();
    drop(tokens);

    match token_info {
        Some(info) if info.expires_at > Instant::now() => {
            // store post, return updated feed...
        }
        _ => {
            // return page with error prompting registration
        }
    }
}
```

**Routing** — `PageResolver::fetch()` dispatches by method and path:

```rust
impl PageResolver for MicroblogResolver {
    fn fetch(&self, request: &Request) -> Result<Response, ResolveError> {
        match (request.method, request.url.path()) {
            (RequestMethod::Get, "/register") =>
                Ok(Response::page(StatusCode::Ok, REGISTER_JRG)),
            (RequestMethod::Post, "/actions/register") =>
                Ok(self.handle_register(&request.body)),
            (RequestMethod::Post, "/actions/post") =>
                Ok(self.handle_post(request)),
            // ...
        }
    }
}
```

The resolver is passed to `jaringan_protocol::serve()` which handles the TCP transport. Tags like `ResponseTag::Token` are serialized automatically to `Tag-*` response headers.

## Local resolver

`LocalFileResolver` maps a `jrg://host/path` to a filesystem root for tests and local serving:

- `/` -> `<root>/index.jrg`
- `/foo/` -> `<root>/foo/index.jrg`
- `/foo.jrg` -> `<root>/foo.jrg`
- `/foo` -> `404 Not Found`

Query strings and fragments are accepted by the URL parser but ignored by the local file mapping.

The prototype local resolver also includes one demo action endpoint for M4 experimentation:

- `POST /actions/search` requires `Action-Token: demo-search`, then records `POST /actions/search <payload>` to `<root>/.jrg-actions.log`.
- Missing or invalid action tokens return `403 Forbidden` and do not write the side-effect log.
- Authorized requests return a generated `text/jrg` search-results page echoing the submitted `q` field.

## Not in 0.1

- No redirect safety UI yet; the prototype browser and `get --follow` follow redirects automatically.
- No content negotiation beyond basic content type enums and encryption capability values.
- No automatic encryption key exchange yet; encrypted TCP currently uses pre-shared keys.
