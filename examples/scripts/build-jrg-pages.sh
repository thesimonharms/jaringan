#!/usr/bin/env bash
# Build .jrg pages with embedded WASM scripts and trigger content
set -euo pipefail

BASE="$(dirname "$0")"

build_jrg() {
  local name="$1"
  local label="$2"
  local content="$3"
  local output="$BASE/${name}.jrg"
  
  # Read base64 as a single unwrapped line
  local b64
  b64=$(tr -d '\n' < "$BASE/$name.wasm.b64")
  
  cat > "$output" << JRG_EOF
$content

~> $label
$b64
~<
JRG_EOF

  local size
  size=$(wc -c < "$output")
  echo "$output — ${size} bytes"
}

# ── dynamic-include.jrg ─────────────────────────────────────────────────

build_jrg "dynamic-include" \
  "Dynamic Include" \
  "# Dynamic Include Demo

The script below scans all paragraph blocks for lines starting with \`include: <url>\`.
It fetches that URL and splices the response content in place of the line.

include: jrg://examples/index.jrg

---

## How it works

1. The parser decodes the base64 WASM from the \`~>...~<\` block
2. The runtime calls \`process(json)\` with the full page content
3. The script finds the \`include: ...\` line and calls \`jaringan.fetch\`
4. The host bridge resolves \`jrg://\` URLs via the gateway and returns JSON
5. The script parses the response and inserts the fetched blocks"

# ── data-enrichment.jrg ─────────────────────────────────────────────────

build_jrg "data-enrichment" \
  "Data Enrichment" \
  "# Data Enrichment Demo

This page demonstrates fetching external JSON data and rendering it inline.

> **Note:** The current ScriptInput does not yet pass page metadata (YAML frontmatter)
> through to the WASM script, so \`enrich_url\` is not available. The script correctly
> reports this — it's a demonstration of the script executing and producing
> conditional output based on available data.

Once page metadata is wired into the bridge, metadata such as:

\`\`\`
~~~~~
enrich_url: https://httpbin.org/uuid
~~~~~
\`\`\`

would allow the script to fetch enrichment data from that URL and render a table."

# ── form-processor.jrg ─────────────────────────────────────────────────

build_jrg "form-processor" \
  "Form Processor" \
  "# Form Processor Demo

This page demonstrates client-side form validation and submission via WASM.

Enter values in the fields below. The script validates required fields,
then fetches an endpoint and displays the result.

?name label=\"Name\" placeholder=\"Enter your name\"
?email label=\"Email\" placeholder=\"user@example.com\"

---

## Behavior

- Blank **Name** or **Email** fields trigger a validation error
- Both filled → the script fetches https://httpbin.org/post and shows the response
- The WASM binary is embedded as base64 between the \`~>\` and \`~<\` markers

---

## Source

The WASM was compiled from Rust using \`jaringan-script-sdk\` (a \`#![no_std]\` crate
with bump allocator and host-import wrappers). Build with:

\`\`\`bash
cd examples/scripts/form-processor
cargo build --target wasm32-unknown-unknown --release
\`\`\`"
