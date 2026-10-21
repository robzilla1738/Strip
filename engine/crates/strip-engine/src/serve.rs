//! Line-delimited JSON on a Unix socket. Queries read the latest snapshot.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;

use crate::private_api::PrivateApi;

pub fn listen(path: &Path, state: Arc<Mutex<String>>, private_api: PrivateApi, ax_trusted: bool) -> std::io::Result<()> {
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    thread::spawn(move || {
        for incoming in listener.incoming() {
            let Ok(stream) = incoming else {
                continue;
            };
            let state = Arc::clone(&state);
            let private_api = private_api.clone();
            thread::spawn(move || {
                let _ = handle(stream, &state, &private_api, ax_trusted);
            });
        }
    });
    Ok(())
}

fn handle(stream: UnixStream, state: &Mutex<String>, private_api: &PrivateApi, ax_trusted: bool) -> std::io::Result<()> {
    let mut write = stream.try_clone()?;
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let line = line?;
        let request: serde_json::Value = serde_json::from_str(&line).unwrap_or(serde_json::json!({}));
        let id = request.get("id").and_then(|value| value.as_u64()).unwrap_or(0);
        let kind = request.get("type").and_then(|value| value.as_str()).unwrap_or("");
        let response = match kind {
            "hello" => serde_json::json!({
                "id": id,
                "type": "hello",
                "protocol": 1,
                "engine_version": strip_core::ENGINE_VERSION,
                "ax_trusted": ax_trusted,
                "private_api": {
                    "skylight": private_api.skylight,
                    "ax_get_window": private_api.ax_get_window,
                },
            }),
            "query" => {
                let what = request.get("what").and_then(|value| value.as_str()).unwrap_or("");
                if what == "state" {
                    let data: serde_json::Value = serde_json::from_str(&state.lock().unwrap_or_else(|err| err.into_inner()))
                        .unwrap_or(serde_json::json!({}));
                    serde_json::json!({"id": id, "type": "result", "ok": true, "data": data})
                } else {
                    serde_json::json!({"id": id, "type": "result", "ok": false, "error": {"code": "bad_request", "message": "unknown query"}})
                }
            }
            "shutdown" => {
                crate::lifecycle::request_stop();
                serde_json::json!({"id": id, "type": "result", "ok": true})
            }
            _ => serde_json::json!({"id": id, "type": "result", "ok": false, "error": {"code": "bad_request", "message": "unsupported"}}),
        };
        writeln!(write, "{response}")?;
        if kind == "shutdown" {
            break;
        }
    }
    Ok(())
}
