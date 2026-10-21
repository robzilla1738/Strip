//! Line-delimited JSON on a Unix socket. Commands are applied on the main thread.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;

use strip_ipc::{decode_client, encode, ClientMessage, ServerMessage, MAX_LINE_BYTES};

use crate::bus::Bus;
use crate::private_api::PrivateApi;

pub struct Serve {
    pub state: Arc<Mutex<String>>,
    pub metrics: Arc<Mutex<String>>,
    pub private_api: PrivateApi,
    pub ax_trusted: bool,
    pub bus: Arc<Bus>,
}

pub fn listen(path: &Path, serve: Serve) -> std::io::Result<()> {
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
            let serve = Serve {
                state: Arc::clone(&serve.state),
                metrics: Arc::clone(&serve.metrics),
                private_api: serve.private_api.clone(),
                ax_trusted: serve.ax_trusted,
                bus: Arc::clone(&serve.bus),
            };
            thread::spawn(move || {
                let _ = handle(stream, &serve);
            });
        }
    });
    Ok(())
}

fn handle(stream: UnixStream, serve: &Serve) -> std::io::Result<()> {
    let mut write = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            break;
        }
        if line.len() > MAX_LINE_BYTES {
            write_message(&mut write, &ServerMessage::error(0, "bad_request", "line exceeds 1 MiB"))?;
            break;
        }
        let message = match decode_client(line.trim()) {
            Ok(message) => message,
            Err(_) => {
                write_message(&mut write, &ServerMessage::error(0, "bad_request", "malformed json"))?;
                continue;
            }
        };
        match message {
            ClientMessage::Hello { id, .. } => {
                write_message(
                    &mut write,
                    &ServerMessage::hello(
                        id,
                        strip_core::ENGINE_VERSION,
                        serve.ax_trusted,
                        serve.private_api.skylight,
                        serve.private_api.ax_get_window,
                    ),
                )?;
            }
            ClientMessage::Query { id, what } => write_message(&mut write, &query(serve, id, &what))?,
            ClientMessage::Subscribe { id, events } => {
                write_message(&mut write, &ServerMessage::ok(id))?;
                let (sub_id, events_rx) = serve.bus.subscribe(events);
                let mut event_write = write.try_clone()?;
                thread::spawn(move || {
                    while let Ok(event) = events_rx.recv() {
                        if writeln!(event_write, "{event}").is_err() {
                            break;
                        }
                    }
                });
                // Stay on this connection until the client closes it.
                let mut rest = String::new();
                loop {
                    rest.clear();
                    if reader.read_line(&mut rest)? == 0 {
                        break;
                    }
                }
                serve.bus.unsubscribe(sub_id);
                break;
            }
            other => {
                let id = message_id(&other);
                let rx = serve.bus.post(other);
                match rx.recv() {
                    Ok(response) => writeln!(write, "{response}")?,
                    Err(_) => write_message(&mut write, &ServerMessage::error(id, "internal", "engine stopped"))?,
                }
            }
        }
    }
    Ok(())
}

fn query(serve: &Serve, id: u64, what: &str) -> ServerMessage {
    let state = serve.state.lock().unwrap_or_else(|err| err.into_inner()).clone();
    let parsed: serde_json::Value = serde_json::from_str(&state).unwrap_or(serde_json::json!({}));
    match what {
        "state" => ServerMessage::data(id, parsed),
        "displays" => ServerMessage::data(id, parsed.get("displays").cloned().unwrap_or(serde_json::json!([]))),
        "metrics" => {
            let body = serve.metrics.lock().unwrap_or_else(|err| err.into_inner()).clone();
            let data = serde_json::from_str(&body).unwrap_or(serde_json::json!({}));
            ServerMessage::data(id, data)
        }
        "config_errors" => ServerMessage::data(id, serde_json::json!([])),
        _ => ServerMessage::error(id, "bad_request", "unknown query"),
    }
}

fn message_id(message: &ClientMessage) -> u64 {
    match message {
        ClientMessage::Hello { id, .. }
        | ClientMessage::Command { id, .. }
        | ClientMessage::Query { id, .. }
        | ClientMessage::Subscribe { id, .. }
        | ClientMessage::ReloadConfig { id }
        | ClientMessage::SetManaging { id, .. }
        | ClientMessage::Shutdown { id } => *id,
    }
}

fn write_message(write: &mut UnixStream, message: &ServerMessage) -> std::io::Result<()> {
    write.write_all(encode(message).as_bytes())
}
