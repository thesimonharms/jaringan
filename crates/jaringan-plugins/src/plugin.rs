use serde::{Deserialize, Serialize};

/// A hook that a TUI plugin can register for.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "hook")]
pub enum PluginHook {
    #[serde(rename = "OnPageLoad")]
    OnPageLoad,
    #[serde(rename = "OnKey")]
    OnKey,
    #[serde(rename = "OnRender")]
    OnRender,
}

/// Registration info returned by a plugin's `register` export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginRegistration {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub hooks: Vec<PluginHook>,
    #[serde(default)]
    pub keybindings: Vec<PluginKeybinding>,
}

/// A keybinding that a plugin wants to capture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginKeybinding {
    pub key: String,
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub alt: bool,
    pub action: String,
}

/// A loaded and running TUI plugin.
pub struct Plugin {
    pub registration: PluginRegistration,
    pub wasm_binary: Vec<u8>,
    pub path: std::path::PathBuf,
}
