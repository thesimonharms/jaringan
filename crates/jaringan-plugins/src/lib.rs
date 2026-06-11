pub mod hot_reload;
pub mod plugin;

use std::path::{Path, PathBuf};
use plugin::{Plugin, PluginHook, PluginRegistration};
use wasmtime::{Engine, Linker, Memory, Module, Store};
use jaringan_script::{ScriptInput, ScriptOutput, WasmRuntime};

/// Registry of loaded TUI plugins.
pub struct PluginRegistry {
    plugins: Vec<Plugin>,
    engine: Engine,
    watch_path: PathBuf,
}

impl PluginRegistry {
    pub fn new<P: Into<PathBuf>>(plugins_dir: P) -> Result<Self, Box<dyn std::error::Error>> {
        let engine = Engine::new(&wasmtime::Config::new())?;
        Ok(Self {
            plugins: Vec::new(),
            engine,
            watch_path: plugins_dir.into(),
        })
    }

    /// Create an empty registry (no plugins directory).
    pub fn empty() -> Self {
        Self {
            plugins: Vec::new(),
            engine: Engine::new(&wasmtime::Config::new()).expect("WASM engine"),
            watch_path: PathBuf::from("/tmp/jaringan-plugins"),
        }
    }

    /// Load all `.wasm` files from the plugins directory.
    pub fn load_all(&mut self) -> Result<(), String> {
        let dir = &self.watch_path;
        if !dir.exists() {
            let _ = std::fs::create_dir_all(dir);
            return Ok(());
        }
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .map_err(|e| format!("read plugins dir: {e}"))?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map_or(false, |ext| ext == "wasm"))
            .collect();
        entries.sort_by_key(|e| e.path());

        self.plugins.clear();
        for entry in entries {
            let path = entry.path();
            match self.load_plugin(&path) {
                Ok(registration) => {
                    let wasm_binary = std::fs::read(&path).unwrap_or_default();
                    self.plugins.push(Plugin {
                        registration,
                        wasm_binary,
                        path,
                    });
                }
                Err(e) => {
                    eprintln!("[jaringan] warning: failed to load plugin {}: {e}", path.display());
                }
            }
        }
        Ok(())
    }

    /// Load a single plugin and call its `register` export.
    fn load_plugin(&self, path: &Path) -> Result<PluginRegistration, String> {
        let wasm_binary = std::fs::read(path).map_err(|e| format!("read {path:?}: {e}"))?;
        let module = Module::new(&self.engine, &wasm_binary)
            .map_err(|e| format!("module {path:?}: {e}"))?;
        let mut store = Store::new(&self.engine, ());
        let linker = Linker::new(&self.engine);

        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| format!("instantiate: {e}"))?;

        // Extract memory
        let memory: Memory = instance
            .get_export(&mut store, "memory")
            .ok_or_else(|| "missing 'memory' export".to_string())?
            .into_memory()
            .ok_or_else(|| "memory export is not Memory".to_string())?;

        // Call register() -> returns pointer to JSON
        let register = instance
            .get_export(&mut store, "register")
            .ok_or_else(|| "missing 'register' export".to_string())?
            .into_func()
            .ok_or_else(|| "register export is not Func".to_string())?;
        let register_typed = register
            .typed::<(), i32>(&store)
            .map_err(|e| format!("register type: {e}"))?;
        let ptr = register_typed
            .call(&mut store, ())
            .map_err(|e| format!("register call: {e}"))? as usize;

        // Read 4-byte length + JSON
        let mem_data = memory.data(&store);
        if ptr + 4 > mem_data.len() {
            return Err("register output ptr out of bounds".into());
        }
        let len_bytes: [u8; 4] = mem_data[ptr..ptr + 4].try_into().unwrap();
        let len = u32::from_le_bytes(len_bytes) as usize;
        let json_start = ptr + 4;
        let json_end = json_start + len;
        if json_end > mem_data.len() {
            return Err("register output JSON exceeds memory".into());
        }

        let json_str = std::str::from_utf8(&mem_data[json_start..json_end])
            .map_err(|e| format!("register output not UTF-8: {e}"))?;
        serde_json::from_str(json_str)
            .map_err(|e| format!("register JSON parse: {e}"))
    }

    /// Trigger a hook on all plugins that registered for it.
    pub fn trigger_hook(&self, hook: &PluginHook, input: &ScriptInput) -> Vec<(String, ScriptOutput)> {
        let mut results = Vec::new();
        for plugin in &self.plugins {
            if !plugin.registration.hooks.iter().any(|h| h == hook) {
                continue;
            }
            // Use WasmRuntime to execute the plugin
            match WasmRuntime::new() {
                Ok(runtime) => match runtime.execute(&plugin.wasm_binary, input) {
                    Ok(output) => results.push((plugin.registration.name.clone(), output)),
                    Err(e) => eprintln!("[jaringan] plugin {} hook error: {e}", plugin.registration.name),
                },
                Err(e) => eprintln!("[jaringan] plugin {} runtime error: {e}", plugin.registration.name),
            }
        }
        results
    }

    pub fn plugins(&self) -> &[Plugin] {
        &self.plugins
    }
}
