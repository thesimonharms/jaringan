use notify::{Event, EventKind, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::mpsc;

/// A channel-based file watcher for the plugins directory.
pub struct PluginWatcher {
    _watcher: notify::RecommendedWatcher,
    rx: mpsc::Receiver<notify::Event>,
}

impl PluginWatcher {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let (tx, rx) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |res| {
            if let Ok(event) = res {
                let _ = tx.send(event);
            }
        })?;
        watcher.watch(path.as_ref(), RecursiveMode::NonRecursive)?;
        Ok(Self { _watcher: watcher, rx })
    }

    /// Try to receive a file-change event (non-blocking).
    pub fn poll(&self) -> Option<Event> {
        self.rx.try_recv().ok()
    }

    /// Returns true if a .wasm file was changed/created/deleted.
    pub fn has_plugin_change(&self) -> bool {
        self.poll().map_or(false, |event| {
            matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_))
                && event.paths.iter().any(|p| p.extension().map_or(false, |e| e == "wasm"))
        })
    }
}
