use serde::{Deserialize, Serialize};
use wasmtime::{Engine, Linker, Memory, Module, Store};

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
        let mut store = Store::new(&self.engine, ());
        let module = Module::new(&self.engine, wasm_binary)?;
        let linker = Linker::new(&self.engine);

        // Instantiate the module (no imports needed for basic scripts).
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

        let process_typed = process_func
            .typed::<(i32, i32), i32>(&store)?;

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

        let output_json =
            std::str::from_utf8(&mem_data[json_start..json_end])
                .map_err(|e| WasmError::Memory(format!("output is not valid UTF-8: {}", e)))?;

        let output: ScriptOutput = serde_json::from_str(output_json)?;
        Ok(output)
    }
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
}
