//! Session restore matching and the pure checks around it.
//!
//! The engine calls these on launch. Identity match is `(CGWindowID, pid)`,
//! then a unique `(bundle, title, role, subrole)`. Two candidates that share
//! a title stay unmatched.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde_json::Value;
use strip_core::Frame;

#[derive(Clone, Debug, PartialEq)]
pub struct SavedWindow {
    pub id: u64,
    pub pid: i32,
    pub bundle_id: String,
    pub title: String,
    pub role: String,
    pub subrole: String,
    pub display: String,
    pub column: usize,
    pub stack: usize,
    pub width_ratio: f64,
    pub height_ratio: f64,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct SavedSession {
    pub windows: Vec<SavedWindow>,
    pub scroll: Vec<(String, f64)>,
    pub focus: Vec<(String, i64)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LiveWindow {
    pub id: u64,
    pub pid: i32,
    pub bundle_id: String,
    pub title: String,
    pub role: String,
    pub subrole: String,
}

pub fn load_session(path: &Path) -> SavedSession {
    let Ok(text) = fs::read_to_string(path) else {
        return SavedSession::default();
    };
    decode_session(&text)
}

pub fn decode_session(text: &str) -> SavedSession {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return SavedSession::default();
    };
    let Some(displays) = value.get("displays").and_then(Value::as_array) else {
        return SavedSession::default();
    };
    let mut session = SavedSession::default();
    for display in displays {
        let Some(uuid) = display.get("uuid").and_then(Value::as_str) else {
            continue;
        };
        let Some(strips) = display.get("strips").and_then(Value::as_array) else {
            continue;
        };
        for strip in strips {
            let scroll = strip.get("scroll_x").and_then(Value::as_f64).unwrap_or(0.0);
            let focus = strip.get("focused_column").and_then(Value::as_i64).unwrap_or(0);
            session.scroll.push((uuid.to_string(), scroll));
            session.focus.push((uuid.to_string(), focus));
            let Some(columns) = strip.get("columns").and_then(Value::as_array) else {
                continue;
            };
            for (column_index, column) in columns.iter().enumerate() {
                let width = column.get("width_ratio").and_then(Value::as_f64).unwrap_or(0.5);
                let Some(windows) = column.get("windows").and_then(Value::as_array) else {
                    continue;
                };
                for (stack, window) in windows.iter().enumerate() {
                    let Some(id) = window.get("id").and_then(Value::as_u64) else {
                        continue;
                    };
                    session.windows.push(SavedWindow {
                        id,
                        pid: window.get("pid").and_then(Value::as_i64).unwrap_or(0) as i32,
                        bundle_id: json_string(window, "bundle_id"),
                        title: json_string(window, "title"),
                        role: json_string(window, "role"),
                        subrole: json_string(window, "subrole"),
                        display: uuid.to_string(),
                        column: column_index,
                        stack,
                        width_ratio: width,
                        height_ratio: window.get("height_ratio").and_then(Value::as_f64).unwrap_or(1.0),
                    });
                }
            }
        }
    }
    session
}

/// Pairs `(saved index, live index)`.
pub fn match_windows(saved: &[SavedWindow], live: &[LiveWindow]) -> Vec<(usize, usize)> {
    let mut pairs = Vec::new();
    let mut saved_used = vec![false; saved.len()];
    let mut live_used = vec![false; live.len()];
    for (saved_index, saved_window) in saved.iter().enumerate() {
        let Some(live_index) = live
            .iter()
            .position(|window| window.id == saved_window.id && window.pid == saved_window.pid)
        else {
            continue;
        };
        pairs.push((saved_index, live_index));
        saved_used[saved_index] = true;
        live_used[live_index] = true;
    }
    let mut saved_groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, window) in saved.iter().enumerate() {
        if saved_used[index] {
            continue;
        }
        saved_groups.entry(identity_key(window)).or_default().push(index);
    }
    let mut live_groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, window) in live.iter().enumerate() {
        if live_used[index] {
            continue;
        }
        live_groups
            .entry(format!("{}\u{1f}{}\u{1f}{}\u{1f}{}", window.bundle_id, window.title, window.role, window.subrole))
            .or_default()
            .push(index);
    }
    for (group, saved_indexes) in saved_groups {
        let Some(live_indexes) = live_groups.get(&group) else {
            continue;
        };
        if saved_indexes.len() == 1 && live_indexes.len() == 1 {
            pairs.push((saved_indexes[0], live_indexes[0]));
        }
    }
    pairs.sort_unstable();
    pairs
}

fn identity_key(window: &SavedWindow) -> String {
    format!("{}\u{1f}{}\u{1f}{}\u{1f}{}", window.bundle_id, window.title, window.role, window.subrole)
}

fn json_string(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

/// Left half of a column inserts before it. Right half inserts after it.
pub fn drop_index(columns: &[(f64, f64)], x: f64) -> Option<usize> {
    for (index, (origin, width)) in columns.iter().enumerate() {
        if x >= *origin && x < origin + width {
            if x < origin + width / 2.0 {
                return Some(index);
            }
            return Some(index + 1);
        }
    }
    None
}

pub fn shares_native_tab(a: Frame, b: Frame) -> bool {
    (a.x - b.x).abs() < 2.0 && (a.y - b.y).abs() < 2.0 && (a.width - b.width).abs() < 2.0 && (a.height - b.height).abs() < 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(id: u64, pid: i32, title: &str) -> SavedWindow {
        SavedWindow {
            id,
            pid,
            bundle_id: "app".to_string(),
            title: title.to_string(),
            role: "AXWindow".to_string(),
            subrole: "AXStandardWindow".to_string(),
            display: "d".to_string(),
            column: 0,
            stack: 0,
            width_ratio: 0.5,
            height_ratio: 1.0,
        }
    }

    fn live(id: u64, pid: i32, title: &str) -> LiveWindow {
        LiveWindow {
            id,
            pid,
            bundle_id: "app".to_string(),
            title: title.to_string(),
            role: "AXWindow".to_string(),
            subrole: "AXStandardWindow".to_string(),
        }
    }

    #[test]
    fn id_and_pid_match_before_the_title() {
        let saved = [saved(5, 9, "Old")];
        let live = [live(5, 9, "New")];
        assert_eq!(match_windows(&saved, &live), vec![(0, 0)]);
    }

    #[test]
    fn unique_bundle_title_role_subrole_matches_when_ids_differ() {
        let saved = [saved(1, 2, "Doc")];
        let live = [live(9, 3, "Doc")];
        assert_eq!(match_windows(&saved, &live), vec![(0, 0)]);
    }

    #[test]
    fn ambiguous_title_stays_unmatched() {
        let saved = [saved(1, 2, "Doc"), saved(3, 4, "Doc")];
        let live = [live(8, 5, "Doc"), live(9, 6, "Doc")];
        assert!(match_windows(&saved, &live).is_empty());
    }

    #[test]
    fn one_saved_title_against_two_live_copies_stays_unmatched() {
        let saved = [saved(1, 2, "Doc")];
        let live = [live(8, 5, "Doc"), live(9, 6, "Doc")];
        assert!(match_windows(&saved, &live).is_empty());
    }

    #[test]
    fn decode_session_keeps_column_order_and_widths() {
        let text = r#"{
            "displays": [{
                "uuid": "disp",
                "strips": [{
                    "scroll_x": 12.0,
                    "focused_column": 1,
                    "columns": [
                        {"width_ratio": 0.5, "windows": [{"id": 10, "pid": 1, "bundle_id": "app", "title": "A", "role": "AXWindow", "subrole": "AXStandardWindow"}]},
                        {"width_ratio": 0.66, "windows": [{"id": 11, "pid": 1, "bundle_id": "app", "title": "B", "role": "AXWindow", "subrole": "AXStandardWindow"}]}
                    ]
                }]
            }]
        }"#;
        let session = decode_session(text);
        assert_eq!(session.windows.len(), 2);
        assert_eq!(session.windows[0].column, 0);
        assert_eq!(session.windows[1].column, 1);
        assert!((session.windows[1].width_ratio - 0.66).abs() < 1e-9);
        assert_eq!(session.scroll, vec![("disp".to_string(), 12.0)]);
    }

    #[test]
    fn drop_index_uses_the_column_halves() {
        let columns = [(0.0, 100.0), (108.0, 100.0)];
        assert_eq!(drop_index(&columns, 20.0), Some(0));
        assert_eq!(drop_index(&columns, 80.0), Some(1));
        assert_eq!(drop_index(&columns, 150.0), Some(1));
        assert_eq!(drop_index(&columns, 180.0), Some(2));
    }

    #[test]
    fn native_tabs_share_a_frame() {
        let frame = Frame { x: 10.0, y: 20.0, width: 400.0, height: 300.0 };
        let shifted = Frame { x: 40.0, y: 20.0, width: 400.0, height: 300.0 };
        assert!(shares_native_tab(frame, frame));
        assert!(!shares_native_tab(frame, shifted));
    }
}
