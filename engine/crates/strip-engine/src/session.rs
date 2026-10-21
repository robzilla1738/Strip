//! One strip per display. Publishes frames through the writer pool.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use strip_core::{
    layout, CenterMode, DisplayFrame, DisplayMode, Focus, Frame, Gesture, LayoutParams, Spring, Strip, SwipeView, Viewport, Visibility, Window, WindowId,
};

use crate::ax::{self, AxApi, DisplayInfo, ListedWindow, Traits};
use crate::config::{self, AdmitWindow, Config, Placement};
use crate::display_link::DisplayDriver;
use crate::parking::restored_frame;
use crate::private_api::PrivateApi;
use crate::restore::{self, SavedSession};
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
    motions: HashMap<u64, Motion>,
    driver: DisplayDriver,
    animating: bool,
    gesture: Gesture,
    saved: Option<SavedSession>,
    restore_until: Instant,
    restored_members: HashMap<(String, usize), Vec<u64>>,
    scroll_applied: HashSet<String>,
    floating: HashMap<u64, FloatingWindow>,
    fullscreen: HashMap<u64, Departed>,
    departed: HashMap<u64, Departed>,
    settle: bool,
    force_resolve: bool,
    space_id: u64,
    pointer_x: f64,
    pointer_y: f64,
    pointer_at: Instant,
    pointer_seen: bool,
    drag: Option<Drag>,
}

struct Motion {
    x: Spring,
    y: Spring,
    width: f64,
    height: f64,
    pid: i32,
    bundle_id: String,
    pending_shrink: Option<(f64, f64)>,
}

impl Motion {
    fn rest(pid: i32, bundle: &str, frame: Frame) -> Self {
        Self {
            x: Spring::launch(frame.x, frame.x),
            y: Spring::launch(frame.y, frame.y),
            width: frame.width,
            height: frame.height,
            pid,
            bundle_id: bundle.to_string(),
            pending_shrink: None,
        }
    }

    fn settled(&self) -> bool {
        (self.x.x - self.x.target).abs() < 0.5 && self.x.v.abs() < 5.0 && (self.y.x - self.y.target).abs() < 0.5 && self.y.v.abs() < 5.0
    }

    fn rendered(&self) -> Frame {
        Frame {
            x: self.x.x,
            y: self.y.x,
            width: self.width,
            height: self.height,
        }
    }

    fn snap(&mut self, frame: Frame) {
        self.x = Spring::launch(frame.x, frame.x);
        self.y = Spring::launch(frame.y, frame.y);
        self.width = frame.width;
        self.height = frame.height;
        self.pending_shrink = None;
    }
}

#[derive(Clone)]
struct Meta {
    pid: i32,
    bundle_id: String,
    title: String,
    owner: String,
    display: String,
    role: String,
    subrole: String,
}

struct FloatingWindow {
    title: String,
    pid: i32,
    bundle_id: String,
}

struct Departed {
    window: Window,
    index: usize,
    display: String,
}

struct Drag {
    id: u64,
    display: String,
    column: usize,
}

impl Runtime {
    pub fn start(
        config: Config,
        config_path: Option<PathBuf>,
        state_path: Option<PathBuf>,
        private_api: &PrivateApi,
        state: Arc<Mutex<String>>,
        metrics_body: Arc<Mutex<String>>,
        ax_trusted: bool,
    ) -> Self {
        ax::prepare_app();
        let only_owner = std::env::var("STRIP_ONLY_OWNER").ok().filter(|value| !value.is_empty());
        let saved = state_path.as_ref().map(|path| restore::load_session(path)).filter(|session| !session.windows.is_empty());
        let now = Instant::now();
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
            motions: HashMap::new(),
            driver: DisplayDriver::new(),
            animating: false,
            gesture: Gesture::new(),
            saved,
            restore_until: now + Duration::from_secs(10),
            restored_members: HashMap::new(),
            scroll_applied: HashSet::new(),
            floating: HashMap::new(),
            fullscreen: HashMap::new(),
            departed: HashMap::new(),
            settle: false,
            force_resolve: false,
            space_id: crate::private_api::current_space(),
            pointer_x: 0.0,
            pointer_y: 0.0,
            pointer_at: now,
            pointer_seen: false,
            drag: None,
        }
    }

    pub fn drive_swipe(&mut self, fingers: u8, dx: f64, dy: f64, phase: &str) {
        let _ = self.apply_swipe(fingers, dx, dy, phase);
    }

    fn apply_swipe(&mut self, fingers: u8, dx: f64, dy: f64, phase: &str) -> Result<(), CommandError> {
        let display = self.active_display().ok_or(CommandError::NoFocusedWindow)?;
        let visible = self.viewport_of(&display);
        let ratios: Vec<f64> = self
            .strips
            .get(&display)
            .map(|strip| strip.columns.iter().map(|column| column.width_ratio).collect())
            .unwrap_or_default();
        let scroll = self.strips.get(&display).map(|strip| strip.scroll_x).unwrap_or(0.0);
        let view = SwipeView::from_ratios(
            &ratios,
            visible.width,
            self.config.layout.gap_inner,
            self.config.swipe.swipe_sensitivity,
            self.config.swipe.swipe_natural,
            self.config.swipe.swipe_deceleration,
        );
        let update = match phase {
            "began" => {
                self.gesture.begin(fingers, self.config.swipe.swipe_fingers, scroll);
                strip_core::SwipeUpdate::default()
            }
            "moved" => self.gesture.moved(dx, dy, &view, 1.0 / 60.0),
            "ended" => self.gesture.ended(&view),
            _ => return Err(CommandError::BadRequest("phase must be began, moved, or ended".to_string())),
        };
        let Some(scroll) = update.scroll else {
            return Ok(());
        };
        if let Some(strip) = self.strips.get_mut(&display) {
            strip.scroll_x = scroll;
            if let Some(index) = update.focus {
                if index < strip.columns.len() {
                    strip.focus = Focus::Column(index);
                }
            }
        }
        let screens = self.screens.clone();
        let params = layout_params(&self.config);
        self.publish_layouts(&screens, &params);
        self.publish_state(&screens);
        Ok(())
    }

    pub fn is_animating(&self) -> bool {
        self.animating
    }

    pub fn drain_frames(&mut self) {
        let dts = self.driver.take_dts();
        for dt in dts {
            self.step_animation(dt);
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
        let need_resolve = self.force_resolve || listed.iter().any(|window| !self.meta.contains_key(&window.id) && !self.floating.contains_key(&window.id));
        if need_resolve || self.saved.is_some() {
            self.pool.with_api(|api| ax::resolve_elements(api, &listed));
        }
        let seen: HashSet<u64> = listed.iter().map(|window| window.id).collect();
        self.floating.retain(|id, _| seen.contains(id));
        let restoring = self.saved.as_ref().is_some_and(|saved| !saved.windows.is_empty()) && Instant::now() <= self.restore_until;
        if restoring {
            self.settle = true;
            self.restore_arrived(&listed);
        }
        if Instant::now() > self.restore_until {
            self.saved = None;
        }
        let front = ax::frontmost_pid();
        let params = layout_params(&self.config);
        let own_pid = std::process::id() as i32;
        let mut fresh = Vec::new();
        for window in &listed {
            if self.floating.contains_key(&window.id) || self.contains_id(window.id) {
                continue;
            }
            if self.ignored(window) {
                self.detach(window.id, &params);
                continue;
            }
            if let Some(departed) = self.departed.remove(&window.id) {
                self.reinsert(window, departed, front, &params);
                continue;
            }
            if let Some(saved) = self.fullscreen.get(&window.id).map(|item| Departed {
                window: item.window.clone(),
                index: item.index,
                display: item.display.clone(),
            }) {
                let traits = self.pool.with_api(|api| api.describe(window.id));
                if traits.fullscreen {
                    continue;
                }
                self.fullscreen.remove(&window.id);
                self.reinsert(window, saved, front, &params);
                continue;
            }
            let traits = self.pool.with_api(|api| api.describe(window.id));
            if traits.role.is_empty() && traits.subrole.is_empty() {
                continue;
            }
            fresh.push((window.clone(), traits));
        }
        self.adopt_fresh(fresh, front, own_pid, &params);
        self.remove_missing(&seen, &params);
        self.focus_follows_pointer();
        self.publish_layouts(&displays, &params);
        self.settle = false;
        self.pool.poll_enhanced_ui();
        self.publish_state(&self.screens);
        self.store_metrics();
    }

    /// Wait, re-enable the event tap, and re-apply the current strips without animation.
    pub fn resync_after_wake(&mut self) {
        std::thread::sleep(Duration::from_millis(1500));
        let _ = crate::hotkeys::enable_tap();
        self.space_id = crate::private_api::current_space();
        self.settle = true;
        self.force_resolve = true;
        self.tick();
        self.settle = false;
        self.force_resolve = false;
    }

    pub fn binding_for(&self, binding: crate::bindings::Binding) -> Option<crate::bindings::ResolvedBinding> {
        crate::bindings::lookup(&self.config.bindings, &binding)
    }

    pub fn primary_display(&self) -> Option<String> {
        self.active_display()
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
        #[cfg(feature = "debug-hooks")]
        if action == "inject_gesture" {
            let fingers = args.get("fingers").and_then(|value| value.as_u64()).unwrap_or(0) as u8;
            let dx = args.get("dx").and_then(|value| value.as_f64()).unwrap_or(0.0);
            let dy = args.get("dy").and_then(|value| value.as_f64()).unwrap_or(0.0);
            let phase = args.get("phase").and_then(|value| value.as_str()).unwrap_or("");
            self.apply_swipe(fingers, dx, dy, phase)?;
            return Ok(None);
        }
        #[cfg(feature = "debug-hooks")]
        if action == "simulate_wake" {
            self.resync_after_wake();
            return Ok(None);
        }
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
            "toggle_float" => {
                self.toggle_float()?;
                return Ok(None);
            }
            "send_next_display" => {
                self.send_next_display()?;
                return Ok(None);
            }
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
            let snap = self.focus_snap(&display)?;
            if self.config.focus.mouse_follows_focus {
                if let Some(frame) = self.last_on_screen.get(&snap.window_id).copied() {
                    ax::warp_cursor(frame.x + frame.width / 2.0, frame.y + frame.height / 2.0);
                }
            }
            Ok(Some(snap))
        } else {
            Ok(None)
        }
    }

    pub fn unpark_all(&mut self) {
        self.animating = false;
        self.driver.stop();
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
            if let Some(strip) = self.strips.get(&display) {
                if let Some((index, window)) = located(strip, id) {
                    self.departed.insert(id, Departed { window, index, display: display.clone() });
                }
            }
            if let Some(strip) = self.strips.get_mut(&display) {
                let visible = Viewport { x: 0.0, y: 0.0, width: 1.0, height: 1.0 };
                strip.remove(WindowId(id), params, visible);
            }
            self.meta.remove(&id);
            self.last_on_screen.remove(&id);
            self.motions.remove(&id);
        }
    }

    fn publish_layouts(&mut self, displays: &[DisplayInfo], params: &LayoutParams) {
        let mut flying = false;
        let mut active_displays = Vec::new();
        for display in displays {
            let Some(strip) = self.strips.get(&display.uuid) else {
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
            let mut display_flying = false;
            for placement in placements {
                let id = placement.window.0;
                if matches!(placement.visibility, Visibility::Visible | Visibility::Partial) {
                    self.last_on_screen.insert(id, placement.frame);
                }
                let Some(meta) = self.meta.get(&id) else {
                    continue;
                };
                let pid = meta.pid;
                let bundle = meta.bundle_id.clone();
                let animate = !matches!(placement.visibility, Visibility::Hidden);
                if self.follow(pid, id, &bundle, placement.frame, animate) {
                    display_flying = true;
                }
            }
            if display_flying {
                flying = true;
                active_displays.push(display.cg_id);
            }
        }
        if flying {
            self.animating = true;
            for display in active_displays {
                self.driver.start(display);
            }
        }
        self.store_metrics();
    }

    fn publish_state(&self, displays: &[DisplayInfo]) {
        let body = serde_json::json!({
            "managing": self.managing,
            "displays": displays.iter().map(|display| self.display_json(display)).collect::<Vec<_>>(),
            "floating": self.floating.iter().map(|(id, info)| serde_json::json!({
                "id": id,
                "pid": info.pid,
                "bundle_id": info.bundle_id,
                "title": info.title,
            })).collect::<Vec<_>>(),
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
            "ignored": self.config.displays.ignored_displays.iter().any(|id| id == &display.uuid),
            "active_space": self.space_id,
            "strips": [{
                "space": self.space_id,
                "scroll_x": strip.map(|strip| strip.scroll_x).unwrap_or(0.0),
                "focused_column": focused,
                "pinned_left": strip.map(|strip| pin_json(self, strip.pinned_left.as_ref())).unwrap_or(serde_json::Value::Null),
                "pinned_right": strip.map(|strip| pin_json(self, strip.pinned_right.as_ref())).unwrap_or(serde_json::Value::Null),
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
            "role": meta.map(|meta| meta.role.clone()).unwrap_or_default(),
            "subrole": meta.map(|meta| meta.subrole.clone()).unwrap_or_default(),
        })
    }
}

fn pin_json(runtime: &Runtime, pin: Option<&strip_core::Pinned>) -> serde_json::Value {
    let Some(pin) = pin else {
        return serde_json::Value::Null;
    };
    serde_json::json!({
        "previous_index": pin.previous_index,
        "width_ratio": pin.column.width_ratio,
        "windows": pin.column.windows.iter().map(|window| runtime.window_json(window)).collect::<Vec<_>>(),
    })
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

enum Follow {
    Idle,
    Flying,
    Write(Frame),
    Launch { sized: Option<Frame>, current: Frame },
}

impl Runtime {
    fn follow(&mut self, pid: i32, id: u64, bundle: &str, target: Frame, animate: bool) -> bool {
        if !self.motions.contains_key(&id) {
            self.pool.publish(pid, id, target, bundle);
            self.motions.insert(id, Motion::rest(pid, bundle, target));
            return false;
        }
        let duration = f64::from(self.config.animation.animation_duration_ms) / 1000.0;
        let action = {
            let motion = self.motions.get_mut(&id).expect("motion");
            motion.pid = pid;
            motion.bundle_id = bundle.to_string();
            if self.settle {
                motion.snap(target);
                Follow::Write(target)
            } else {
            let size_delta = (motion.width - target.width).abs().max((motion.height - target.height).abs());
            let pos_delta = (motion.x.target - target.x).abs().max((motion.y.target - target.y).abs());
            if !animate || duration <= 0.0 {
                if size_delta > 0.5 || pos_delta > 0.5 || !motion.settled() {
                    motion.snap(target);
                    Follow::Write(target)
                } else {
                    Follow::Idle
                }
            } else if pos_delta <= 0.5 && size_delta <= 0.5 {
                if motion.settled() { Follow::Idle } else { Follow::Flying }
            } else if pos_delta <= 0.5 && motion.settled() {
                motion.snap(target);
                Follow::Write(target)
            } else {
                let grew = target.width * target.height > motion.width * motion.height + 1.0;
                let shrank = target.width * target.height + 1.0 < motion.width * motion.height;
                let sized = if size_delta > 0.5 && grew {
                    let frame = Frame {
                        x: motion.x.x,
                        y: motion.y.x,
                        width: target.width,
                        height: target.height,
                    };
                    motion.width = target.width;
                    motion.height = target.height;
                    Some(frame)
                } else {
                    None
                };
                if shrank {
                    motion.pending_shrink = Some((target.width, target.height));
                } else if size_delta > 0.5 && !grew {
                    motion.width = target.width;
                    motion.height = target.height;
                }
                if pos_delta > 0.5 {
                    motion.x.retarget(target.x);
                    motion.y.retarget(target.y);
                }
                Follow::Launch { sized, current: motion.rendered() }
            }
            }
        };
        match action {
            Follow::Idle => false,
            Follow::Flying => true,
            Follow::Write(frame) => {
                self.pool.publish(pid, id, frame, bundle);
                false
            }
            Follow::Launch { sized, current } => {
                if let Some(frame) = sized {
                    self.pool.publish(pid, id, frame, bundle);
                }
                self.pool.publish_position(pid, id, current, bundle);
                true
            }
        }
    }

    fn step_animation(&mut self, dt: f64) {
        if !self.animating {
            return;
        }
        let zeta = self.config.animation.animation_damping;
        let omega = Spring::omega(f64::from(self.config.animation.animation_duration_ms) / 1000.0);
        let ids: Vec<u64> = self.motions.keys().copied().collect();
        let mut still = false;
        let mut shrinks = Vec::new();
        for id in ids {
            let step = {
                let Some(motion) = self.motions.get_mut(&id) else {
                    continue;
                };
                if motion.settled() {
                    continue;
                }
                let x_done = motion.x.step(dt, zeta, omega);
                let y_done = motion.y.step(dt, zeta, omega);
                let done = x_done && y_done;
                let frame = motion.rendered();
                let shrink = if done { motion.pending_shrink.take() } else { None };
                if let Some((width, height)) = shrink {
                    motion.width = width;
                    motion.height = height;
                }
                (motion.pid, motion.bundle_id.clone(), frame, done, shrink)
            };
            let (pid, bundle, frame, done, shrink) = step;
            self.pool.publish_position(pid, id, frame, &bundle);
            if let Some((width, height)) = shrink {
                shrinks.push((pid, id, bundle, Frame { x: frame.x, y: frame.y, width, height }));
            }
            if !done {
                still = true;
            }
        }
        if !still {
            self.animating = false;
            self.driver.stop();
            for (pid, id, bundle, frame) in shrinks {
                self.pool.publish(pid, id, frame, &bundle);
            }
        }
        self.store_metrics();
    }

    fn store_metrics(&self) {
        let mut metrics = self.pool.metrics();
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
        let pacing = self.driver.snapshot();
        metrics.frame_interval_p95_ms = pacing.frame_interval_p95_ms;
        metrics.refresh_interval_ms = pacing.refresh_interval_ms;
        metrics.display_link_running = pacing.running;
        let body = serde_json::json!({
            "ax_write_ms": ax_write_ms,
            "frames_per_second": metrics.frames_per_second,
            "last_frame_ms": metrics.last_frame_ms,
            "idle_cpu": metrics.idle_cpu,
            "size_writes_during_animation": metrics.size_writes_during_animation,
            "display_link_running": metrics.display_link_running,
            "frame_interval_p95_ms": metrics.frame_interval_p95_ms,
            "refresh_interval_ms": metrics.refresh_interval_ms,
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

impl Runtime {
    pub fn note_pointer(&mut self, kind: u32, x: f64, y: f64) {
        self.pointer_x = x;
        self.pointer_y = y;
        self.pointer_at = Instant::now();
        self.pointer_seen = true;
        if !self.config.focus.drag_to_reorder {
            return;
        }
        if kind == 1 {
            if let Some((id, display, column, frame)) = self.window_under(x, y) {
                if y <= frame.y + 28.0 {
                    self.drag = Some(Drag { id, display, column });
                }
            }
            return;
        }
        if kind != 2 {
            return;
        }
        let Some(drag) = self.drag.take() else {
            return;
        };
        let params = layout_params(&self.config);
        let Some(target) = self.screen_at(x, y) else {
            return;
        };
        if target != drag.display {
            self.move_window_to_display(drag.id, &drag.display, &target, &params);
            return;
        }
        let frames = self.column_spans(&drag.display);
        let Some(dest) = restore::drop_index(&frames, x) else {
            return;
        };
        let visible = self.viewport_of(&drag.display);
        if let Some(strip) = self.strips.get_mut(&drag.display) {
            let to = dest.min(strip.columns.len().saturating_sub(1));
            strip.move_column(drag.column, to);
            strip.reveal(&params, visible);
        }
        let screens = self.screens.clone();
        self.publish_layouts(&screens, &params);
        self.publish_state(&screens);
    }

    fn toggle_float(&mut self) -> Result<(), CommandError> {
        if let Some(id) = self.args_window_floating() {
            self.unfloat(id)?;
            return Ok(());
        }
        let display = self.active_display().ok_or(CommandError::NoFocusedWindow)?;
        let params = layout_params(&self.config);
        let visible = self.viewport_of(&display);
        let window = {
            let strip = self.strips.get(&display).ok_or(CommandError::NoFocusedWindow)?;
            focused_window(strip).cloned().ok_or(CommandError::NoFocusedWindow)?
        };
        let id = window.id.0;
        let meta = self.meta.get(&id).cloned();
        if let Some(strip) = self.strips.get_mut(&display) {
            strip.remove(WindowId(id), &params, visible);
        }
        self.floating.insert(
            id,
            FloatingWindow {
                title: meta.as_ref().map(|meta| meta.title.clone()).unwrap_or_default(),
                pid: window.pid,
                bundle_id: meta.map(|meta| meta.bundle_id).unwrap_or_default(),
            },
        );
        self.meta.remove(&id);
        self.motions.remove(&id);
        let screens = self.screens.clone();
        self.publish_layouts(&screens, &params);
        self.publish_state(&screens);
        Ok(())
    }

    fn send_next_display(&mut self) -> Result<(), CommandError> {
        if self.screens.len() < 2 {
            eprintln!("multi-display skipped");
            return Ok(());
        }
        let from = self.active_display().ok_or(CommandError::NoFocusedWindow)?;
        let index = self.screens.iter().position(|screen| screen.uuid == from).unwrap_or(0);
        let target = self.screens[(index + 1) % self.screens.len()].uuid.clone();
        if self.config.displays.ignored_displays.iter().any(|id| id == &target || id == &from) {
            eprintln!("multi-display skipped");
            return Ok(());
        }
        let params = layout_params(&self.config);
        self.move_window_to_display_focused(&from, &target, &params)?;
        Ok(())
    }

    fn args_window_floating(&self) -> Option<u64> {
        None
    }

    fn unfloat(&mut self, id: u64) -> Result<(), CommandError> {
        let info = self.floating.remove(&id).ok_or(CommandError::NoFocusedWindow)?;
        let display = self.active_display().ok_or(CommandError::NoFocusedWindow)?;
        let params = layout_params(&self.config);
        let visible = self.viewport_of(&display);
        let window = Window::new(id, info.pid);
        let strip = self.strips.entry(display.clone()).or_default();
        strip.insert(window, info.pid.wrapping_add(1), None, &params, visible);
        self.meta.insert(
            id,
            Meta {
                pid: info.pid,
                bundle_id: info.bundle_id.clone(),
                title: info.title.clone(),
                owner: info.bundle_id.clone(),
                display,
                role: String::new(),
                subrole: String::new(),
            },
        );
        let screens = self.screens.clone();
        self.publish_layouts(&screens, &params);
        self.publish_state(&screens);
        Ok(())
    }

    fn move_window_to_display_focused(&mut self, from: &str, target: &str, params: &LayoutParams) -> Result<(), CommandError> {
        let visible_from = self.viewport_of(from);
        let window = {
            let strip = self.strips.get(from).ok_or(CommandError::NoFocusedWindow)?;
            focused_window(strip).cloned().ok_or(CommandError::NoFocusedWindow)?
        };
        let id = window.id.0;
        if let Some(strip) = self.strips.get_mut(from) {
            strip.remove(WindowId(id), params, visible_from);
        }
        let visible = self.viewport_of(target);
        let pid = window.pid;
        let strip = self.strips.entry(target.to_string()).or_default();
        strip.insert(window, pid, None, params, visible);
        if let Some(meta) = self.meta.get_mut(&id) {
            meta.display = target.to_string();
        }
        let screens = self.screens.clone();
        self.publish_layouts(&screens, params);
        self.publish_state(&screens);
        Ok(())
    }

    fn move_window_to_display(&mut self, id: u64, from: &str, target: &str, params: &LayoutParams) {
        let Some(window) = self.strips.get(from).and_then(|strip| located(strip, id)).map(|(_, window)| window) else {
            return;
        };
        let visible_from = self.viewport_of(from);
        if let Some(strip) = self.strips.get_mut(from) {
            strip.remove(WindowId(id), params, visible_from);
        }
        let visible = self.viewport_of(target);
        let pid = window.pid;
        let strip = self.strips.entry(target.to_string()).or_default();
        strip.insert(window, pid, None, params, visible);
        if let Some(meta) = self.meta.get_mut(&id) {
            meta.display = target.to_string();
        }
        let screens = self.screens.clone();
        self.publish_layouts(&screens, params);
        self.publish_state(&screens);
    }

    fn restore_arrived(&mut self, listed: &[ListedWindow]) {
        let Some(saved) = self.saved.clone() else {
            return;
        };
        let live: Vec<restore::LiveWindow> = listed
            .iter()
            .map(|window| restore::LiveWindow {
                id: window.id,
                pid: window.pid,
                bundle_id: config::shown_bundle(&ax::bundle_id(window.pid), &window.owner),
                title: window.title.clone(),
                role: String::new(),
                subrole: String::new(),
            })
            .collect();
        for (saved_index, live_index) in restore::match_windows(&saved.windows, &live) {
            let saved_window = &saved.windows[saved_index];
            let window = &listed[live_index];
            if self.contains_id(window.id) {
                continue;
            }
            self.place_restored(saved_window, window);
        }
    }

    fn place_restored(&mut self, saved: &restore::SavedWindow, live: &ListedWindow) {
        let key = (saved.display.clone(), saved.column);
        let mut model = Window::new(live.id, live.pid);
        model.height_ratio = saved.height_ratio;
        model.original_height = live.bounds.height;
        let siblings = self.restored_members.get(&key).cloned().unwrap_or_default();
        let appended = if siblings.is_empty() {
            None
        } else {
            self.strips.get(&saved.display).and_then(|strip| {
                strip.columns.iter().position(|column| column.windows.iter().any(|window| siblings.contains(&window.id.0)))
            })
        };
        let display = saved.display.clone();
        let role = saved.role.clone();
        let subrole = saved.subrole.clone();
        if let Some(index) = appended {
            if let Some(strip) = self.strips.get_mut(&display) {
                if let Some(column) = strip.columns.get_mut(index) {
                    column.windows.push(model);
                }
            }
        } else {
            let scroll = self.saved.as_ref().and_then(|session| {
                session.scroll.iter().find(|(uuid, _)| uuid == &display).map(|(_, scroll)| *scroll)
            });
            let focus = self.saved.as_ref().and_then(|session| {
                session.focus.iter().find(|(uuid, _)| uuid == &display).map(|(_, index)| *index)
            });
            let apply_scroll = !self.scroll_applied.contains(&display);
            {
                let strip = self.strips.entry(display.clone()).or_default();
                strip.insert_at(model, saved.column, saved.width_ratio);
                if apply_scroll {
                    if let Some(scroll) = scroll {
                        strip.scroll_x = scroll;
                    }
                    if strip.focus == Focus::None && !strip.columns.is_empty() {
                        let index = usize::try_from(focus.unwrap_or(0).max(0)).unwrap_or(0);
                        strip.focus = Focus::Column(index.min(strip.columns.len().saturating_sub(1)));
                    }
                }
            }
            if apply_scroll {
                self.scroll_applied.insert(display.clone());
            }
        }
        self.restored_members.entry(key).or_default().push(live.id);
        self.remember(live, &display, &role, &subrole);
    }

    fn adopt_fresh(&mut self, fresh: Vec<(ListedWindow, Traits)>, front: i32, own_pid: i32, params: &LayoutParams) {
        let mut absorbed = vec![false; fresh.len()];
        for index in 0..fresh.len() {
            if absorbed[index] {
                continue;
            }
            let (window, traits) = &fresh[index];
            let decision = self.decide(window, traits, own_pid);
            match decision {
                Placement::Float => self.remember_float(window),
                Placement::Fullscreen => {
                    self.fullscreen.insert(
                        window.id,
                        Departed { window: model_from(window), index: 0, display: String::new() },
                    );
                }
                Placement::Manage { slot } => {
                    let display = self.insert_managed(window, front, slot, params, traits);
                    for (peer_index, (peer, peer_traits)) in fresh.iter().enumerate().skip(index + 1) {
                        if absorbed[peer_index] || peer.pid != window.pid || !restore::shares_native_tab(window.bounds, peer.bounds) {
                            continue;
                        }
                        if matches!(self.decide(peer, peer_traits, own_pid), Placement::Manage { .. }) {
                            self.push_tab(&display, window.id, peer, peer_traits);
                            absorbed[peer_index] = true;
                        }
                    }
                }
            }
        }
    }

    fn decide(&self, window: &ListedWindow, traits: &Traits, own_pid: i32) -> Placement {
        let bundle = config::shown_bundle(&ax::bundle_id(window.pid), &window.owner);
        config::admit(
            &self.config.rules,
            AdmitWindow {
                bundle_id: &bundle,
                title: &window.title,
                role: &traits.role,
                subrole: &traits.subrole,
                size_settable: traits.size_settable,
                fixed_size: traits.fixed_size,
                fullscreen: traits.fullscreen,
                own_process: window.pid == own_pid,
            },
        )
    }

    fn insert_managed(&mut self, window: &ListedWindow, front: i32, slot: Option<u32>, params: &LayoutParams, traits: &Traits) -> String {
        let Some(display) = display_for(&self.screens, window.bounds) else {
            return String::new();
        };
        let uuid = display.uuid.clone();
        let visible = viewport(&display.visible);
        let role = traits.role.clone();
        let subrole = traits.subrole.clone();
        {
            let strip = self.strips.entry(uuid.clone()).or_default();
            strip.insert(model_from(window), front, slot, params, visible);
        }
        self.remember(window, &uuid, &role, &subrole);
        uuid
    }

    fn push_tab(&mut self, display: &str, leader: u64, peer: &ListedWindow, traits: &Traits) {
        let role = traits.role.clone();
        let subrole = traits.subrole.clone();
        let pushed = self.strips.get_mut(display).and_then(|strip| {
            let column = strip.columns.iter_mut().find(|column| column.windows.iter().any(|window| window.id.0 == leader))?;
            column.mode = DisplayMode::Tabbed;
            column.windows.push(model_from(peer));
            Some(())
        }).is_some();
        if pushed {
            self.remember(peer, display, &role, &subrole);
        }
    }

    fn remember_float(&mut self, window: &ListedWindow) {
        let bundle = config::shown_bundle(&ax::bundle_id(window.pid), &window.owner);
        eprintln!("auto-float title={} subrole-pending bundle={bundle}", window.title);
        self.floating.insert(
            window.id,
            FloatingWindow { title: window.title.clone(), pid: window.pid, bundle_id: bundle },
        );
    }

    fn reinsert(&mut self, window: &ListedWindow, departed: Departed, front: i32, params: &LayoutParams) {
        let Some(display) = display_for(&self.screens, window.bounds).map(|display| display.uuid.clone()).or_else(|| {
            if departed.display.is_empty() { None } else { Some(departed.display.clone()) }
        }) else {
            return;
        };
        let visible = self.viewport_of(&display);
        let slot = u32::try_from(departed.index.saturating_add(1)).unwrap_or(1);
        let mut model = departed.window;
        model.id = WindowId(window.id);
        model.pid = window.pid;
        let strip = self.strips.entry(display.clone()).or_default();
        strip.insert(model, front, Some(slot), params, visible);
        self.remember(window, &display, "", "");
    }

    fn remember(&mut self, window: &ListedWindow, display: &str, role: &str, subrole: &str) {
        let bundle = config::shown_bundle(&ax::bundle_id(window.pid), &window.owner);
        self.meta.insert(
            window.id,
            Meta {
                pid: window.pid,
                bundle_id: bundle,
                title: window.title.clone(),
                owner: window.owner.clone(),
                display: display.to_string(),
                role: role.to_string(),
                subrole: subrole.to_string(),
            },
        );
    }

    fn contains_id(&self, id: u64) -> bool {
        self.strips.values().any(|strip| strip.contains(WindowId(id)))
    }

    fn ignored(&self, window: &ListedWindow) -> bool {
        let Some(display) = display_for(&self.screens, window.bounds) else {
            return false;
        };
        self.config.displays.ignored_displays.iter().any(|id| id == &display.uuid)
    }

    fn detach(&mut self, id: u64, params: &LayoutParams) {
        let Some(display) = self.meta.get(&id).map(|meta| meta.display.clone()) else {
            return;
        };
        if let Some(strip) = self.strips.get_mut(&display) {
            strip.remove(WindowId(id), params, Viewport { x: 0.0, y: 0.0, width: 1.0, height: 1.0 });
        }
        self.meta.remove(&id);
        self.motions.remove(&id);
    }

    fn focus_follows_pointer(&mut self) {
        if !self.config.focus.focus_follows_mouse || !self.pointer_seen || self.pointer_at.elapsed() < Duration::from_millis(100) {
            return;
        }
        let Some((id, display, column, _)) = self.window_under(self.pointer_x, self.pointer_y) else {
            return;
        };
        let Some(strip) = self.strips.get_mut(&display) else {
            return;
        };
        if strip.focus == Focus::Column(column) {
            return;
        }
        let _ = id;
        strip.focus = Focus::Column(column);
        let screens = self.screens.clone();
        self.publish_state(&screens);
    }

    fn window_under(&self, x: f64, y: f64) -> Option<(u64, String, usize, Frame)> {
        for (uuid, strip) in &self.strips {
            for (index, column) in strip.columns.iter().enumerate() {
                for window in &column.windows {
                    let Some(frame) = self.last_on_screen.get(&window.id.0).copied() else {
                        continue;
                    };
                    if x >= frame.x && x < frame.right() && y >= frame.y && y < frame.bottom() {
                        return Some((window.id.0, uuid.clone(), index, frame));
                    }
                }
            }
        }
        None
    }

    fn screen_at(&self, x: f64, y: f64) -> Option<String> {
        self.screens.iter().find(|screen| x >= screen.frame.x && x < screen.frame.right() && y >= screen.frame.y && y < screen.frame.bottom()).map(|screen| screen.uuid.clone())
    }

    fn column_spans(&self, display: &str) -> Vec<(f64, f64)> {
        let Some(strip) = self.strips.get(display) else {
            return Vec::new();
        };
        let mut spans = Vec::new();
        for column in &strip.columns {
            let frame = column.windows.first().and_then(|window| self.last_on_screen.get(&window.id.0)).copied();
            if let Some(frame) = frame {
                spans.push((frame.x, frame.width));
            }
        }
        spans
    }
}

fn focused_window(strip: &Strip) -> Option<&Window> {
    let column = match strip.focus {
        Focus::Column(index) => strip.columns.get(index)?,
        Focus::PinnedLeft => &strip.pinned_left.as_ref()?.column,
        Focus::PinnedRight => &strip.pinned_right.as_ref()?.column,
        Focus::None => return None,
    };
    column.windows.get(column.focused_window).or_else(|| column.windows.first())
}

fn located(strip: &Strip, id: u64) -> Option<(usize, Window)> {
    for (index, column) in strip.columns.iter().enumerate() {
        if let Some(window) = column.windows.iter().find(|window| window.id.0 == id) {
            return Some((index, window.clone()));
        }
    }
    if let Some(window) = strip.pinned_left.as_ref().and_then(|pin| pin.column.windows.iter().find(|window| window.id.0 == id)) {
        return Some((pin_index(strip.pinned_left.as_ref()), window.clone()));
    }
    if let Some(window) = strip.pinned_right.as_ref().and_then(|pin| pin.column.windows.iter().find(|window| window.id.0 == id)) {
        return Some((pin_index(strip.pinned_right.as_ref()), window.clone()));
    }
    None
}

fn pin_index(pin: Option<&strip_core::Pinned>) -> usize {
    pin.map(|pin| pin.previous_index).unwrap_or(0)
}

fn model_from(window: &ListedWindow) -> Window {
    let mut model = Window::new(window.id, window.pid);
    model.original_height = window.bounds.height;
    model
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


