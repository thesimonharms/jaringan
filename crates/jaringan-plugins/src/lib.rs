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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn unique_temp_dir() -> PathBuf {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("jrg-test-{ts}"))
    }

    /// A minimal WAT plugin that registers with name "test" and OnPageLoad hook,
    /// and implements process() returning an empty ScriptOutput.
    const PLUGIN_WAT: &str = r#"
(module
  (memory (export "memory") 1)
  (func (export "register") (result i32)
    i32.const 0
  )
  (func (export "process") (param i32 i32) (result i32)
    i32.const 200
  )
  ;; Register output at offset 0: len(80) + JSON
  (data (i32.const 0) "\50\00\00\00")
  (data (i32.const 4) "{\"name\":\"test\",\"version\":\"1.0\",\"hooks\":[{\"hook\":\"OnPageLoad\"}],\"keybindings\":[]}")
  ;; Process output at offset 200: len(13) + {"blocks":[]}
  (data (i32.const 200) "\0d\00\00\00")
  (data (i32.const 204) "{\"blocks\":[]}")
)
"#;

    #[test]
    fn loads_single_plugin_from_directory() {
        let dir = unique_temp_dir();
        let _ = fs::create_dir_all(&dir);

        let wasm = wat::parse_str(PLUGIN_WAT).unwrap();
        fs::write(dir.join("test-plugin.wasm"), &wasm).unwrap();

        let mut registry = PluginRegistry::new(&dir).unwrap();
        registry.load_all().unwrap();
        assert_eq!(registry.plugins().len(), 1);
        assert_eq!(registry.plugins()[0].registration.name, "test");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_registry_has_no_plugins() {
        let dir = unique_temp_dir();
        let _ = fs::create_dir_all(&dir);

        let mut registry = PluginRegistry::new(&dir).unwrap();
        registry.load_all().unwrap();
        assert_eq!(registry.plugins().len(), 0);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_registry_creator() {
        let registry = PluginRegistry::empty();
        assert_eq!(registry.plugins().len(), 0);
    }

    #[test]
    fn ignores_non_wasm_files() {
        let dir = unique_temp_dir();
        let _ = fs::create_dir_all(&dir);

        // Create a non-wasm file
        fs::write(dir.join("not-a-plugin.txt"), b"hello").unwrap();

        let mut registry = PluginRegistry::new(&dir).unwrap();
        registry.load_all().unwrap();
        assert_eq!(registry.plugins().len(), 0);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn trigger_hook_on_plugin() {
        let dir = unique_temp_dir();
        let _ = fs::create_dir_all(&dir);

        let wasm = wat::parse_str(PLUGIN_WAT).unwrap();
        fs::write(dir.join("test-plugin.wasm"), &wasm).unwrap();

        let mut registry = PluginRegistry::new(&dir).unwrap();
        registry.load_all().unwrap();

        let input = ScriptInput {
            title: Some("Test".into()),
            inputs: vec![],
            metadata: None,
            blocks: vec![],
            tui: None,
        };

        let results = registry.trigger_hook(&PluginHook::OnPageLoad, &input);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "test");

        let _ = fs::remove_dir_all(&dir);
    }
}
