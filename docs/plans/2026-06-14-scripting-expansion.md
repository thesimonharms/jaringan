# Scripting Expansion Implementation Plan

> **For Hermes:** Use subagent-driven-development to implement this plan task-by-task.

**Goal:** Expand scripting capabilities with script form definitions, a build CLI, session-level store, compiled module caching, and server-side script execution.

**Architecture:** All 5 features build on the existing WASM runtime (`jaringan-script`), SDK (`jaringan-script-sdk`), and browser CLI (`jaringan-browser`). Each feature is self-contained and testable independently.

**Tech Stack:** Rust, wasmtime (WASM runtime), WAT/WebAssembly, wasm32-unknown-unknown target, existing SDK macro system.

---

## Wave 1: Script Form Definitions (#1) + Module Cache (#6)

### Task 1.1: Add `form()` export to WASM script protocol

**Objective:** Script modules can optionally export a `form()` function that returns JSON field definitions, allowing the runtime to discover form inputs before execution.

**Files:**
- Modify: `crates/jaringan-script/src/lib.rs` (ScriptInput, ScriptOutput, execute_with_bridge)
- Modify: `crates/jaringan-script-sdk/src/lib.rs` (add `export_form!` macro or helper)

**Step 1: Update SDK — add `form()` support**

Add a `export_form_process!` macro or separate `form()` export convention to `jaringan-script-sdk/src/lib.rs`:

```rust
/// Declare both `process` and `form` exports.
/// `form_fn` takes () and returns a JSON array of field definitions.
/// `process_fn` takes input string, returns output string (existing behavior).
#[macro_export]
macro_rules! export_form_process {
    ($form_fn:expr, $process_fn:expr) => {
        // form() export
        #[no_mangle]
        pub unsafe extern "C" fn form() -> i64 {
            let json = $form_fn();
            let bytes = json.as_bytes();
            let len = bytes.len() as i64;
            // Write JSON at offset 65536, return (ptr << 32 | len)
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), 0x10000 as *mut u8, bytes.len());
            0x10000i64 << 32 | len
        }
        // process() export (existing behavior)
        export_process!($process_fn);
    };
}
```

Actually, the SDK already uses a different approach — `export_process!` writes to 65536 with a 4-byte length prefix. Let me keep that consistent. The form export should use the same memory layout.

Let me think about this more carefully. The current SDK uses:
- `sdk::export_process!(script_main)` which creates `process(input_ptr, input_len) -> i32`
- The return is a pointer to a 4-byte LE length prefix + JSON body at 65536

For `form()`, I'll use a simpler approach: `form() -> i64` where the return is a packed pointer (high 32 bits = pointer, low 32 bits = length). The runtime reads this to get the form field definitions.

**Step 2: Update runtime to check for `form()` export**

In `execute_with_bridge`, after loading the module and before calling `process()`:

```rust
/// Get form field definitions from a WASM module (if it exports `form()`).
pub fn get_script_form(
    &self,
    wasm_binary: &[u8],
) -> Result<Option<Vec<ScriptInputField>>, WasmError> {
    // Try to get the `form` export
    // If not present, return None
    // If present, call it and parse the result
}
```

**Step 3: Update ScriptInput to carry form fields**

The runtime should call `form()` first, then pass those fields to the page for rendering.

```rust
pub struct ScriptInput {
    // ... existing fields ...
    pub form_fields: Option<Vec<ScriptInputField>>,
}
```

**Step 4: Browser integration**

In `jaringan-browser/src/main.rs`, when executing scripts, pass form fields from `form()` to the page rendering.

**Step 5: Tests**

Add test with a WAT module that exports `form()` and verify the runtime correctly extracts field definitions.

**Step 6: Commit**

```bash
git add crates/jaringan-script/ crates/jaringan-script-sdk/
git add -A
git commit -m "feat(script): add script form definitions (#1)"
```

---

### Task 1.2: Module compilation cache

**Objective:** Cache compiled WASM modules by hash so re-visiting a page doesn't re-compile.

**Files:**
- Modify: `crates/jaringan-script/src/lib.rs` (WasmRuntime)

**Step 1: Add module cache to WasmRuntime**

```rust
use std::collections::HashMap;

pub struct WasmRuntime {
    engine: Engine,
    module_cache: std::sync::Mutex<HashMap<u64, wasmtime::Module>>,
}
```

Key by a hash of the WASM binary bytes. On `execute_with_bridge`:
1. Compute hash of `wasm_binary`
2. Check cache
3. If miss, call `Module::new` and insert
4. If hit, use cached module

**Step 2: Tests**

The module cache is transparent to callers, so the existing tests should still pass. Add a test that runs two scripts with the same WASM binary and verifies the module is only compiled once (hard to observe directly but we can verify correctness).

**Step 3: Commit**

```bash
git add crates/jaringan-script/
git commit -m "feat(script): cache compiled WASM modules by hash (#6)"
```

---

## Wave 2: Session-Level Store (#5)

### Task 2.1: Make store persist across navigations

**Objective:** The `store_get`/`store_set` host functions currently use a per-page `HashMap` in `BridgeState`. Make this persist across navigations by keying on origin.

**Files:**
- Modify: `crates/jaringan-script/src/lib.rs` (BridgeState store handling)
- Modify: `crates/jaringan-script/src/bridge.rs` (add session store)
- Modify: `crates/jaringan-browser/src/main.rs` (pass session store to bridge)

**Step 1: Create a session-level store**

In `jaringan-script/src/bridge.rs`, add a shared store:

```rust
pub type SharedStore = Arc<std::sync::Mutex<HashMap<String, String>>>;

pub fn new_shared_store() -> SharedStore {
    Arc::new(std::sync::Mutex::new(HashMap::new()))
}
```

**Step 2: Update BridgeState**

Add `session_store: Option<SharedStore>` to `BridgeState`. The `store_get`/`store_set` host functions should use this instead of the per-page `self.store`.

**Step 3: Update browser to maintain a shared store**

In `jaringan-browser`, create one `SharedStore` per browser session and pass it to every `BridgeState`.

**Step 4: Tests**

Write a test that creates a runtime, executes script A that sets a value, then script B that reads it, verifying the value persists.

**Step 5: Commit**

```bash
git add crates/jaringan-script/ crates/jaringan-browser/
git commit -m "feat(script): session-level store persists across navigations (#5)"
```

---

## Wave 3: Script Build CLI (#4)

### Task 3.1: Add `script build` subcommand

**Objective:** Add a `jaringan-browser script build` subcommand that compiles Rust/WAT source to WASM and optionally generates a .jrg page template.

**Files:**
- Modify: `crates/jaringan-browser/src/main.rs` (add `Script` subcommand)
- Create: `crates/jaringan-browser/src/script_build.rs` (build logic)

**Step 1: Add `Script` subcommand to CLI**

```rust
#[derive(Debug, Subcommand)]
enum Command {
    // ... existing commands ...
    
    /// Build WASM scripts for Jaringan pages.
    Script {
        #[command(subcommand)]
        command: ScriptCommand,
    },
}

#[derive(Debug, Subcommand)]
enum ScriptCommand {
    /// Compile a Rust crate to WASM and optionally generate a .jrg template.
    Build {
        /// Path to the Rust crate root (containing Cargo.toml).
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Output .wasm file path (default: <crate-name>.wasm).
        #[arg(long)]
        output: Option<PathBuf>,
        /// Generate a .jrg page template with the WASM embedded as base64.
        #[arg(long)]
        template: Option<PathBuf>,
        /// Title for the generated .jrg template.
        #[arg(long, default_value = "Scripted Page")]
        title: String,
        /// Label for the script block in the template.
        #[arg(long, default_value = "Script")]
        label: String,
    },
}
```

**Step 2: Implement build logic**

```rust
fn cmd_script_build(path: &Path, output: Option<&Path>, template: Option<&Path>, title: &str, label: &str) -> anyhow::Result<()> {
    // 1. Ensure wasm32-unknown-unknown target is installed
    run_cargo(&["target", "add", "wasm32-unknown-unknown"])?;
    
    // 2. Build the crate
    run_cargo(&["build", "--target", "wasm32-unknown-unknown", "--release"])?;
    
    // 3. Find the .wasm file
    let wasm_path = find_wasm_output(path)?;
    
    // 4. Optionally copy to output path
    if let Some(out) = output {
        fs::copy(&wasm_path, out)?;
    }
    
    // 5. Optionally generate .jrg template
    if let Some(tpl) = template {
        let wasm_bytes = fs::read(&wasm_path)?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&wasm_bytes);
        let jrg = format!("# {title}\n\n~> {label}\n{b64}\n~<\n");
        fs::write(tpl, jrg)?;
    }
    
    Ok(())
}
```

**Step 3: Tests**

Add a test that uses the form-processor example crate:
1. Build the example WASM
2. Verify the .wasm file exists and has the right format
3. Verify the .jrg template is valid

**Step 4: Commit**

```bash
git add crates/jaringan-browser/
git commit -m "feat(browser): add script build subcommand (#4)"
```

---

## Wave 4: Server-Side Script Execution (#7)

### Task 4.1: Add script execution to search engine re-indexing

**Objective:** The search engine (`jaringan-search`) should run WASM scripts on pages during indexing so the index reflects rendered (script-processed) content, not raw page source.

**Files:**
- Modify: `crates/jaringan-search/src/lib.rs` (add script execution during indexing)
- Modify: `crates/jaringan-search/Cargo.toml` (add `jaringan-script` dependency)

**Step 1: Add script execution when building index entries**

When the search engine indexes a page:
1. Parse the document
2. If the document has Script blocks, execute them using `WasmRuntime`
3. Index the *rendered* output blocks rather than the raw source

For the bridge, scripts during indexing get:
- `fetch_fn`: a simple HTTP fetch (using reqwest blocking) for non-local URLs
- `log_fn`: stdout logging
- `store`: empty (no session store for indexing)
- `resolve_fn`: for jrg:// URLs, resolve via the search engine's own resolver

**Step 2: Add script-execution flag**

Add `--execute-scripts` flag to the search engine:

```rust
struct SearchConfig {
    execute_scripts: bool,
}
```

When false (default), behavior is unchanged — existing pages without scripts are indexed immediately.

**Step 3: Tests**

Add integration tests:
1. Index a page with a WAT identity script, `execute_scripts: true`
2. Verify the indexed content matches the rendered output (not raw source)

**Step 4: Commit**

```bash
git add crates/jaringan-search/
git commit -m "feat(search): server-side script execution for re-indexing (#7)"
```

---

## Verification

After all waves:

```bash
cargo test --workspace 2>&1 | tail -20
cargo build --workspace 2>&1 | tail -5
```

Check for:
- No new warnings
- No regressions in existing tests
- All new tests pass
