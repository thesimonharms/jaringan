# Jaringan Encryption 0.1

Jaringan keeps `jrg://` as the single user-facing scheme while adding encryption capabilities under the protocol layer. Encryption is a transport/content capability, not a reason to introduce `jrgs://`.

## Scope

The current implementation provides:

- reusable XChaCha20-Poly1305 payload encryption primitives;
- compact encryption capability metadata;
- an encrypted TCP request/response framing mode using a pre-shared symmetric key.

It does not yet implement automatic key agreement or a Noise/X25519 handshake. The first wire mode is intentionally simple and local-dev friendly: both peers already know the same 32-byte key and refer to it by `key-id`.

## Suite

The first supported suite is:

```text
xchacha20poly1305
```

It uses:

- 32-byte symmetric keys;
- 24-byte nonces;
- XChaCha20-Poly1305 authenticated encryption;
- associated data to bind ciphertext to protocol context.

## Capability header shape

Encryption capability values are serialized as compact header values:

```text
xchacha20poly1305; key-id=key-2026
```

`key-id` identifies which pre-shared or negotiated key a peer should use. It is not the key itself.

Wire frames use:

```text
Content-Encryption: xchacha20poly1305; key-id=key-2026
```

## Encrypted payload shape

The protocol model represents encrypted payloads as:

```rust
EncryptedPayload {
    suite: EncryptionSuite::XChaCha20Poly1305,
    nonce_base64: String,
    ciphertext_base64: String,
}
```

The ciphertext includes the Poly1305 authentication tag produced by the AEAD implementation.

## Encrypted TCP framing

Encrypted TCP uses the same TCP connection model as the plaintext prototype, but each request and response is wrapped in an encrypted frame.

Frame header:

```text
JRG-ENC/0.1
Content-Encryption: xchacha20poly1305; key-id=local-dev
Nonce: <base64-24-byte-nonce>
Content-Length: <base64-ciphertext-length>

<base64-ciphertext-and-auth-tag>
```

The decrypted request payload is the normal plaintext Jaringan wire request:

```text
GET jrg://127.0.0.1:7070/ JRG/0.1
Host: 127.0.0.1:7070

```

The decrypted response payload is the normal plaintext Jaringan wire response:

```text
JRG/0.1 200 OK
Content-Type: text/jrg; charset=utf-8

# Page
```

Plaintext request/response bytes are never written directly to the TCP stream in encrypted mode.

## Associated data

Encrypted TCP binds ciphertext to frame direction and selected capability:

```text
JRG-ENC/0.1 request; xchacha20poly1305; key-id=local-dev
JRG-ENC/0.1 response; xchacha20poly1305; key-id=local-dev
```

This prevents request frames from being replayed as responses and binds decryption to the expected `key-id`/suite.

## CLI prototype

Encrypted serving:

```bash
JARINGAN_ENCRYPTION_KEY_HEX=<64-hex-chars> \
  cargo run -p jaringan-browser -- serve docs/examples --bind 127.0.0.1:7070 --encrypted-key-id local-dev
```

Encrypted fetch:

```bash
JARINGAN_ENCRYPTION_KEY_HEX=<same-64-hex-chars> \
  cargo run -p jaringan-browser -- get --encrypted-key-id local-dev jrg://127.0.0.1:7070/
```

`JARINGAN_ENCRYPTION_KEY_HEX` is a raw 32-byte symmetric key encoded as 64 hex characters. Do not commit real keys. Use test-only keys for examples.

## Failure behavior

Peers reject frames when:

- the frame magic is not `JRG-ENC/0.1`;
- required frame headers are missing;
- `Content-Encryption` does not match the configured suite/key id;
- ciphertext authentication fails.

A wrong key causes decryption failure and the connection closes without returning plaintext content. Long-running encrypted servers treat bad encrypted frames as per-client failures and continue accepting later clients.

## Non-goals for this slice

- No automatic key exchange yet.
- No centralized encryption authority.
- No separate secure URL scheme.
- No browser/TUI transport-security indicator yet.

## Future work

- Add key agreement, likely X25519 or Noise.
- Add keyring-style management for encryption keys without environment variables.
- Surface encrypted-transport state in the browser security indicator separately from page signatures.
- Bind signatures and encryption state into a clearer trust UI.
