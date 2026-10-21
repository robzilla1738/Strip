//! Newline-delimited JSON, protocol version 1. Shared by the engine and the CLI.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_LINE_BYTES: usize = 1024 * 1024;

pub const ACTIONS: &[&str] = &[
    "focus_left",
    "focus_right",
    "focus_up",
    "focus_down",
    "focus_first",
    "focus_last",
    "focus_index",
    "move_to_index",
    "move_left",
    "move_right",
    "move_up",
    "move_down",
    "move_first",
    "move_last",
    "cycle_width",
    "full_width",
    "center",
    "wider",
    "narrower",
    "taller",
    "shorter",
    "equalize_heights",
    "stack_left",
    "stack_right",
    "unstack_right",
    "unstack_left",
    "toggle_tabbed",
    "toggle_float",
    "send_next_display",
    "pin_left",
    "pin_right",
    "stop",
    "focus_window",
    "move_column",
    "start",
    "reload_config",
];

pub fn is_known_action(action: &str) -> bool {
    ACTIONS.contains(&action)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    #[serde(rename = "hello")]
    Hello { id: u64, client: String, protocol: u32 },
    #[serde(rename = "command")]
    Command {
        id: u64,
        action: String,
        #[serde(default)]
        args: Value,
    },
    #[serde(rename = "query")]
    Query { id: u64, what: String },
    #[serde(rename = "subscribe")]
    Subscribe { id: u64, events: Vec<String> },
    #[serde(rename = "reload_config")]
    ReloadConfig { id: u64 },
    #[serde(rename = "set_managing")]
    SetManaging { id: u64, enabled: bool },
    #[serde(rename = "shutdown")]
    Shutdown { id: u64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrivateApiFlags {
    pub skylight: bool,
    pub ax_get_window: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    #[serde(rename = "hello")]
    Hello {
        id: u64,
        protocol: u32,
        engine_version: String,
        ax_trusted: bool,
        private_api: PrivateApiFlags,
    },
    #[serde(rename = "result")]
    Result {
        id: u64,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<ErrorBody>,
    },
    #[serde(rename = "event")]
    Event { event: String, data: Value },
}

impl ServerMessage {
    pub fn hello(id: u64, engine_version: &str, ax_trusted: bool, skylight: bool, ax_get_window: bool) -> Self {
        Self::Hello {
            id,
            protocol: PROTOCOL_VERSION,
            engine_version: engine_version.to_string(),
            ax_trusted,
            private_api: PrivateApiFlags { skylight, ax_get_window },
        }
    }

    pub fn ok(id: u64) -> Self {
        Self::Result { id, ok: true, data: None, error: None }
    }

    pub fn data(id: u64, data: Value) -> Self {
        Self::Result { id, ok: true, data: Some(data), error: None }
    }

    pub fn error(id: u64, code: &str, message: &str) -> Self {
        Self::Result {
            id,
            ok: false,
            data: None,
            error: Some(ErrorBody { code: code.to_string(), message: message.to_string() }),
        }
    }
}

pub fn encode(message: &impl Serialize) -> String {
    let mut line = serde_json::to_string(message).unwrap_or_else(|_| "{}".to_string());
    line.push('\n');
    line
}

pub fn decode_client(line: &str) -> Result<ClientMessage, String> {
    serde_json::from_str(line).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn fixtures_round_trip() {
        let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        dir.push("fixtures");
        let mut files = fs::read_dir(&dir).expect("fixtures").map(|entry| entry.unwrap().path()).collect::<Vec<_>>();
        files.sort();
        assert!(files.len() >= 16, "expected one fixture per protocol message");
        for path in files {
            let text = fs::read_to_string(&path).unwrap();
            let original: Value = serde_json::from_str(&text).unwrap();
            let encoded = round_trip(&original).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
            assert_eq!(encoded, original, "{}", path.display());
        }
    }

    fn round_trip(original: &Value) -> Result<Value, String> {
        let kind = original.get("type").and_then(Value::as_str).unwrap_or("");
        let client_hello = kind == "hello" && original.get("client").is_some();
        let server_hello = kind == "hello" && original.get("engine_version").is_some();
        if client_hello || matches!(kind, "command" | "query" | "subscribe" | "reload_config" | "set_managing" | "shutdown") {
            let message: ClientMessage = serde_json::from_value(original.clone()).map_err(|err| err.to_string())?;
            return serde_json::to_value(&message).map_err(|err| err.to_string());
        }
        if server_hello || matches!(kind, "result" | "event") {
            let message: ServerMessage = serde_json::from_value(original.clone()).map_err(|err| err.to_string())?;
            return serde_json::to_value(&message).map_err(|err| err.to_string());
        }
        Err(format!("unclassified message {kind}"))
    }

    #[test]
    fn unknown_names_are_not_actions() {
        assert!(is_known_action("focus_index"));
        assert!(!is_known_action("not_a_command"));
    }
}
