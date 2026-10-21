//! Hot reload for the config file. The watcher is held by the engine so it
//! stays armed for the process lifetime.

use std::path::Path;
use std::sync::mpsc;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

pub fn watch_config(path: &Path, on_change: mpsc::Sender<()>) -> Result<RecommendedWatcher, String> {
    let mut watcher = notify::recommended_watcher(move |result: Result<notify::Event, notify::Error>| {
        let Ok(event) = result else {
            return;
        };
        if matches!(
            event.kind,
            EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
        ) {
            let _ = on_change.send(());
        }
    })
    .map_err(|err| err.to_string())?;
    watcher
        .watch(path, RecursiveMode::NonRecursive)
        .map_err(|err| err.to_string())?;
    Ok(watcher)
}
