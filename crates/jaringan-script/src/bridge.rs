use std::collections::HashMap;
use std::sync::Arc;
use wasmtime::{AsContext, AsContextMut, Memory};

/// Holds optional closures that WASM scripts can call via imported host functions.
#[derive(Clone)]
pub struct BridgeState {
    pub fetch_fn: Option<Arc<dyn Fn(&str) -> Result<String, String> + Send + Sync>>,
    pub navigate_fn: Option<Arc<dyn Fn(&str) -> Result<String, String> + Send + Sync>>,
    pub log_fn: Option<Arc<dyn Fn(&str, &str) + Send + Sync>>,
    pub store: Option<HashMap<String, String>>,
    pub resolve_fn: Option<Arc<dyn Fn(&str) -> Result<String, String> + Send + Sync>>,
    pub page_inputs: Option<HashMap<String, String>>,
}

impl BridgeState {
    /// Create a BridgeState with no host functions (all closures are None).
    pub fn empty() -> Self {
        Self {
            fetch_fn: None,
            navigate_fn: None,
            log_fn: None,
            store: None,
            resolve_fn: None,
            page_inputs: None,
        }
    }
}

/// Read a NUL-free string from WASM linear memory at the given pointer and
/// length. Returns an empty string if the pointer/length are out of bounds
/// (bounds-checked to prevent host panics from malicious WASM modules).
///
/// `store` must implement `AsContext` (e.g. `&Store<T>` or `StoreContext<'_, T>`).
pub fn read_string(mem: &Memory, store: &impl AsContext, ptr: i32, len: i32) -> String {
    let data = mem.data(store);
    if ptr < 0 || len < 0 {
        return String::new();
    }
    let start = ptr as usize;
    let end = start.saturating_add(len as usize);
    if start >= data.len() || end > data.len() {
        return String::new();
    }
    String::from_utf8_lossy(&data[start..end]).into_owned()
}

/// Write a JSON string into WASM memory at offset 65536 with a 4-byte
/// little-endian length prefix.  Grows memory if needed.  Returns the offset
/// (65536).
///
/// `store` must implement `AsContextMut` (e.g. `&mut Store<T>` or
/// `&mut StoreContextMut<'_, T>`).
pub fn write_json<T: AsContextMut>(mem: &Memory, store: &mut T, json: &str) -> i32 {
    let bytes = json.as_bytes();
    let len = bytes.len();
    let needed = 4 + len;
    let offset: usize = 65536;

    // Grow memory if needed (wasm page = 64 KiB).
    let current_size = mem.size(&*store) as usize * 65_536;
    if offset + needed > current_size {
        let pages_needed = (offset + needed - current_size).div_ceil(65_536);
        if mem.grow(&mut *store, pages_needed as u64).is_err() {
            return 0; // out of memory — return null pointer
        }
    }

    let data = mem.data_mut(&mut *store);
    data[offset..offset + 4].copy_from_slice(&(len as u32).to_le_bytes());
    data[offset + 4..offset + 4 + len].copy_from_slice(bytes);

    offset as i32
}

/// Write an error JSON (`{"error":"<msg>"}`) into WASM memory.  Same layout as
/// `write_json`.  Returns the offset (65536).
pub fn write_error<T: AsContextMut>(mem: &Memory, store: &mut T, msg: &str) -> i32 {
    // Use serde_json for proper escaping (handles control chars, quotes, backslashes)
    let json = serde_json::json!({ "error": msg });
    let json_str = serde_json::to_string(&json).unwrap_or_else(|_| {
        r#"{"error":"internal serialization error"}"#.to_string()
    });
    write_json(mem, store, &json_str)
}
