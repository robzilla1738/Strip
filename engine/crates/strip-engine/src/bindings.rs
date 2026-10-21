//! Shortcut strings (`ctrl+alt-h`) and the versioned default-set merge.

use std::collections::{BTreeMap, HashSet};

pub const SHORTCUT_SET_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Binding {
    pub keycode: u16,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub cmd: bool,
}

/// US-ANSI virtual key codes. The recorder writes these names; parse time
/// resolves them with this table so a saved binding does not depend on the
/// active layout.
fn key_table() -> &'static [(&'static str, u16)] {
    &[
        ("a", 0x00),
        ("s", 0x01),
        ("d", 0x02),
        ("f", 0x03),
        ("h", 0x04),
        ("g", 0x05),
        ("z", 0x06),
        ("x", 0x07),
        ("c", 0x08),
        ("v", 0x09),
        ("b", 0x0b),
        ("q", 0x0c),
        ("w", 0x0d),
        ("e", 0x0e),
        ("r", 0x0f),
        ("y", 0x10),
        ("t", 0x11),
        ("1", 0x12),
        ("2", 0x13),
        ("3", 0x14),
        ("4", 0x15),
        ("6", 0x16),
        ("5", 0x17),
        ("equal", 0x18),
        ("9", 0x19),
        ("7", 0x1a),
        ("minus", 0x1b),
        ("8", 0x1c),
        ("0", 0x1d),
        ("rightbracket", 0x1e),
        ("o", 0x1f),
        ("u", 0x20),
        ("leftbracket", 0x21),
        ("i", 0x22),
        ("p", 0x23),
        ("l", 0x25),
        ("j", 0x26),
        ("k", 0x28),
        ("n", 0x2d),
        ("m", 0x2e),
        ("left", 0x7b),
        ("right", 0x7c),
        ("down", 0x7d),
        ("up", 0x7e),
    ]
}

pub fn keycode_for(name: &str) -> Option<u16> {
    key_table()
        .iter()
        .find(|(entry, _)| *entry == name)
        .map(|(_, code)| *code)
}

pub fn key_name(keycode: u16) -> Option<&'static str> {
    key_table()
        .iter()
        .find(|(_, code)| *code == keycode)
        .map(|(name, _)| *name)
}

pub fn parse_binding(input: &str) -> Result<Binding, String> {
    let Some((mods, key)) = input.rsplit_once('-') else {
        return Err(format!("binding `{input}` has no key"));
    };
    let Some(keycode) = keycode_for(key) else {
        return Err(format!("unknown key `{key}`"));
    };
    let mut binding = Binding {
        keycode,
        ctrl: false,
        alt: false,
        shift: false,
        cmd: false,
    };
    if !mods.is_empty() {
        for part in mods.split('+') {
            match part {
                "ctrl" => binding.ctrl = true,
                "alt" => binding.alt = true,
                "shift" => binding.shift = true,
                "cmd" => binding.cmd = true,
                other => return Err(format!("unknown modifier `{other}`")),
            }
        }
    }
    Ok(binding)
}

pub fn format_binding(binding: &Binding) -> String {
    let mut mods = Vec::new();
    if binding.ctrl {
        mods.push("ctrl");
    }
    if binding.alt {
        mods.push("alt");
    }
    if binding.shift {
        mods.push("shift");
    }
    if binding.cmd {
        mods.push("cmd");
    }
    let key = key_name(binding.keycode).unwrap_or("unknown");
    format!("{}-{key}", mods.join("+"))
}

pub fn default_bindings() -> BTreeMap<String, Vec<String>> {
    let pairs: &[(&str, &[&str])] = &[
        ("focus_left", &["ctrl+alt-h", "ctrl+alt-left"]),
        ("focus_right", &["ctrl+alt-l", "ctrl+alt-right"]),
        ("focus_up", &["ctrl+alt-k", "ctrl+alt-up"]),
        ("focus_down", &["ctrl+alt-j", "ctrl+alt-down"]),
        ("focus_first", &["ctrl+alt+cmd-left"]),
        ("focus_last", &["ctrl+alt+cmd-right"]),
        (
            "focus_index",
            &[
                "ctrl+alt-1",
                "ctrl+alt-2",
                "ctrl+alt-3",
                "ctrl+alt-4",
                "ctrl+alt-5",
                "ctrl+alt-6",
                "ctrl+alt-7",
                "ctrl+alt-8",
                "ctrl+alt-9",
            ],
        ),
        (
            "move_to_index",
            &[
                "ctrl+alt+shift-1",
                "ctrl+alt+shift-2",
                "ctrl+alt+shift-3",
                "ctrl+alt+shift-4",
                "ctrl+alt+shift-5",
                "ctrl+alt+shift-6",
                "ctrl+alt+shift-7",
                "ctrl+alt+shift-8",
                "ctrl+alt+shift-9",
            ],
        ),
        ("move_left", &["ctrl+alt+shift-h", "ctrl+alt+shift-left"]),
        ("move_right", &["ctrl+alt+shift-l", "ctrl+alt+shift-right"]),
        ("move_up", &["ctrl+alt+shift-k", "ctrl+alt+shift-up"]),
        ("move_down", &["ctrl+alt+shift-j", "ctrl+alt+shift-down"]),
        ("move_first", &["ctrl+alt+shift+cmd-left"]),
        ("move_last", &["ctrl+alt+shift+cmd-right"]),
        ("cycle_width", &["ctrl+alt-r"]),
        ("full_width", &["ctrl+alt-f"]),
        ("center", &["ctrl+alt-c"]),
        ("wider", &["ctrl+alt-equal"]),
        ("narrower", &["ctrl+alt-minus"]),
        ("taller", &["ctrl+alt+shift-equal"]),
        ("shorter", &["ctrl+alt+shift-minus"]),
        ("equalize_heights", &["ctrl+alt-0"]),
        ("stack_left", &["ctrl+alt-rightbracket"]),
        ("stack_right", &["ctrl+alt-leftbracket"]),
        ("unstack_right", &["ctrl+alt+shift-rightbracket"]),
        ("unstack_left", &["ctrl+alt+shift-leftbracket"]),
        ("toggle_tabbed", &["ctrl+alt+shift-t"]),
        ("toggle_float", &["ctrl+alt-t"]),
        ("send_next_display", &["ctrl+alt-n"]),
        ("pin_left", &["ctrl+alt-p"]),
        ("pin_right", &["ctrl+alt+shift-p"]),
        ("stop", &["ctrl+alt-q"]),
    ];
    pairs
        .iter()
        .map(|(action, keys)| {
            (
                (*action).to_string(),
                keys.iter().map(|key| (*key).to_string()).collect(),
            )
        })
        .collect()
}

/// On a shortcut-set upgrade, keep every binding the user already has.
/// Add a factory binding only when that key combination is unused.
pub fn merge_new_defaults(
    user: &BTreeMap<String, Vec<String>>,
    defaults: &BTreeMap<String, Vec<String>>,
) -> BTreeMap<String, Vec<String>> {
    let mut out = user.clone();
    let mut used: HashSet<String> = user.values().flatten().cloned().collect();
    for (action, keys) in defaults {
        let entry = out.entry(action.clone()).or_default();
        for key in keys {
            if entry.iter().any(|existing| existing == key) || used.contains(key) {
                continue;
            }
            entry.push(key.clone());
            used.insert(key.clone());
        }
    }
    out
}

pub fn resolve_bindings(
    specified: &BTreeMap<String, Vec<String>>,
    version: u32,
) -> BTreeMap<String, Vec<String>> {
    let defaults = default_bindings();
    let merged = if version < SHORTCUT_SET_VERSION {
        merge_new_defaults(specified, &defaults)
    } else {
        specified.clone()
    };
    let mut resolved = defaults;
    for (action, keys) in merged {
        resolved.insert(action, keys);
    }
    resolved
}

pub struct ResolvedBinding {
    pub action: String,
    pub n: Option<u32>,
}

/// The first action whose binding matches this key. Index shortcuts carry `n`.
pub fn lookup(bindings: &BTreeMap<String, Vec<String>>, pressed: &Binding) -> Option<ResolvedBinding> {
    for (action, keys) in bindings {
        for key in keys {
            let Ok(parsed) = parse_binding(key) else {
                continue;
            };
            if parsed == *pressed {
                return Some(ResolvedBinding {
                    action: action.clone(),
                    n: index_argument(key),
                });
            }
        }
    }
    None
}

/// Digit carried by a `focus_index` / `move_to_index` binding, if any.
pub fn index_argument(binding_text: &str) -> Option<u32> {
    let key = binding_text.rsplit_once('-')?.1;
    key.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_default_binding_round_trips() {
        for keys in default_bindings().values() {
            for key in keys {
                let parsed = parse_binding(key).unwrap_or_else(|err| panic!("{key}: {err}"));
                assert_eq!(format_binding(&parsed), *key);
            }
        }
    }

    #[test]
    fn keycode_4_is_h() {
        assert_eq!(keycode_for("h"), Some(4));
    }

    #[test]
    fn lookup_matches_only_bound_combinations() {
        let bindings = default_bindings();
        let right = lookup(&bindings, &parse_binding("ctrl+alt-l").unwrap()).unwrap();
        assert_eq!(right.action, "focus_right");
        assert_eq!(right.n, None);
        let index = lookup(&bindings, &parse_binding("ctrl+alt-3").unwrap()).unwrap();
        assert_eq!(index.action, "focus_index");
        assert_eq!(index.n, Some(3));
        let moved = lookup(&bindings, &parse_binding("ctrl+alt+shift-h").unwrap()).unwrap();
        assert_eq!(moved.action, "move_left");
        assert!(lookup(&bindings, &parse_binding("ctrl+alt-z").unwrap()).is_none());
    }

    #[test]
    fn upgrade_keeps_user_bindings_and_skips_conflicts() {
        let mut user = BTreeMap::new();
        user.insert("focus_left".into(), vec!["ctrl+alt-x".into()]);
        user.insert("cycle_width".into(), vec!["ctrl+alt-l".into()]);
        let merged = merge_new_defaults(&user, &default_bindings());

        assert_eq!(
            merged.get("focus_left").map(Vec::as_slice),
            Some(
                [
                    "ctrl+alt-x".to_string(),
                    "ctrl+alt-h".to_string(),
                    "ctrl+alt-left".to_string(),
                ]
                .as_slice()
            )
        );
        let focus_right = merged.get("focus_right").unwrap();
        assert!(!focus_right.iter().any(|key| key == "ctrl+alt-l"));
        assert!(focus_right.iter().any(|key| key == "ctrl+alt-right"));
        assert_eq!(
            merged.get("cycle_width").map(Vec::as_slice),
            Some(["ctrl+alt-l".to_string(), "ctrl+alt-r".to_string()].as_slice())
        );
        assert_eq!(merged.get("focus_left").unwrap()[0], "ctrl+alt-x");
    }

    #[test]
    fn same_version_does_not_restore_a_removed_key() {
        let mut specified = BTreeMap::new();
        specified.insert("focus_left".into(), vec!["ctrl+alt-h".into()]);
        let resolved = resolve_bindings(&specified, SHORTCUT_SET_VERSION);
        assert_eq!(
            resolved.get("focus_left").map(Vec::as_slice),
            Some(["ctrl+alt-h".to_string()].as_slice())
        );
        assert!(resolved.contains_key("focus_right"));
    }
}
