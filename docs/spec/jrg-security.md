# Jaringan Security 0.1

Jaringan uses one URL scheme: `jrg://`.

Signing, encryption, and trust indicators are features of the browser/protocol stack, not reasons to invent a second scheme. A browser should show whether a page is secure or not secure while still allowing ordinary unsigned pages to exist.

## Principles

- `jrg://` is secure-capable by default.
- Unsigned pages are valid and render normally.
- Signed pages are verified against public keyrings.
- Public keyrings are the signing authority; Jaringan tooling is not a centralized security gatekeeper.
- The browser displays security state clearly instead of hiding the page.

## Prototype signature metadata

Signatures live in the trailing metadata block:

```text
~~~~~
title: Signed page
signed-by: alice
signature: ed25519:<base64-signature>
```

The signature covers the complete source text with the `signature:` metadata line omitted. This means visible content and other metadata are covered, while the signature can still be embedded in the page.

## Browser indicators

Current browser states:

- `secure: signed by <signer>` — signature verifies against the configured keyring.
- `not secure: unsigned` — no signing metadata; page is still allowed.
- `not secure: unknown signer <signer>` — page is signed but the signer is not in the keyring.
- `not secure: <reason>` — malformed or invalid signature metadata.

## Keyrings

The browser loads a user keyring from:

```text
~/.config/jaringan/keyring
```

Set `JARINGAN_KEYRING=/path/to/keyring` to use a different file.

The keyring is human-editable. Blank lines and `#` comments are ignored. Each trusted signer uses one line:

```text
alice ed25519:<base64-public-key>
```

Malformed keyring files do not block browsing; the browser prints a warning and continues with an empty keyring, so affected signed pages show as unknown signers instead of secure.

## Non-goals

- No centralized certificate authority requirement.
- No forced rejection of unsigned pages.
- No separate `jrgs://` or similar scheme just because a page is signed.

## Future work

- Load user/system keyring files in the browser and CLI.
- Add a page-signing CLI helper.
- Add encrypted transport while keeping `jrg://` as the user-facing scheme.
- Add origin-scoped action permissions and clearer prompts for side effects.
