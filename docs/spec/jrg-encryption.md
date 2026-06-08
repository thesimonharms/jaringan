# Jaringan Encryption 0.1

Jaringan keeps `jrg://` as the single user-facing scheme while adding encryption capabilities under the protocol layer. Encryption is a transport/content capability, not a reason to introduce `jrgs://`.

## Scope

The current implementation adds reusable protocol primitives for encrypted payloads and capability negotiation metadata. It does not yet replace the prototype TCP stream with an encrypted handshake.

## Suite

The first supported suite is:

```text
xchacha20poly1305
```

It uses:

- 32-byte symmetric keys.
- 24-byte nonces.
- XChaCha20-Poly1305 authenticated encryption.
- Associated data to bind ciphertext to protocol context such as the request URL or header transcript.

## Capability header shape

Encryption capability values are serialized as compact header values:

```text
xchacha20poly1305; key-id=key-2026
```

`key-id` identifies which pre-shared or negotiated key a peer should use. It is not the key itself.

Future wire headers can use this value in fields such as:

```text
Accept-Encryption: xchacha20poly1305; key-id=key-2026
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

## Associated data

Callers must pass associated data when encrypting and decrypting. For page responses, a good prototype associated-data value is the canonical `jrg://` URL. Future handshakes should bind more context: method, host, path, protocol version, and selected capabilities.

## Non-goals for this slice

- No automatic key exchange yet.
- No encrypted TCP serving/client path yet.
- No centralized encryption authority.
- No separate secure URL scheme.

## Future work

- Add key agreement, likely X25519 or Noise.
- Add encrypted TCP request/response framing.
- Surface encrypted-transport state in the browser security indicator separately from page signatures.
- Bind signatures and encryption state into a clearer trust UI.
