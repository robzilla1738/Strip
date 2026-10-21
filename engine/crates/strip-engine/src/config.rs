//! `config.toml` schema from Section 1.6. Unknown fields are kept (no deny).

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::bindings::{self, SHORTCUT_SET_VERSION};

pub const SECTION_1_6_EXAMPLE: &str = r##"
[general]
launch_at_login = true
start_managing_on_launch = true
show_in_dock = true
shortcut_set_version = 1

[layout]
preset_column_widths = [0.25, 0.33, 0.5, 0.66, 0.75]
default_column_width = 0.5
width_step = 0.10
gap_inner = 8            # 0–64 pt
gap_outer = 8            # 0–64 pt
dock_aware = true
center_focused_column = "never"   # never | on_overflow | always
keep_original_height = false

[animation]
animation_duration_ms = 250
animation_damping = 0.9

[swipe]
swipe_fingers = 4        # 0 | 3 | 4 | 5
swipe_natural = true
swipe_sensitivity = 1.5
swipe_deceleration = 4.0

[focus]
focus_follows_mouse = false
mouse_follows_focus = false
drag_to_reorder = true

[appearance]
show_indicator = true
focus_border = false
focus_border_width = 4
focus_border_color = "#4C8DFF"
focus_border_radius = 10
dim_inactive = false
dim_inactive_opacity = 0.25

[displays]
ignored_displays = []    # display UUID strings

[[rules]]
bundle_id = "com.apple.systempreferences"
floating = true

[bindings]
focus_left = ["ctrl+alt-h", "ctrl+alt-left"]
# … every action ID from 1.3, each a list of "mods-key" strings
"##;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CenterMode {
    #[default]
    Never,
    OnOverflow,
    Always,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    pub launch_at_login: bool,
    pub start_managing_on_launch: bool,
    pub show_in_dock: bool,
    pub shortcut_set_version: u32,
}

impl Default for General {
    fn default() -> Self {
        Self {
            launch_at_login: true,
            start_managing_on_launch: true,
            show_in_dock: true,
            shortcut_set_version: SHORTCUT_SET_VERSION,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub preset_column_widths: Vec<f64>,
    pub default_column_width: f64,
    pub width_step: f64,
    pub gap_inner: f64,
    pub gap_outer: f64,
    pub dock_aware: bool,
    pub center_focused_column: CenterMode,
    pub keep_original_height: bool,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            preset_column_widths: vec![0.25, 0.33, 0.5, 0.66, 0.75],
            default_column_width: 0.5,
            width_step: 0.10,
            gap_inner: 8.0,
            gap_outer: 8.0,
            dock_aware: true,
            center_focused_column: CenterMode::Never,
            keep_original_height: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Animation {
    pub animation_duration_ms: u32,
    pub animation_damping: f64,
}

impl Default for Animation {
    fn default() -> Self {
        Self {
            animation_duration_ms: 250,
            animation_damping: 0.9,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Swipe {
    pub swipe_fingers: u8,
    pub swipe_natural: bool,
    pub swipe_sensitivity: f64,
    pub swipe_deceleration: f64,
}

impl Default for Swipe {
    fn default() -> Self {
        Self {
            swipe_fingers: 4,
            swipe_natural: true,
            swipe_sensitivity: 1.5,
            swipe_deceleration: 4.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Focus {
    pub focus_follows_mouse: bool,
    pub mouse_follows_focus: bool,
    pub drag_to_reorder: bool,
}

impl Default for Focus {
    fn default() -> Self {
        Self {
            focus_follows_mouse: false,
            mouse_follows_focus: false,
            drag_to_reorder: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub show_indicator: bool,
    pub focus_border: bool,
    pub focus_border_width: f64,
    pub focus_border_color: String,
    pub focus_border_radius: f64,
    pub dim_inactive: bool,
    pub dim_inactive_opacity: f64,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            show_indicator: true,
            focus_border: false,
            focus_border_width: 4.0,
            focus_border_color: "#4C8DFF".to_string(),
            focus_border_radius: 10.0,
            dim_inactive: false,
            dim_inactive_opacity: 0.25,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Displays {
    pub ignored_displays: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    #[serde(default)]
    pub bundle_id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub subrole: Option<String>,
    #[serde(default)]
    pub floating: bool,
    #[serde(default)]
    pub slot: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub general: General,
    pub layout: Layout,
    pub animation: Animation,
    pub swipe: Swipe,
    pub focus: Focus,
    pub appearance: Appearance,
    pub displays: Displays,
    pub rules: Vec<Rule>,
    pub bindings: BTreeMap<String, Vec<String>>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: General::default(),
            layout: Layout::default(),
            animation: Animation::default(),
            swipe: Swipe::default(),
            focus: Focus::default(),
            appearance: Appearance::default(),
            displays: Displays::default(),
            rules: vec![Rule {
                bundle_id: Some("com.apple.systempreferences".to_string()),
                title: None,
                role: None,
                subrole: None,
                floating: true,
                slot: None,
            }],
            bindings: bindings::default_bindings(),
        }
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut user: toml::Value = toml::from_str(text).map_err(|err| err.to_string())?;
        let version = shortcut_version(&user).unwrap_or(SHORTCUT_SET_VERSION);
        let specified = take_bindings(&mut user)?;
        let mut base = toml::Value::try_from(Config::default()).map_err(|err| err.to_string())?;
        let _ = take_bindings(&mut base)?;
        let merged = overlay(base, user);
        let mut parsed: Config = merged.try_into().map_err(|err: toml::de::Error| err.to_string())?;
        parsed.bindings = bindings::resolve_bindings(&specified, version);
        parsed.general.shortcut_set_version = version.max(SHORTCUT_SET_VERSION);
        parsed.clamp()?;
        Ok(parsed)
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path).map_err(|err| err.to_string())?;
        Self::parse(&text)
    }

    fn clamp(&mut self) -> Result<(), String> {
        self.layout.gap_inner = self.layout.gap_inner.clamp(0.0, 64.0);
        self.layout.gap_outer = self.layout.gap_outer.clamp(0.0, 64.0);
        self.layout.default_column_width = self.layout.default_column_width.clamp(0.1, 1.0);
        self.layout.width_step = self.layout.width_step.clamp(0.01, 1.0);
        if !matches!(self.swipe.swipe_fingers, 0 | 3 | 4 | 5) {
            self.swipe.swipe_fingers = 4;
        }
        for rule in &self.rules {
            if let Some(title) = &rule.title {
                regex::Regex::new(title).map_err(|err| format!("rule title `{title}`: {err}"))?;
            }
        }
        Ok(())
    }
}

fn shortcut_version(value: &toml::Value) -> Option<u32> {
    let number = value.get("general")?.get("shortcut_set_version")?.as_integer()?;
    u32::try_from(number).ok()
}

fn take_bindings(value: &mut toml::Value) -> Result<BTreeMap<String, Vec<String>>, String> {
    let Some(table) = value.as_table_mut() else {
        return Ok(BTreeMap::new());
    };
    let Some(bindings) = table.remove("bindings") else {
        return Ok(BTreeMap::new());
    };
    BTreeMap::deserialize(bindings).map_err(|err| err.to_string())
}

/// Tables merge key by key. Arrays and scalars from the file replace the default.
fn overlay(base: toml::Value, user: toml::Value) -> toml::Value {
    match (base, user) {
        (toml::Value::Table(mut base_table), toml::Value::Table(user_table)) => {
            for (key, value) in user_table {
                let next = if let Some(existing) = base_table.remove(&key) {
                    overlay(existing, value)
                } else {
                    value
                };
                base_table.insert(key, next);
            }
            toml::Value::Table(base_table)
        }
        (_, user) => user,
    }
}

/// First matching rule wins. A rule matches when every field it sets matches.
pub fn first_rule<'a>(
    rules: &'a [Rule],
    bundle_id: &str,
    title: &str,
    role: &str,
    subrole: &str,
) -> Option<&'a Rule> {
    rules.iter().find(|rule| {
        if let Some(expected) = &rule.bundle_id {
            if expected != bundle_id {
                return false;
            }
        }
        if let Some(pattern) = &rule.title {
            let Ok(expression) = regex::Regex::new(pattern) else {
                return false;
            };
            if !expression.is_match(title) {
                return false;
            }
        }
        if let Some(expected) = &rule.role {
            if expected != role {
                return false;
            }
        }
        if let Some(expected) = &rule.subrole {
            if expected != subrole {
                return false;
            }
        }
        true
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    Manage { slot: Option<u32> },
    Float,
    Fullscreen,
}

pub struct AdmitWindow<'a> {
    pub bundle_id: &'a str,
    pub title: &'a str,
    pub role: &'a str,
    pub subrole: &'a str,
    pub size_settable: bool,
    pub fixed_size: bool,
    pub fullscreen: bool,
    pub own_process: bool,
}

/// Rules that force floating win first. Dialogs, fixed-size windows, Quick Look,
/// and this process float even without a rule. A slot rule applies only when
/// the window would otherwise be managed.
pub fn admit(rules: &[Rule], window: AdmitWindow<'_>) -> Placement {
    if window.fullscreen {
        return Placement::Fullscreen;
    }
    if let Some(rule) = first_rule(rules, window.bundle_id, window.title, window.role, window.subrole) {
        if rule.floating {
            return Placement::Float;
        }
    }
    if window.own_process || is_quicklook(window.bundle_id) {
        return Placement::Float;
    }
    if !window.subrole.is_empty() && window.subrole != "AXStandardWindow" {
        return Placement::Float;
    }
    if !window.size_settable || window.fixed_size {
        return Placement::Float;
    }
    if let Some(rule) = first_rule(rules, window.bundle_id, window.title, window.role, window.subrole) {
        if let Some(slot) = rule.slot {
            return Placement::Manage { slot: Some(slot) };
        }
    }
    Placement::Manage { slot: None }
}

pub fn shown_bundle(bundle_id: &str, owner: &str) -> String {
    if bundle_id.is_empty() { owner.to_string() } else { bundle_id.to_string() }
}

fn is_quicklook(bundle: &str) -> bool {
    bundle.to_ascii_lowercase().starts_with("com.apple.quicklook") || bundle.starts_with("com.apple.QuickLookUIFramework")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_section_1_6_example_as_the_defaults() {
        let parsed = Config::parse(SECTION_1_6_EXAMPLE).expect("example parses");
        let defaults = Config::default();
        assert_eq!(parsed.general, defaults.general);
        assert_eq!(parsed.layout, defaults.layout);
        assert_eq!(parsed.animation, defaults.animation);
        assert_eq!(parsed.swipe, defaults.swipe);
        assert_eq!(parsed.focus, defaults.focus);
        assert_eq!(parsed.appearance, defaults.appearance);
        assert_eq!(parsed.displays, defaults.displays);
        assert_eq!(parsed.rules, defaults.rules);
        assert_eq!(parsed.bindings, defaults.bindings);
        assert_eq!(parsed.layout.preset_column_widths, vec![0.25, 0.33, 0.5, 0.66, 0.75]);
        assert_eq!(parsed.layout.default_column_width, 0.5);
        assert_eq!(parsed.layout.width_step, 0.10);
        assert_eq!(parsed.layout.gap_inner, 8.0);
        assert_eq!(parsed.layout.gap_outer, 8.0);
        assert!(parsed.layout.dock_aware);
        assert_eq!(parsed.layout.center_focused_column, CenterMode::Never);
        assert!(!parsed.layout.keep_original_height);
        assert_eq!(parsed.animation.animation_duration_ms, 250);
        assert_eq!(parsed.animation.animation_damping, 0.9);
        assert_eq!(parsed.swipe.swipe_fingers, 4);
        assert!(parsed.swipe.swipe_natural);
        assert_eq!(parsed.swipe.swipe_sensitivity, 1.5);
        assert_eq!(parsed.swipe.swipe_deceleration, 4.0);
        assert!(!parsed.focus.focus_follows_mouse);
        assert!(!parsed.focus.mouse_follows_focus);
        assert!(parsed.focus.drag_to_reorder);
        assert!(parsed.appearance.show_indicator);
        assert!(!parsed.appearance.focus_border);
        assert_eq!(parsed.appearance.focus_border_color, "#4C8DFF");
        assert!(!parsed.appearance.dim_inactive);
        assert_eq!(parsed.appearance.dim_inactive_opacity, 0.25);
        assert!(parsed.displays.ignored_displays.is_empty());
        assert_eq!(parsed.general.shortcut_set_version, 1);
        assert!(parsed.general.launch_at_login);
        assert!(parsed.general.start_managing_on_launch);
        assert!(parsed.general.show_in_dock);
    }

    #[test]
    fn unknown_fields_are_accepted() {
        let text = format!("{SECTION_1_6_EXAMPLE}\n[future]\nextra = true\n");
        assert!(Config::parse(&text).is_ok());
    }

    fn sample<'a>(title: &'a str, subrole: &'a str) -> AdmitWindow<'a> {
        AdmitWindow {
            bundle_id: "TestWindows",
            title,
            role: "AXWindow",
            subrole,
            size_settable: true,
            fixed_size: false,
            fullscreen: false,
            own_process: false,
        }
    }

    #[test]
    fn standard_window_is_managed() {
        let rules = Config::default().rules;
        assert_eq!(admit(&rules, sample("Window 1", "AXStandardWindow")), Placement::Manage { slot: None });
    }

    #[test]
    fn title_rule_floats_and_dialog_and_fixed_size_float() {
        let rules = vec![Rule {
            bundle_id: Some("TestWindows".to_string()),
            title: Some("Float.*".to_string()),
            role: None,
            subrole: None,
            floating: true,
            slot: None,
        }];
        assert_eq!(admit(&rules, sample("Float 1", "AXStandardWindow")), Placement::Float);
        assert_eq!(admit(&rules, sample("Window 1", "AXStandardWindow")), Placement::Manage { slot: None });
        let mut dialog = sample("Dialog 1", "AXSystemDialog");
        dialog.bundle_id = "TestWindows";
        assert_eq!(admit(&rules, dialog), Placement::Float);
        let mut fixed = sample("Fixed 1", "AXStandardWindow");
        fixed.fixed_size = true;
        assert_eq!(admit(&rules, fixed), Placement::Float);
        let mut quick = sample("Preview", "AXStandardWindow");
        quick.bundle_id = "com.apple.quicklook.QuickLookUIService";
        assert_eq!(admit(&[], quick), Placement::Float);
        let mut own = sample("Strip", "AXStandardWindow");
        own.own_process = true;
        assert_eq!(admit(&[], own), Placement::Float);
    }

    #[test]
    fn owner_name_fills_an_empty_bundle_id() {
        assert_eq!(shown_bundle("", "TestWindows"), "TestWindows");
        assert_eq!(shown_bundle("local.app", "TestWindows"), "local.app");
    }

    #[test]
    fn empty_file_is_every_default() {
        let parsed = Config::parse("").expect("empty");
        assert_eq!(parsed, Config::default());
    }
}
