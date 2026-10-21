//! Apply a command on the main thread and publish the events it produces.

use serde_json::Value;

use crate::bus::Bus;
use crate::session::{CommandError, Runtime};

pub fn shows_strip_indicator(action: &str) -> bool {
    !matches!(action, "stop" | "start" | "reload_config")
}

/// Sends the published state to subscribers when it changed since the last send.
pub fn flush_state(runtime: &Runtime, bus: &Bus) {
    if let Some(text) = runtime.take_state_change() {
        bus.emit_raw("state", &text);
    }
}

pub fn run(runtime: &mut Runtime, bus: &Bus, action: &str, args: &Value) -> Result<(), CommandError> {
    let focus = runtime.perform(action, args)?;
    flush_state(runtime, bus);
    if shows_strip_indicator(action) {
        let display = focus.as_ref().map(|item| item.display.clone()).or_else(|| runtime.primary_display());
        if let Some(display) = display {
            bus.emit(
                "indicator",
                serde_json::json!({
                    "display": display,
                    "show": true,
                    "hold": false,
                }),
            );
        }
    }
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

#[cfg(test)]
mod tests {
    use super::shows_strip_indicator;

    #[test]
    fn navigation_announces_the_indicator() {
        assert!(shows_strip_indicator("focus_left"));
        assert!(shows_strip_indicator("cycle_width"));
        assert!(shows_strip_indicator("pin_left"));
        assert!(!shows_strip_indicator("stop"));
        assert!(!shows_strip_indicator("start"));
        assert!(!shows_strip_indicator("reload_config"));
    }
}
