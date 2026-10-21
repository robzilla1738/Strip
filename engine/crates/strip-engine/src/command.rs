//! Apply a command on the main thread and publish the events it produces.

use serde_json::Value;

use crate::bus::Bus;
use crate::session::{CommandError, Runtime};

pub fn run(runtime: &mut Runtime, bus: &Bus, action: &str, args: &Value) -> Result<(), CommandError> {
    let focus = runtime.perform(action, args)?;
    bus.emit("tour", serde_json::json!({ "action": action }));
    if let Some(focus) = focus {
        bus.emit(
            "focus",
            serde_json::json!({
                "window_id": focus.window_id,
                "display": focus.display,
                "column": focus.column,
            }),
        );
    }
    Ok(())
}
