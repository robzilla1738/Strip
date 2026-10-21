//! One strip per display. Publishes frames through the writer pool.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use strip_core::{layout, CenterMode, DisplayFrame, DisplayMode, Focus, Frame, LayoutParams, Strip, Viewport, Visibility, Window};

use crate::ax::{self, AxApi, DisplayInfo, ListedWindow};
use crate::config::Config;
use crate::parking::restored_frame;
use crate::private_api::PrivateApi;
use crate::writer::{Clock, SystemClock, WriterPool};

pub struct Runtime {
    config: Config,
    pool: WriterPool<AxApi>,
    strips: HashMap<String, Strip>,
    meta: HashMap<u64, Meta>,
    last_on_screen: HashMap<u64, Frame>,
    state: Arc<Mutex<String>>,
    metrics_body: Arc<Mutex<String>>,
    only_owner: Option<String>,
    managing: bool,
    ax_trusted: bool,
    screens: Vec<DisplayInfo>,
    config_path: Option<PathBuf>,
}

struct Meta {
    pid: i32,
    bundle_id: String,
    title: String,
    owner: String,
    display: String,
}

impl Runtime {
    pub fn start(
        config: Config,
        config_path: Option<PathBuf>,
        private_api: &PrivateApi,
        state: Arc<Mutex<String>>,
        metrics_body: Arc<Mutex<String>>,
        ax_trusted: bool,
    ) -> Self {
        ax::prepare_app();
        let only_owner = std::env::var("STRIP_ONLY_OWNER").ok().filter(|value| !value.is_empty());
        Self {
            config,
            pool: WriterPool::new(AxApi::new(private_api), Arc::new(SystemClock) as Arc<dyn Clock>),
            strips: HashMap::new(),
            meta: HashMap::new(),
            last_on_screen: HashMap::new(),
            state,
            metrics_body,
            only_owner,
            managing: true,
            ax_trusted,
            screens: Vec::new(),
            config_path,
        }
    }

    pub fn tick(&mut self) {
        if !self.managing {
            self.publish_state(&[]);
            return;
        }
        let displays = ax::displays(self.config.layout.dock_aware);
        self.screens = displays.clone();
        let mut listed = self.eligible(&ax::on_screen_windows());
        listed.sort_by_key(|window| window.id);
        self.pool.with_api(|api| ax::resolve_elements(api, &listed));
        let front = ax::frontmost_pid();
        let params = layout_params(&self.config);
        let mut seen = HashSet::new();
        for window in &listed {
            seen.insert(window.id);
            let Some(display) = display_for(&displays, window.bounds) else {
                continue;
            };
            let strip = self.strips.entry(display.uuid.clone()).or_default();
            if strip.contains(strip_core::WindowId(window.id)) {
                continue;
            }
            let mut model = Window::new(window.id, window.pid);
            model.original_height = window.bounds.height;
            let visible = viewport(&display.visible);
            strip.insert(model, front, None, &params, visible);
            self.meta.insert(
                window.id,
                Meta {
                    pid: window.pid,
                    bundle_id: ax::bundle_id(window.pid),
                    title: window.title.clone(),
                    owner: window.owner.clone(),
                    display: display.uuid.clone(),
                },
            );
        }
        self.remove_missing(&seen, &params);
        self.publish_layouts(&displays, &params);
        self.pool.poll_enhanced_ui();
        self.publish_state(&self.screens);
        self.store_metrics();
    }

    pub fn set_managing(&mut self, enabled: bool) {
        if enabled {
            self.managing = true;
        } else {
            self.unpark_all();
        }
    }

    pub fn reload(&mut self) -> Result<(), String> {
        let Some(path) = self.config_path.clone() else {
            return Ok(());
        };
        self.config = Config::load(&path)?;
        Ok(())
    }

    /// Apply one command on the calling thread. Focus actions return a snapshot
    /// even when the column index does not change.
    pub fn perform(&mut self, action: &str, args: &serde_json::Value) -> Result<Option<FocusSnap>, CommandError> {
        if !strip_ipc::is_known_action(action) {
            return Err(CommandError::UnknownAction);
        }
        if !self.ax_trusted && action != "reload_config" {
            return Err(CommandError::NotTrusted);
        }
        match action {
            "stop" => {
                self.unpark_all();
                return Ok(None);
            }
            "start" => {
                self.managing = true;
                return Ok(None);
            }
            "reload_config" => {
                self.reload().map_err(CommandError::BadRequest)?;
                return Ok(None);
            }
            "toggle_float" | "send_next_display" => return Ok(None),
            _ => {}
        }
        let display = self.resolve_display(action, args)?;
        let params = layout_params(&self.config);
        let visible = self.viewport_of(&display);
        let focus_action = is_focus_action(action);
        {
            let strip = self.strips.get_mut(&display).ok_or(CommandError::NoFocusedWindow)?;
            if focus_action && strip.columns.is_empty() && strip.pinned_left.is_none() && strip.pinned_right.is_none() {
                return Err(CommandError::NoFocusedWindow);
            }
            apply_action(strip, action, args, &params, visible)?;
        }
        let screens = self.screens.clone();
        self.publish_layouts(&screens, &params);
        self.publish_state(&screens);
        if focus_action {
            Ok(Some(self.focus_snap(&display)?))
        } else {
            Ok(None)
        }
    }

    pub fn unpark_all(&mut self) {
        let displays = ax::displays(self.config.layout.dock_aware);
        let visible = displays.first().map(|display| display.visible).unwrap_or(Frame {
            x: 0.0,
            y: 0.0,
            width: 1280.0,
            height: 800.0,
        });
        let ids: Vec<u64> = self.meta.keys().copied().collect();
        for (cascade, id) in ids.into_iter().enumerate() {
            let frame = restored_frame(self.last_on_screen.get(&id).copied(), visible, cascade);
            let pid = self.meta.get(&id).map(|meta| meta.pid).unwrap_or(0);
            let bundle = self.meta.get(&id).map(|meta| meta.bundle_id.clone()).unwrap_or_default();
            self.pool.publish(pid, id, frame, &bundle);
        }
        let deadline = Instant::now() + Duration::from_millis(800);
        while !self.pool.is_idle() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        self.managing = false;
    }

    fn eligible(&self, listed: &[ListedWindow]) -> Vec<ListedWindow> {
        listed
            .iter()
            .filter(|window| {
                let ignored = matches!(
                    window.owner.as_str(),
                    "Window Server" | "Dock" | "SystemUIServer" | "Control Center" | "Notification Center" | "Spotlight"
                );
                if ignored {
                    return false;
                }
                self.only_owner.as_ref().is_none_or(|owner| window.owner == *owner)
            })
            .cloned()
            .collect()
    }

    fn remove_missing(&mut self, seen: &HashSet<u64>, params: &LayoutParams) {
        let stale: Vec<(String, u64)> = self
            .meta
            .iter()
            .filter(|(id, _)| !seen.contains(id))
            .map(|(id, meta)| (meta.display.clone(), *id))
            .collect();
        for (display, id) in stale {
            if let Some(strip) = self.strips.get_mut(&display) {
                let visible = Viewport { x: 0.0, y: 0.0, width: 1.0, height: 1.0 };
                strip.remove(strip_core::WindowId(id), params, visible);
            }
            self.meta.remove(&id);
            self.last_on_screen.remove(&id);
        }
    }

    fn publish_layouts(&mut self, displays: &[DisplayInfo], params: &LayoutParams) {
        for display in displays {
            let Some(strip) = self.strips.get_mut(&display.uuid) else {
                continue;
            };
            let visible = viewport(&display.visible);
            let screen = DisplayFrame {
                min_x: display.frame.x,
                min_y: display.frame.y,
                max_x: display.frame.right(),
                max_y: display.frame.bottom(),
                neighbour_left: display.neighbour_left,
                neighbour_right: display.neighbour_right,
            };
            let placements = layout(strip, visible, screen, params);
            for placement in placements {
                let id = placement.window.0;
                if matches!(placement.visibility, Visibility::Visible | Visibility::Partial) {
                    self.last_on_screen.insert(id, placement.frame);
                }
                let Some(meta) = self.meta.get(&id) else {
                    continue;
                };
                self.pool.publish(meta.pid, id, placement.frame, &meta.bundle_id);
            }
        }
    }

    fn publish_state(&self, displays: &[DisplayInfo]) {
        let body = serde_json::json!({
            "managing": self.managing,
            "displays": displays.iter().map(|display| self.display_json(display)).collect::<Vec<_>>(),
            "floating": [],
        });
        if let Ok(text) = serde_json::to_string(&body) {
            *self.state.lock().unwrap_or_else(|err| err.into_inner()) = text;
        }
    }

    fn display_json(&self, display: &DisplayInfo) -> serde_json::Value {
        let strip = self.strips.get(&display.uuid);
        let focused = strip
            .map(|strip| match strip.focus {
                Focus::Column(index) => index as i64,
                _ => 0,
            })
            .unwrap_or(0);
        let columns = strip.map(|strip| self.columns_json(strip)).unwrap_or_default();
        serde_json::json!({
            "uuid": display.uuid,
            "name": display.name,
            "frame": frame_json(display.frame),
            "visible_frame": frame_json(display.visible),
            "ignored": false,
            "active_space": 1,
            "strips": [{
                "space": 1,
                "scroll_x": strip.map(|strip| strip.scroll_x).unwrap_or(0.0),
                "focused_column": focused,
                "pinned_left": null,
                "pinned_right": null,
                "columns": columns,
            }],
        })
    }
}

impl Runtime {
    fn columns_json(&self, strip: &Strip) -> Vec<serde_json::Value> {
        strip
            .columns
            .iter()
            .enumerate()
            .map(|(index, column)| {
                serde_json::json!({
                    "index": index,
                    "width_ratio": column.width_ratio,
                    "mode": match column.mode {
                        DisplayMode::Stacked => "stacked",
                        DisplayMode::Tabbed => "tabbed",
                    },
                    "active_tab": column.active_tab,
                    "windows": column.windows.iter().map(|window| self.window_json(window)).collect::<Vec<_>>(),
                })
            })
            .collect()
    }

    fn window_json(&self, window: &Window) -> serde_json::Value {
        let meta = self.meta.get(&window.id.0);
        let frame = self.last_on_screen.get(&window.id.0).copied().unwrap_or(Frame {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        });
        let bundle = meta
            .map(|meta| {
                if meta.bundle_id.is_empty() {
                    meta.owner.clone()
                } else {
                    meta.bundle_id.clone()
                }
            })
            .unwrap_or_default();
        serde_json::json!({
            "id": window.id.0,
            "pid": window.pid,
            "bundle_id": bundle,
            "title": meta.map(|meta| meta.title.clone()).unwrap_or_default(),
            "frame": frame_json(frame),
            "target": frame_json(frame),
            "visible": true,
            "height_ratio": window.height_ratio,
        })
    }
}

pub struct FocusSnap {
    pub window_id: u64,
    pub display: String,
    pub column: i64,
}

pub enum CommandError {
    BadRequest(String),
    UnknownAction,
    NoFocusedWindow,
    NotTrusted,
    DisplayNotFound,
}

impl CommandError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::BadRequest(_) => "bad_request",
            Self::UnknownAction => "unknown_action",
            Self::NoFocusedWindow => "no_focused_window",
            Self::NotTrusted => "not_trusted",
            Self::DisplayNotFound => "display_not_found",
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::BadRequest(message) => message,
            Self::UnknownAction => "unknown action",
            Self::NoFocusedWindow => "no focused window",
            Self::NotTrusted => "accessibility is not trusted",
            Self::DisplayNotFound => "display not found",
        }
    }
}

impl Runtime {
    fn store_metrics(&self) {
        let metrics = self.pool.metrics();
        let mut ax_write_ms = serde_json::Map::new();
        for (bundle, timing) in metrics.bundles() {
            ax_write_ms.insert(
                bundle,
                serde_json::json!({
                    "p50": timing.p50_ms,
                    "p95": timing.p95_ms,
                    "max": timing.max_ms,
                }),
            );
        }
        let body = serde_json::json!({
            "ax_write_ms": ax_write_ms,
            "frames_per_second": metrics.frames_per_second,
            "last_frame_ms": metrics.last_frame_ms,
            "idle_cpu": metrics.idle_cpu,
            "size_writes_during_animation": metrics.size_writes_during_animation,
            "display_link_running": metrics.display_link_running,
        });
        if let Ok(text) = serde_json::to_string(&body) {
            *self.metrics_body.lock().unwrap_or_else(|err| err.into_inner()) = text;
        }
    }

    fn resolve_display(&self, action: &str, args: &serde_json::Value) -> Result<String, CommandError> {
        if action == "focus_window" {
            let id = required_u64(args, "window_id")?;
            return self
                .locate_window(id)
                .map(|(uuid, _)| uuid)
                .ok_or(CommandError::NoFocusedWindow);
        }
        if action == "move_column" {
            if let Some(uuid) = args.get("display").and_then(|value| value.as_str()) {
                let known = self.screens.iter().any(|screen| screen.uuid == uuid) || self.strips.contains_key(uuid);
                if !known {
                    return Err(CommandError::DisplayNotFound);
                }
                return Ok(uuid.to_string());
            }
        }
        self.active_display().ok_or(CommandError::NoFocusedWindow)
    }

    fn active_display(&self) -> Option<String> {
        let occupied = self.screens.iter().find_map(|screen| {
            let strip = self.strips.get(&screen.uuid)?;
            let occupied = !strip.columns.is_empty() || strip.pinned_left.is_some() || strip.pinned_right.is_some();
            occupied.then(|| screen.uuid.clone())
        });
        occupied.or_else(|| self.screens.first().map(|screen| screen.uuid.clone()))
    }

    fn viewport_of(&self, uuid: &str) -> Viewport {
        self.screens
            .iter()
            .find(|screen| screen.uuid == uuid)
            .map(|screen| viewport(&screen.visible))
            .unwrap_or(Viewport { x: 0.0, y: 0.0, width: 1.0, height: 1.0 })
    }

    fn locate_window(&self, id: u64) -> Option<(String, usize)> {
        for (uuid, strip) in &self.strips {
            if let Some(index) = strip.columns.iter().position(|column| column.windows.iter().any(|window| window.id.0 == id)) {
                return Some((uuid.clone(), index));
            }
        }
        None
    }

    fn focus_snap(&self, display: &str) -> Result<FocusSnap, CommandError> {
        let strip = self.strips.get(display).ok_or(CommandError::NoFocusedWindow)?;
        let (column, window_id) = match strip.focus {
            Focus::Column(index) => {
                let column_model = strip.columns.get(index).ok_or(CommandError::NoFocusedWindow)?;
                let window = column_model
                    .windows
                    .get(column_model.focused_window)
                    .or_else(|| column_model.windows.first())
                    .ok_or(CommandError::NoFocusedWindow)?;
                (index as i64, window.id.0)
            }
            Focus::PinnedLeft => {
                let window = strip
                    .pinned_left
                    .as_ref()
                    .and_then(|pin| pin.column.windows.first())
                    .ok_or(CommandError::NoFocusedWindow)?;
                (0, window.id.0)
            }
            Focus::PinnedRight => {
                let window = strip
                    .pinned_right
                    .as_ref()
                    .and_then(|pin| pin.column.windows.first())
                    .ok_or(CommandError::NoFocusedWindow)?;
                (0, window.id.0)
            }
            Focus::None => return Err(CommandError::NoFocusedWindow),
        };
        Ok(FocusSnap { window_id, display: display.to_string(), column })
    }
}

fn is_focus_action(action: &str) -> bool {
    matches!(
        action,
        "focus_left" | "focus_right" | "focus_up" | "focus_down" | "focus_first" | "focus_last" | "focus_index" | "focus_window"
    )
}

fn apply_action(strip: &mut Strip, action: &str, args: &serde_json::Value, params: &LayoutParams, visible: Viewport) -> Result<(), CommandError> {
    match action {
        "focus_left" => strip.focus_left(params, visible),
        "focus_right" => strip.focus_right(params, visible),
        "focus_up" => strip.focus_up(params, visible),
        "focus_down" => strip.focus_down(params, visible),
        "focus_first" => strip.focus_first(params, visible),
        "focus_last" => strip.focus_last(params, visible),
        "focus_index" => strip.focus_index(required_u32(args, "n")?, params, visible),
        "move_to_index" => strip.move_to_index(required_u32(args, "n")?, params, visible),
        "move_left" => strip.move_left(params, visible),
        "move_right" => strip.move_right(params, visible),
        "move_up" => strip.move_up(params, visible),
        "move_down" => strip.move_down(params, visible),
        "move_first" => strip.move_to_index(1, params, visible),
        "move_last" => strip.move_to_index(9, params, visible),
        "cycle_width" => strip.cycle_width(params, visible),
        "full_width" => strip.full_width(params, visible),
        "center" => strip.center(params, visible),
        "wider" => strip.wider(params, visible),
        "narrower" => strip.narrower(params, visible),
        "taller" => strip.taller(params, visible),
        "shorter" => strip.shorter(params, visible),
        "equalize_heights" => strip.equalize_heights(params, visible),
        "stack_left" => strip.stack_left(params, visible),
        "stack_right" => strip.stack_right(params, visible),
        "unstack_left" => strip.unstack_left(params, visible),
        "unstack_right" => strip.unstack_right(params, visible),
        "toggle_tabbed" => strip.toggle_tabbed(),
        "pin_left" => strip.pin_left(params, visible),
        "pin_right" => strip.pin_right(params, visible),
        "focus_window" => {
            let id = required_u64(args, "window_id")?;
            if strip.pinned_left.as_ref().is_some_and(|pin| pin.column.windows.iter().any(|window| window.id.0 == id)) {
                strip.focus = Focus::PinnedLeft;
            } else if strip.pinned_right.as_ref().is_some_and(|pin| pin.column.windows.iter().any(|window| window.id.0 == id)) {
                strip.focus = Focus::PinnedRight;
            } else {
                let index = strip
                    .columns
                    .iter()
                    .position(|column| column.windows.iter().any(|window| window.id.0 == id))
                    .ok_or(CommandError::NoFocusedWindow)?;
                strip.focus = Focus::Column(index);
            }
            strip.reveal(params, visible);
        }
        "move_column" => {
            let from = required_u32(args, "from")? as usize;
            let to = required_u32(args, "to")? as usize;
            strip.move_column(from, to);
        }
        _ => return Err(CommandError::UnknownAction),
    }
    Ok(())
}

fn required_u32(args: &serde_json::Value, key: &str) -> Result<u32, CommandError> {
    args.get(key)
        .and_then(serde_json::Value::as_u64)
        .map(|value| value as u32)
        .ok_or_else(|| CommandError::BadRequest(format!("missing {key}")))
}

fn required_u64(args: &serde_json::Value, key: &str) -> Result<u64, CommandError> {
    args.get(key)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| CommandError::BadRequest(format!("missing {key}")))
}

fn frame_json(frame: Frame) -> [f64; 4] {
    [frame.x, frame.y, frame.width, frame.height]
}

fn viewport(frame: &Frame) -> Viewport {
    Viewport {
        x: frame.x,
        y: frame.y,
        width: frame.width,
        height: frame.height,
    }
}

fn display_for(displays: &[DisplayInfo], frame: Frame) -> Option<&DisplayInfo> {
    let center_x = frame.x + frame.width / 2.0;
    let center_y = frame.y + frame.height / 2.0;
    displays.iter().find(|display| {
        center_x >= display.frame.x
            && center_x < display.frame.right()
            && center_y >= display.frame.y
            && center_y < display.frame.bottom()
    })
}

fn layout_params(config: &Config) -> LayoutParams {
    LayoutParams {
        gap_inner: config.layout.gap_inner,
        gap_outer: config.layout.gap_outer,
        sliver: 5.0,
        center: match config.layout.center_focused_column {
            crate::config::CenterMode::Never => CenterMode::Never,
            crate::config::CenterMode::OnOverflow => CenterMode::OnOverflow,
            crate::config::CenterMode::Always => CenterMode::Always,
        },
        keep_original_height: config.layout.keep_original_height,
        presets: config.layout.preset_column_widths.clone(),
        width_step: config.layout.width_step,
        default_column_width: config.layout.default_column_width,
    }
}


