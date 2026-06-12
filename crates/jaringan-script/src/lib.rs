pub mod bridge;

pub use bridge::{BridgeState, read_string, write_error, write_json};
use serde::{Deserialize, Serialize};
use wasmtime::{AsContext, AsContextMut, Caller, Engine, Linker, Memory, Module, Store};

/// A single input field for a script's UI form.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptInputField {
    pub name: String,
    pub label: String,
    pub value: Option<String>,
    pub placeholder: Option<String>,
}

/// A block of rendered content produced by a script.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ScriptBlock {
    #[serde(rename = "heading")]
    Heading { level: u8, text: String },
    #[serde(rename = "paragraph")]
    Paragraph { text: String },
    #[serde(rename = "code")]
    Code { language: Option<String>, text: String },
    #[serde(rename = "list")]
    List { items: Vec<String>, ordered: bool },
    #[serde(rename = "table")]
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    #[serde(rename = "image")]
    Image { url: String, alt: Option<String> },
    #[serde(rename = "link")]
    Link { url: String, text: String },
    #[serde(rename = "quote")]
    Quote { text: String, attribution: Option<String> },
}

/// TUI context passed into the script about the current browser state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuiContext {
    pub current_url: Option<String>,
    pub current_title: Option<String>,
    pub scroll_offset: u64,
    pub selected_index: usize,
    pub mode: String,
}

/// Metadata attached to the script invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptMetadata {
    pub url: Option<String>,
    pub domain: Option<String>,
    pub title: Option<String>,
}

/// Full input payload sent into a WASM script.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptInput {
    pub title: Option<String>,
    pub inputs: Vec<ScriptInputField>,
    pub metadata: Option<ScriptMetadata>,
    pub blocks: Vec<ScriptBlock>,
    pub tui: Option<TuiContext>,
}

/// Output produced by a WASM script.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptOutput {
    pub blocks: Vec<ScriptBlock>,
}

/// Error type for WASM runtime operations.
#[derive(Debug, thiserror::Error)]
pub enum WasmError {
    #[error("WASM engine error: {0}")]
    Engine(#[from] wasmtime::Error),
    #[error("JSON serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Missing required WASM export: {0}")]
    MissingExport(String),
    #[error("Memory operation error: {0}")]
    Memory(String),
}

/// A thin wrapper around a wasmtime engine and store for executing
/// Jaringan scripts compiled to WebAssembly.
pub struct WasmRuntime {
    engine: Engine,
}

impl WasmRuntime {
    /// Create a new runtime with a default wasmtime engine.
    pub fn new() -> Result<Self, WasmError> {
        let engine = Engine::default();
        Ok(Self { engine })
    }

    /// Execute a WASM module with the given `ScriptInput` and return the
    /// `ScriptOutput`.
    ///
    /// The WASM module must export:
    ///   - `memory` — a linear memory (at least 2 pages)
    ///   - `process(i32, i32) -> i32` — receives (input_ptr, input_len) and
    ///     returns a pointer to the output buffer (4-byte little-endian length
    ///     prefix followed by JSON body).
    pub fn execute(
        &self,
        wasm_binary: &[u8],
        input: &ScriptInput,
    ) -> Result<ScriptOutput, WasmError> {
        self.execute_with_bridge(wasm_binary, input, BridgeState::empty())
    }

    /// Execute a WASM module with a custom `BridgeState`.  The bridge lets the
    /// WASM module import host functions (`jaringan.fetch`, `jaringan.log`,
    /// `jaringan.navigate`) that the runtime can wire up to real I/O.
    ///
    /// Same export requirements as [`execute`].
    pub fn execute_with_bridge(
        &self,
        wasm_binary: &[u8],
        input: &ScriptInput,
        bridge: BridgeState,
    ) -> Result<ScriptOutput, WasmError> {
        let mut store = Store::new(&self.engine, bridge);
        let module = Module::new(&self.engine, wasm_binary)?;
        let mut linker = Linker::new(&self.engine);

        // ── Register host functions unconditionally ──────────────────────────
        // The linker only resolves what the module actually imports, so it is
        // safe (and simpler) to always register these.

        // (import "jaringan" "fetch" (func (param i32 i32) (result i32)))
        linker.func_wrap(
            "jaringan",
            "fetch",
            |mut caller: Caller<'_, BridgeState>, url_ptr: i32, url_len: i32| -> i32 {
                let mem = caller
                    .get_export("memory")
                    .and_then(|e| e.into_memory())
                    .expect("memory export required for jaringan.fetch");

                // Read the URL string from WASM memory.
                let ctx = caller.as_context();
                let url = read_string(&mem, &ctx, url_ptr, url_len);
                drop(ctx);

                let state = caller.data(); // &BridgeState

                match state.fetch_fn {
                    Some(ref fetch) => match fetch(&url) {
                        Ok(result_json) => {
                            let mut ctx = caller.as_context_mut();
                            write_json(&mem, &mut ctx, &result_json)
                        }
                        Err(e) => {
                            let mut ctx = caller.as_context_mut();
                            write_error(&mem, &mut ctx, &e)
                        }
                    },
                    None => {
                        // No fetch function — return empty object.
                        let mut ctx = caller.as_context_mut();
                        write_json(&mem, &mut ctx, "{}")
                    }
                }
            },
        )?;

        // (import "jaringan" "log" (func (param i32 i32 i32 i32)))
        linker.func_wrap(
            "jaringan",
            "log",
            |mut caller: Caller<'_, BridgeState>, level_ptr: i32, level_len: i32, msg_ptr: i32, msg_len: i32| {
                let mem = caller
                    .get_export("memory")
                    .and_then(|e| e.into_memory())
                    .expect("memory export required for jaringan.log");

                let ctx = caller.as_context();
                let level = read_string(&mem, &ctx, level_ptr, level_len);
                let message = read_string(&mem, &ctx, msg_ptr, msg_len);
                drop(ctx);

                let state = caller.data();
                if let Some(ref log) = state.log_fn {
                    log(&level, &message);
                }
            },
        )?;

        // (import "jaringan" "navigate" (func (param i32 i32) (result i32)))
        linker.func_wrap(
            "jaringan",
            "navigate",
            |mut caller: Caller<'_, BridgeState>, url_ptr: i32, url_len: i32| -> i32 {
                let mem = caller
                    .get_export("memory")
                    .and_then(|e| e.into_memory())
                    .expect("memory export required for jaringan.navigate");

                let ctx = caller.as_context();
                let url = read_string(&mem, &ctx, url_ptr, url_len);
                drop(ctx);

                let state = caller.data();
                match state.navigate_fn {
                    Some(ref navigate) => match navigate(&url) {
                        Ok(result_json) => {
                            let mut ctx = caller.as_context_mut();
                            write_json(&mem, &mut ctx, &result_json)
                        }
                        Err(e) => {
                            let mut ctx = caller.as_context_mut();
                            write_error(&mem, &mut ctx, &e)
                        }
                    },
                    None => {
                        let mut ctx = caller.as_context_mut();
                        write_json(&mem, &mut ctx, "{}")
                    }
                }
            },
        )?;

        // ── Instantiate ──────────────────────────────────────────────────────
        let instance = linker.instantiate(&mut store, &module)?;

        // Extract the exported memory.
        let memory: Memory = instance
            .get_export(&mut store, "memory")
            .ok_or_else(|| WasmError::MissingExport("memory".into()))?
            .into_memory()
            .ok_or_else(|| WasmError::MissingExport("memory (expected Memory)".into()))?;

        // Serialize the input to JSON and write it into WASM memory.
        let input_json = serde_json::to_string(input)?;
        let input_bytes = input_json.as_bytes();
        let input_len = input_bytes.len() as i32;

        // Grow memory if needed to accommodate the input (plus a generous
        // output buffer).  Start writing at offset 0.
        let input_needed = input_len as u64 + 64 * 1024; // 64 KiB overhead
        let current_size = memory.size(&store) as u64 * (64 * 1024); // wasm page = 64 KiB
        if input_needed > current_size {
            let pages_needed = (input_needed - current_size + 65_535) / 65_536;
            memory.grow(&mut store, pages_needed as u64)?;
        }

        // Write input JSON at address 0.
        unsafe {
            let ptr = memory.data_ptr(&store).cast::<u8>();
            std::ptr::copy_nonoverlapping(input_bytes.as_ptr(), ptr, input_bytes.len());
        }

        // Call the WASM process function.
        let process_func = instance
            .get_export(&mut store, "process")
            .ok_or_else(|| WasmError::MissingExport("process".into()))?
            .into_func()
            .ok_or_else(|| WasmError::MissingExport("process (expected Func)".into()))?;

        let process_typed = process_func.typed::<(i32, i32), i32>(&store)?;

        let output_ptr = process_typed.call(&mut store, (0i32, input_len))?;

        // Read the output: 4-byte little-endian length prefix followed by JSON.
        let mem_data = memory.data(&store);

        if output_ptr < 0 || (output_ptr as u64 + 4) > mem_data.len() as u64 {
            return Err(WasmError::Memory(format!(
                "output pointer {} out of bounds",
                output_ptr
            )));
        }

        let output_ptr_u = output_ptr as usize;
        let len_bytes: [u8; 4] = mem_data[output_ptr_u..output_ptr_u + 4]
            .try_into()
            .map_err(|_| WasmError::Memory("failed to read output length".into()))?;
        let output_len = u32::from_le_bytes(len_bytes) as usize;

        let json_start = output_ptr_u + 4;
        let json_end = json_start + output_len;
        if json_end > mem_data.len() {
            return Err(WasmError::Memory(format!(
                "output JSON length {} exceeds memory bounds",
                output_len
            )));
        }

        let output_json = std::str::from_utf8(&mem_data[json_start..json_end])
            .map_err(|e| WasmError::Memory(format!("output is not valid UTF-8: {}", e)))?;

        let output: ScriptOutput = serde_json::from_str(output_json)?;
        Ok(output)
    }
}

/// Convert a slice of `jaringan_core::Block` into the WASM-friendly `ScriptBlock` format.
pub fn blocks_to_script_blocks(blocks: &[jaringan_core::Block]) -> Vec<ScriptBlock> {
    use jaringan_core::Block;
    blocks.iter().map(|b| match b {
        Block::Heading { level, text } => ScriptBlock::Heading { level: *level, text: text.clone() },
        Block::Paragraph(text) => ScriptBlock::Paragraph { text: text.clone() },
        Block::Link(link) => ScriptBlock::Link { url: link.target.clone(), text: link.label.clone() },
        Block::Input(i) => ScriptBlock::Paragraph { text: format!("{}: {}", i.label, i.value) },
        Block::Button(b) => ScriptBlock::Paragraph { text: format!("◉ {} -> {}", b.label, b.target) },
        Block::Image(img) => ScriptBlock::Image { url: img.source.clone(), alt: Some(img.alt.clone()) },
        Block::Quote(text) => ScriptBlock::Quote { text: text.clone(), attribution: None },
        Block::List(items) => ScriptBlock::List { items: items.clone(), ordered: false },
        Block::Rule => ScriptBlock::List { items: vec!["───────────────".into()], ordered: false },
        Block::Table(t) => ScriptBlock::Table { headers: t.headers.clone(), rows: t.rows.clone() },
        Block::Preformatted { code, language } => ScriptBlock::Code { language: language.clone(), text: code.clone() },
        Block::Script { wasm: _, label: _ } => ScriptBlock::Paragraph { text: "[script block]".into() },
    }).collect()
}

/// Convert `ScriptBlock`s back to `jaringan_core::Block`s, dropping Script blocks.
pub fn script_blocks_to_blocks(script_blocks: &[ScriptBlock]) -> Vec<jaringan_core::Block> {
    use jaringan_core::{Block, Link, Image, Table};
    script_blocks.iter().map(|sb| match sb {
        ScriptBlock::Heading { level, text } => Block::Heading { level: *level, text: text.clone() },
        ScriptBlock::Paragraph { text } => Block::Paragraph(text.clone()),
        ScriptBlock::Code { language, text } => Block::Preformatted { code: text.clone(), language: language.clone() },
        ScriptBlock::List { items, .. } => Block::List(items.clone()),
        ScriptBlock::Table { headers, rows } => Block::Table(Table { headers: headers.clone(), rows: rows.clone() }),
        ScriptBlock::Image { url, alt } => Block::Image(Image { source: url.clone(), alt: alt.clone().unwrap_or_default() }),
        ScriptBlock::Link { url, text } => Block::Link(Link { target: url.clone(), label: text.clone() }),
        ScriptBlock::Quote { text, .. } => Block::Quote(text.clone()),
    }).collect()
}

/// Run all WASM script blocks in a document against the document itself,
/// returning the transformed blocks (with Script blocks consumed).
///
/// Scripts run **sequentially**: each script sees the output of the previous
/// script as its input. The index of each Script block is re-discovered on
/// every iteration so that replacing `current_blocks` does not go stale.
///
/// When `bridge` is `Some`, scripts can invoke host functions (fetch, navigate,
/// log) through the provided [`BridgeState`]; otherwise those imports fall back
/// to no-ops.
pub fn execute_document_scripts(
    runtime: &WasmRuntime,
    doc: &jaringan_core::Document,
    bridge: Option<&BridgeState>,
) -> Result<Vec<jaringan_core::Block>, WasmError> {
    use jaringan_core::Block;

    let mut current_blocks = doc.blocks.clone();

    // Loop: find the FIRST Script block in the current blocks each time.
    // This avoids stale indices when a previous script replaces all blocks.
    loop {
        let script_idx = match current_blocks.iter().position(|b| {
            matches!(b, jaringan_core::Block::Script { .. })
        }) {
            Some(idx) => idx,
            None => break, // no more scripts — done
        };

        let Block::Script { wasm, .. } = &current_blocks[script_idx] else {
            unreachable!("position() returned a Script index");
        };

        let script_blocks = blocks_to_script_blocks(&current_blocks);

        let input = ScriptInput {
            title: doc.title().map(|s| s.to_owned()),
            inputs: current_blocks.iter().filter_map(|b| {
                if let jaringan_core::Block::Input(i) = b {
                    Some(ScriptInputField {
                        name: i.name.clone(),
                        label: i.label.clone(),
                        value: Some(i.value.clone()),
                        placeholder: i.placeholder.clone(),
                    })
                } else { None }
            }).collect(),
            metadata: None,
            blocks: script_blocks,
            tui: None,
        };

        let output = if let Some(b) = bridge {
            runtime.execute_with_bridge(wasm, &input, b.clone())?
        } else {
            runtime.execute(wasm, &input)?
        };

        current_blocks = script_blocks_to_blocks(&output.blocks);
    }

    Ok(current_blocks)
}
#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY_WAT: &str = r#"
(module
  (memory (export "memory") 2)
  (func (export "process") (param i32 i32) (result i32)
    ;; output = 4-byte LE length prefix + input JSON body
    ;; write at offset 65536 (end of page 1)
    (i32.store (i32.const 65536) (local.get 1))
    (memory.copy (i32.const 65540) (i32.const 0) (local.get 1))
    (i32.const 65536)
  )
)
"#;

    const BRIDGE_TEST_WAT: &str = r#"
(module
  (import "jaringan" "fetch" (func $fetch (param i32 i32) (result i32)))
  (import "jaringan" "log" (func $log (param i32 i32 i32 i32)))
  (import "jaringan" "navigate" (func $navigate (param i32 i32) (result i32)))
  (memory (export "memory") 2)

  ;; Store a URL string at offset 8000 for the test
  (data (i32.const 8000) "jrg://example/test.jrg")
  (data (i32.const 8022) "info")
  (data (i32.const 8027) "Hello from WASM")

  (func (export "process") (param $input_ptr i32) (param $input_len i32) (result i32)
    ;; Call fetch with url at offset 8000, length 22
    (call $fetch (i32.const 8000) (i32.const 22))
    ;; Drop the result (just testing it doesn't crash)
    drop

    ;; Call log with level="info" at offset 8022, msg at offset 8027
    (call $log (i32.const 8022) (i32.const 4) (i32.const 8027) (i32.const 15))

    ;; Call navigate
    (call $navigate (i32.const 8000) (i32.const 22))
    drop

    ;; Identity: return input as output
    (i32.store (i32.const 65536) (local.get $input_len))
    (memory.copy (i32.const 65540) (i32.const 0) (local.get $input_len))
    (i32.const 65536)
  )
)
"#;

    #[test]
    fn wasm_runtime_identity_roundtrip() {
        let wasm_binary = wat::parse_str(IDENTITY_WAT).unwrap();
        let runtime = WasmRuntime::new().unwrap();
        let input = ScriptInput {
            title: Some("Test".into()),
            inputs: vec![],
            metadata: None,
            blocks: vec![ScriptBlock::Heading {
                level: 1,
                text: "Hello".into(),
            }],
            tui: None,
        };
        let output = runtime.execute(&wasm_binary, &input).unwrap();
        assert_eq!(output.blocks.len(), 1);
    }

    #[test]
    fn execute_wat_script_on_document() {
        let wasm = wat::parse_str(IDENTITY_WAT).unwrap();
        let runtime = WasmRuntime::new().unwrap();

        // Simulate what execute_document_scripts does
        let input = ScriptInput {
            title: Some("Scripted Page".into()),
            inputs: vec![ScriptInputField {
                name: "username".into(),
                label: "Name".into(),
                value: Some("Simon".into()),
                placeholder: None,
            }],
            metadata: None,
            blocks: vec![
                ScriptBlock::Heading { level: 1, text: "Scripted Page".into() },
                ScriptBlock::Paragraph { text: "Hello from the other side".into() },
            ],
            tui: None,
        };

        let output = runtime.execute(&wasm, &input).unwrap();
        assert_eq!(output.blocks.len(), 2, "identity should preserve block count");
    }

    #[test]
    fn blocks_convert_back_and_forth() {
        use jaringan_core::{Block, Link, Image, Table};

        // Note: Block::Rule is intentionally excluded — it maps to a synthetic
        // ScriptBlock::List rendering and cannot roundtrip faithfully.
        let original = vec![
            Block::Heading { level: 1, text: "Test".into() },
            Block::Paragraph("Content".into()),
            Block::Link(Link { target: "jrg://test".into(), label: "Test Link".into() }),
            Block::Image(Image { source: "img.png".into(), alt: "An image".into() }),
            Block::Quote("Cited text".into()),
            Block::List(vec!["one".into(), "two".into()]),
            Block::Table(Table {
                headers: vec!["A".into()],
                rows: vec![vec!["1".into()]],
            }),
            Block::Preformatted { code: "code".into(), language: None },
        ];

        let script_blocks = blocks_to_script_blocks(&original);
        let back = script_blocks_to_blocks(&script_blocks);

        assert_eq!(original.len(), back.len());
        for (a, b) in original.iter().zip(back.iter()) {
            // Check variant equality via debug formatting
            assert_eq!(format!("{a:?}"), format!("{b:?}"));
        }
    }

    #[test]
    fn bridge_host_function_invoke() {
        let wasm = wat::parse_str(BRIDGE_TEST_WAT).unwrap();
        let runtime = WasmRuntime::new().unwrap();

        let fetch_called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let fetch_called_clone = fetch_called.clone();
        let log_called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let log_called_clone = log_called.clone();

        let bridge = bridge::BridgeState {
            fetch_fn: Some(std::sync::Arc::new(move |url: &str| {
                fetch_called_clone.store(true, std::sync::atomic::Ordering::SeqCst);
                assert_eq!(url, "jrg://example/test.jrg");
                Ok(r#"{"status":200}"#.into())
            })),
            navigate_fn: Some(std::sync::Arc::new(|_url: &str| Ok(r#"{}"#.into()))),
            log_fn: Some(std::sync::Arc::new(move |level: &str, msg: &str| {
                log_called_clone.store(true, std::sync::atomic::Ordering::SeqCst);
                assert_eq!(level, "info");
                assert_eq!(msg, "Hello from WASM");
            })),
        };

        let input = ScriptInput {
            title: Some("Bridge Test".into()),
            inputs: vec![],
            metadata: None,
            blocks: vec![],
            tui: None,
        };

        let output = runtime.execute_with_bridge(&wasm, &input, bridge).unwrap();
        assert_eq!(output.blocks.len(), 0);
        assert!(fetch_called.load(std::sync::atomic::Ordering::SeqCst), "fetch should have been called");
        assert!(log_called.load(std::sync::atomic::Ordering::SeqCst), "log should have been called");
    }
}
