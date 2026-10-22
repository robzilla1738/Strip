//! One strip per display and Space. Publishes frames through the writer pool.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use strip_core::{
    column_spans, column_width_in, layout, CenterMode, Corner, DisplayFrame, DisplayMode, Focus, Frame, Gesture, LayoutParams, Spring, Strip, SwipeView, Viewport, Visibility, Window, WindowId,
};

use crate::ax::{self, AxApi, DisplayInfo, ListedWindow, Traits};
use crate::config::{self, AdmitWindow, Config, Placement};
use crate::display_link::DisplayDriver;
use crate::parking::{relocated_repair, repair_relocated, restored_frame};
use crate::private_api::PrivateApi;
use crate::restore::{self, SavedSession};
use crate::window_api::WindowApi;
use crate::writer::{Clock, SystemClock, WriterPool};

/// Active strip identity. SkyLight uses the live Space id. Without it the id
/// stays `0` until a space switch, then [`crate::private_api::fallback_space_key`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct StripKey {
    display: String,
    space: u64,
}

pub struct Runtime {
    config: Config,
    pool: WriterPool<AxApi>,
    strips: HashMap<StripKey, Strip>,
    meta: HashMap<u64, Meta>,
    last_on_screen: HashMap<u64, Frame>,
    /// Windows whose latest layout is on screen. Parked and inactive tabs are absent.
    shown: HashSet<u64>,
    state: Arc<Mutex<String>>,
    metrics_body: Arc<Mutex<String>>,
    only_owner: Option<String>,
    managing: bool,
    ax_trusted: bool,
    screens: Vec<DisplayInfo>,
    config_path: Option<PathBuf>,
    motions: HashMap<u64, Motion>,
    /// Rendered scroll per strip. `Strip::scroll_x` is where it is heading.
    views: HashMap<StripKey, Spring>,
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
    skylight: bool,
    /// True after a space switch when SkyLight is missing. Keeps the fallback hash stable.
    space_switched: bool,
    space_id: u64,
    park_repaired: HashSet<u64>,
    extra_left: HashSet<String>,
    extra_right: HashSet<String>,
    /// Resume point for `_AXUIElementCreateWithRemoteToken` scans.
    remote_cursors: HashMap<u64, u64>,
    pointer_x: f64,
    pointer_y: f64,
    pointer_at: Instant,
    pointer_seen: bool,
    drag: Option<Drag>,
    /// Bumps when the published state text changes.
    state_version: Cell<u64>,
    /// Last metrics snapshot; throttles `store_metrics` off the frame path.
    metrics_at: Cell<Option<Instant>>,
    emitted_version: Cell<u64>,
    /// True while a swipe moves the strip: windows follow the fingers without a spring.
    tracking: bool,
    swipe_at: Option<Instant>,
    /// True after a swipe went quiet and was ended here; the next finger movement resumes it.
    swipe_expired: bool,
    swipe_state_at: Option<Instant>,
    /// Consecutive ticks a window showed a size other than its target, and that size.
    size_misses: HashMap<u64, (u8, (f64, f64))>,
    /// Windows whose AX element could not be resolved, and when to try again.
    resolve_after: RefCell<HashMap<u64, Instant>>,
    /// True when a listed window had transient empty/`AXUnknown` traits and needs a quick retry.
    pending_traits: bool,
}

/// An unresolvable window (no AX element) is retried at most this often.
const RESOLVE_RETRY: Duration = Duration::from_secs(2);
/// A swipe with no gesture event for this long is ended. macOS stops sending
/// events when it takes a gesture over, for example to switch Spaces.
const SWIPE_IDLE: Duration = Duration::from_millis(300);
/// Caps the release velocity handed to the snap spring, in points per second.
const MAX_FLING: f64 = 9000.0;

struct Motion {
    x: Spring,
    y: Spring,
    width: f64,
    height: f64,
    pid: i32,
    bundle_id: String,
    pending_shrink: Option<(f64, f64)>,
    /// The strip or pin the motion's coordinates are relative to.
    anchor: Option<Anchor>,
    offset: f64,
    /// The parked frame while the window is off screen.
    park: Option<Frame>,
    /// Last screen position sent to the writer.
    written: Option<(f64, f64)>,
    /// A deferred smaller size that is due now the move has ended.
    size_due: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct Anchor {
    strip: StripKey,
    pinned: bool,
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
            anchor: None,
            offset: 0.0,
            park: None,
            written: None,
            size_due: false,
        }
    }

    fn settled(&self) -> bool {
        (self.x.x - self.x.target).abs() < 0.5 && self.x.v.abs() < 5.0 && (self.y.x - self.y.target).abs() < 0.5 && self.y.v.abs() < 5.0
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
    space: u64,
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
    x: f64,
    y: f64,
    /// Pressed on the title bar, so a move is a reorder.
    title_bar: bool,
    /// Pressed near a window edge, so a drag can be a hand resize.
    on_edge: bool,
}

/// Mouse travel below this between title-bar press and release is a click, not a drag.
const DRAG_SLOP: f64 = 6.0;
/// Resize handles sit a few points outside the window frame.
const GRAB_MARGIN: f64 = 6.0;
const SWIPE_STATE_EVERY: Duration = Duration::from_millis(33);

pub struct SwipeNotice {
    pub started: bool,
    pub updated: bool,
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
        Self::build(config, config_path, state_path, private_api, state, metrics_body, ax_trusted)
    }

    fn build(
        config: Config,
        config_path: Option<PathBuf>,
        state_path: Option<PathBuf>,
        private_api: &PrivateApi,
        state: Arc<Mutex<String>>,
        metrics_body: Arc<Mutex<String>>,
        ax_trusted: bool,
    ) -> Self {
        let only_owner = std::env::var("STRIP_ONLY_OWNER").ok().filter(|value| !value.is_empty());
        let saved = state_path.as_ref().map(|path| restore::load_session(path)).filter(|session| !session.windows.is_empty());
        let now = Instant::now();
        Self {
            config,
            pool: WriterPool::new(AxApi::new(private_api), Arc::new(SystemClock) as Arc<dyn Clock>),
            strips: HashMap::new(),
            meta: HashMap::new(),
            last_on_screen: HashMap::new(),
            shown: HashSet::new(),
            state,
            metrics_body,
            only_owner,
            managing: true,
            ax_trusted,
            screens: Vec::new(),
            config_path,
            motions: HashMap::new(),
            views: HashMap::new(),
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
            skylight: private_api.skylight,
            space_switched: false,
            space_id: crate::private_api::current_space(),
            park_repaired: HashSet::new(),
            extra_left: HashSet::new(),
            extra_right: HashSet::new(),
            remote_cursors: HashMap::new(),
            pointer_x: 0.0,
            pointer_y: 0.0,
            pointer_at: now,
            pointer_seen: false,
            drag: None,
            state_version: Cell::new(0),
            metrics_at: Cell::new(None),
            emitted_version: Cell::new(0),
            tracking: false,
            swipe_at: None,
            swipe_expired: false,
            swipe_state_at: None,
            size_misses: HashMap::new(),
            resolve_after: RefCell::new(HashMap::new()),
            pending_traits: false,
        }
    }

    /// Test runtime. Does not call `prepare_app` and does not adopt the user's windows.
    #[cfg(test)]
    fn blank(skylight: bool) -> Self {
        let mut api = PrivateApi::live();
        api.skylight = skylight;
        let mut runtime = Self::build(
            Config::default(),
            None,
            None,
            &api,
            Arc::new(Mutex::new(String::new())),
            Arc::new(Mutex::new(String::new())),
            true,
        );
        runtime.only_owner = Some("__strip_test_none__".into());
        runtime
    }

    /// `started` is the frame a horizontal swipe first arms. `updated` means `scroll_x` changed.
    pub fn drive_swipe(&mut self, fingers: u8, dx: f64, dy: f64, phase: &str) -> SwipeNotice {
        let before = self.gesture.is_armed();
        let updated = self.apply_swipe(fingers, dx, dy, phase).unwrap_or(false);
        SwipeNotice {
            started: !before && self.gesture.is_armed(),
            updated,
        }
    }

    fn apply_swipe(&mut self, fingers: u8, dx: f64, dy: f64, phase: &str) -> Result<bool, CommandError> {
        let swipe_started = Instant::now();
        let result = self.apply_swipe_inner(fingers, dx, dy, phase);
        let slow_ms = swipe_started.elapsed().as_secs_f64() * 1000.0;
        if slow_ms > 10.0 && std::env::var_os("STRIP_SLOW_LOG").is_some() {
            eprintln!("slow-swipe phase={phase} ms={slow_ms:.1}");
        }
        result
    }

    fn apply_swipe_inner(&mut self, fingers: u8, dx: f64, dy: f64, phase: &str) -> Result<bool, CommandError> {
        let display = self.swipe_display().ok_or(CommandError::NoFocusedWindow)?;
        let visible = self.viewport_of(&display);
        let params = layout_params(&self.config);
        let view_width = self.strip(&display).map_or(visible.width, |strip| strip_core::scroll_width(strip, &params, visible));
        let ratios: Vec<f64> = self
            .strip(&display)
            .map(|strip| strip.columns.iter().map(|column| column_width_in(column, view_width, params.gap_inner) / visible.width.max(1.0)).collect())
            .unwrap_or_default();
        let spans = self
            .strip(&display)
            .map(|strip| column_spans(strip, &params, visible))
            .unwrap_or_default();
        let scroll = self.strip(&display).map(|strip| strip.scroll_x).unwrap_or(0.0);
        let view = SwipeView::from_ratios(
            &ratios,
            visible.width,
            self.config.layout.gap_inner,
            self.config.swipe.swipe_sensitivity,
            self.config.swipe.swipe_natural,
            self.config.swipe.swipe_deceleration,
        )
        .with_spans(spans);
        let content = view.spans.last().map_or(0.0, |(prefix, width)| prefix + width);
        let center = match params.center {
            CenterMode::Always => true,
            CenterMode::OnOverflow => content > view_width + 1e-6,
            CenterMode::Never => false,
        };
        let view = view.with_view(view_width, center);
        let now = Instant::now();
        let update = match phase {
            "began" => {
                self.swipe_expired = false;
                self.gesture.begin(fingers, self.config.swipe.swipe_fingers, scroll);
                self.swipe_at = Some(now);
                strip_core::SwipeUpdate::default()
            }
            "moved" => {
                if self.swipe_expired && !self.gesture.is_open() {
                    self.gesture.begin(fingers, self.config.swipe.swipe_fingers, scroll);
                    self.swipe_at = Some(now);
                }
                // Velocity uses the real event spacing; trackpads report at 90–120 Hz.
                let dt = self.swipe_at.map(|then| now.duration_since(then).as_secs_f64()).unwrap_or(1.0 / 60.0).clamp(1.0 / 240.0, 1.0 / 20.0);
                self.swipe_at = Some(now);
                self.gesture.moved(dx, dy, &view, dt)
            }
            "ended" => {
                // Fingers that rested before lifting carry no momentum.
                if self.swipe_at.is_some_and(|then| now.duration_since(then) > Duration::from_millis(80)) {
                    self.gesture.rest();
                }
                self.swipe_at = None;
                self.gesture.ended(&view)
            }
            _ => return Err(CommandError::BadRequest("phase must be began, moved, or ended".to_string())),
        };
        let Some(scroll) = update.scroll else {
            return Ok(false);
        };
        let release = phase == "ended";
        // Sub-pixel finger noise is not worth a full layout pass; the gesture
        // position is absolute, so the next real move still lands exactly.
        if !release && (scroll - self.strip(&display).map_or(scroll, |strip| strip.scroll_x)).abs() < 0.35 {
            return Ok(false);
        }
        if let Some(strip) = self.strip_mut(&display) {
            strip.scroll_x = scroll;
            if let Some(index) = update.focus {
                if index < strip.columns.len() {
                    strip.focus = Focus::Column(index);
                }
            }
        }
        let screens = self.screens.clone();
        self.tracking = !release;
        if release {
            self.hand_off_fling(&display, self.gesture.velocity(), scroll);
        }
        self.publish_layouts(&screens, &params);
        self.tracking = false;
        if release {
            if let Ok(snap) = self.focus_snap(&display) {
                self.raise_window(snap.window_id);
            }
        }
        // Trackpads report at up to 120 Hz; the overlays only need the state at
        // a modest rate while the fingers move, and always on release.
        if release || self.swipe_state_at.is_none_or(|then| now.duration_since(then) >= SWIPE_STATE_EVERY) {
            self.swipe_state_at = Some(now);
            self.publish_state(&screens);
        }
        Ok(true)
    }

    /// Seeds the strip's scroll spring with the finger velocity so the strip
    /// keeps its momentum after lift instead of stopping and easing out. The
    /// snap target already accounts for the fling distance, so velocity that
    /// points away from (or nowhere near) the target is dropped, not bounced.
    fn hand_off_fling(&mut self, display: &str, scroll_velocity: f64, target: f64) {
        let velocity = scroll_velocity.clamp(-MAX_FLING, MAX_FLING);
        if !velocity.is_finite() || velocity.abs() < 1.0 {
            return;
        }
        let omega = Spring::omega(f64::from(self.config.animation.animation_duration_ms) / 1000.0);
        let key = self.key_of(display);
        if let Some(view) = self.views.get_mut(&key) {
            let displacement = target - view.x;
            if displacement.abs() < 1.0 || displacement * velocity <= 0.0 {
                return;
            }
            let cap = if omega > 0.0 {
                (omega * displacement.abs()).min(MAX_FLING)
            } else {
                MAX_FLING
            };
            view.v = velocity.clamp(-cap, cap);
        }
    }

    /// Accessibility granted after launch: commands start working without a restart.
    pub fn set_trusted(&mut self, trusted: bool) {
        self.ax_trusted = trusted;
    }

    pub fn swipe_armed(&self) -> bool {
        self.gesture.is_armed() && self.gesture.is_open()
    }

    /// When an open swipe should be ended for lack of events.
    pub fn swipe_deadline(&self) -> Option<Instant> {
        self.swipe_at.filter(|_| self.gesture.is_open()).map(|then| then + SWIPE_IDLE)
    }

    pub fn expire_swipe(&mut self) {
        if self.swipe_deadline().is_some_and(|due| Instant::now() >= due) {
            let _ = self.apply_swipe(0, 0.0, 0.0, "ended");
            self.swipe_expired = true;
        }
    }

    pub fn is_animating(&self) -> bool {
        self.animating
    }

    pub fn has_pending_windows(&self) -> bool {
        self.pending_traits
    }

    /// Step due display-link frames. Returns true when a frame was published.
    pub fn drain_frames(&mut self) -> bool {
        let dts = self.driver.take_dts();
        if dts.is_empty() {
            return false;
        }
        for dt in dts {
            self.integrate_springs(dt);
        }
        let screens = self.screens.clone();
        let params = layout_params(&self.config);
        self.publish_layouts(&screens, &params);
        self.publish_state(&self.screens);
        true
    }

    /// The published state text when it changed since the last call.
    pub fn take_state_change(&self) -> Option<String> {
        let version = self.state_version.get();
        if version == self.emitted_version.get() {
            return None;
        }
        self.emitted_version.set(version);
        Some(self.state.lock().unwrap_or_else(|err| err.into_inner()).clone())
    }

    pub fn state_version(&self) -> u64 {
        self.state_version.get()
    }

    pub fn snapshot(&self) -> serde_json::Value {
        let text = self.state.lock().unwrap_or_else(|err| err.into_inner()).clone();
        serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({}))
    }

    pub fn tick(&mut self) {
        let tick_started = Instant::now();
        self.tick_inner();
        let tick_ms = tick_started.elapsed().as_secs_f64() * 1000.0;
        if tick_ms > 20.0 && std::env::var_os("STRIP_SLOW_LOG").is_some() {
            eprintln!("slow-tick ms={tick_ms:.1}");
        }
    }

    fn tick_inner(&mut self) {
        if !self.managing {
            self.pending_traits = false;
            self.publish_state(&[]);
            return;
        }
        self.pending_traits = false;
        let displays = ax::displays(self.config.layout.dock_aware);
        self.screens = displays.clone();
        if self.skylight {
            self.space_id = crate::private_api::current_space().max(1);
        }
        let mut listed = self.eligible(&ax::on_screen_windows());
        let active = self.active_space();
        for window in &mut listed {
            window.space = crate::private_api::assign_strip_space(self.skylight, active, None, true);
        }
        listed.sort_by_key(|window| window.id);
        self.resolve_listed(&listed);
        let mut seen: HashSet<u64> = listed.iter().map(|window| window.id).collect();
        self.resolve_after.borrow_mut().retain(|id, _| seen.contains(id));
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
                let traits = self.pool.with_api_ref(|api| api.describe(window.id));
                if traits.fullscreen {
                    continue;
                }
                self.fullscreen.remove(&window.id);
                self.reinsert(window, saved, front, &params);
                continue;
            }
            let (known, traits) = self.pool.with_api_ref(|api| (api.knows(window.id), api.describe(window.id)));
            if !known {
                continue;
            }
            if (traits.role.is_empty() && traits.subrole.is_empty())
                || traits.role == "AXUnknown"
                || traits.subrole == "AXUnknown"
            {
                self.pending_traits = true;
                continue;
            }
            fresh.push((window.clone(), traits));
        }
        self.adopt_fresh(fresh, front, own_pid, &params);
        let off_space = self.off_space_catalog(&listed);
        for window in &off_space {
            seen.insert(window.id);
        }
        self.place_off_space(&off_space, front, &params);
        self.transfer_moved(&listed, &params);
        self.transfer_moved(&off_space, &params);
        self.remove_missing(&seen, &params);
        self.check_sizes(&listed);
        self.focus_follows_pointer();
        self.publish_layouts(&displays, &params);
        self.settle = false;
        self.pool.poll_enhanced_ui();
        self.publish_state(&self.screens);
        self.store_metrics();
    }

    /// Copies `AXWindows` only for apps with a window whose element is unknown.
    /// Windows that never resolve are retried every `RESOLVE_RETRY`, so they do
    /// not force an AX walk of every app on every tick.
    fn resolve_listed(&self, listed: &[ListedWindow]) {
        let now = Instant::now();
        let force = self.force_resolve;
        let (pending, get_window) = self.pool.with_api_ref(|api| {
            let retry = self.resolve_after.borrow();
            let list: Vec<ListedWindow> = listed
                .iter()
                .filter(|window| !self.floating.contains_key(&window.id) && !api.knows(window.id))
                .filter(|window| force || retry.get(&window.id).is_none_or(|due| now >= *due))
                .cloned()
                .collect();
            (list, api.get_window_fn())
        });
        if pending.is_empty() {
            return;
        }
        let discovered = ax::discover_elements(get_window, &pending);
        let missing: Vec<u64> = self.pool.with_api(|api| {
            for (id, pid, element) in discovered {
                unsafe {
                    api.remember(id, pid, element);
                    ax::release_element(element);
                }
            }
            pending.iter().filter(|window| !api.knows(window.id)).map(|window| window.id).collect()
        });
        let mut retry = self.resolve_after.borrow_mut();
        for window in &pending {
            retry.remove(&window.id);
        }
        for id in missing {
            retry.insert(id, now + RESOLVE_RETRY);
        }
    }

    /// Space change. Does not sleep. The fallback hash runs only on this path.
    pub fn switch_space(&mut self) {
        let ids: Vec<u64> = ax::on_screen_windows().into_iter().map(|window| window.id).collect();
        let live = crate::private_api::current_space();
        self.space_id = crate::private_api::space_key_after_switch(self.skylight, live, &ids);
        if !self.skylight {
            self.space_switched = true;
        }
        self.settle = true;
        self.force_resolve = true;
        self.tick();
        self.settle = false;
        self.force_resolve = false;
    }

    /// Re-enable the event taps and re-apply the current strips without animation.
    /// The caller waits the 1.5 s settle time without blocking this thread.
    pub fn resync_after_wake(&mut self) {
        let _ = crate::hotkeys::enable_tap();
        self.apply_wake_space();
        self.settle = true;
        self.force_resolve = true;
        self.tick();
        self.settle = false;
        self.force_resolve = false;
    }

    /// SkyLight re-reads the active Space. Without it, `current_space()` is `1`
    /// and would replace the fallback hash `switch_space` stored, so the strip
    /// `publish_layouts` looks up would be gone. Leave that hash in place.
    fn apply_wake_space(&mut self) {
        let live = if self.skylight {
            crate::private_api::current_space()
        } else {
            self.space_id
        };
        self.space_id = space_id_after_wake(self.skylight, live, self.space_id);
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
            std::thread::sleep(Duration::from_millis(1500));
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
            let strip = self.strip_mut(&display).ok_or(CommandError::NoFocusedWindow)?;
            if focus_action && strip.columns.is_empty() && strip.pinned_left.is_none() && strip.pinned_right.is_none() {
                return Err(CommandError::NoFocusedWindow);
            }
            apply_action(strip, action, args, &params, visible)?;
        }
        let screens = self.screens.clone();
        self.publish_layouts(&screens, &params);
        if self.animating {
            self.step_animation(1.0 / 120.0);
        }
        self.publish_state(&screens);
        if focus_action {
            let snap = self.focus_snap(&display)?;
            self.raise_window(snap.window_id);
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
        let stale: Vec<(StripKey, u64)> = self
            .meta
            .iter()
            .filter(|(id, _)| !seen.contains(id))
            .map(|(id, meta)| {
                (
                    StripKey {
                        display: meta.display.clone(),
                        space: meta.space,
                    },
                    *id,
                )
            })
            .collect();
        for (key, id) in stale {
            if let Some(strip) = self.strips.get(&key) {
                if let Some((index, window)) = located(strip, id) {
                    self.departed.insert(
                        id,
                        Departed {
                            window,
                            index,
                            display: key.display.clone(),
                        },
                    );
                }
            }
            let visible = viewport_for_display(&self.screens, &key.display);
            if let Some(strip) = self.strips.get_mut(&key) {
                strip.remove(WindowId(id), params, visible);
            }
            self.meta.remove(&id);
            self.last_on_screen.remove(&id);
            self.motions.remove(&id);
            self.size_misses.remove(&id);
            self.park_repaired.remove(&id);
            self.shown.remove(&id);
            self.pool.with_api(|api| api.forget(id));
        }
    }

    fn publish_layouts(&mut self, displays: &[DisplayInfo], params: &LayoutParams) {
        self.apply_size_overrides();
        let duration = f64::from(self.config.animation.animation_duration_ms) / 1000.0;
        let glide = !self.tracking && !self.settle && duration > 0.0;
        let mut flying = false;
        let mut active_displays = Vec::new();
        for display in displays {
            let key = self.key_of(&display.uuid);
            let Some(mut shown) = self.strips.get(&key).cloned() else {
                continue;
            };
            // The strip scrolls as one body: one spring per strip carries every
            // column, so columns slide in and out together instead of jumping
            // to or from their parks.
            let target = shown.scroll_x;
            let view = self.views.entry(key.clone()).or_insert_with(|| Spring::launch(target, target));
            if glide {
                view.retarget(target);
            }
            let view_moving = glide && ((view.x - target).abs() >= 0.5 || view.v.abs() >= 5.0);
            if !view_moving {
                *view = Spring::launch(target, target);
            }
            let scroll = view.x;
            shown.scroll_x = scroll;
            let pinned: HashSet<u64> = [shown.pinned_left.as_ref(), shown.pinned_right.as_ref()]
                .into_iter()
                .flatten()
                .flat_map(|pin| pin.column.windows.iter().map(|window| window.id.0))
                .collect();
            let visible = viewport(&display.visible);
            let screen = DisplayFrame {
                min_x: display.frame.x,
                min_y: display.frame.y,
                max_x: display.frame.right(),
                max_y: display.frame.bottom(),
                neighbour_left: display.neighbour_left || self.extra_left.contains(&display.uuid),
                neighbour_right: display.neighbour_right || self.extra_right.contains(&display.uuid),
            };
            let placements = layout(&shown, visible, screen, params);
            let mut display_flying = view_moving;
            for placement in placements {
                let id = placement.window.0;
                let hidden = matches!(placement.visibility, Visibility::Hidden);
                let mut frame = placement.frame;
                if hidden {
                    if let Some(repaired) = self.repair_park(id, display, frame) {
                        frame = repaired;
                    }
                } else {
                    self.park_repaired.remove(&id);
                }
                let Some(meta) = self.meta.get(&id) else {
                    if hidden {
                        self.shown.remove(&id);
                    }
                    continue;
                };
                let pid = meta.pid;
                let bundle = meta.bundle_id.clone();
                let anchor = Anchor { strip: key.clone(), pinned: pinned.contains(&id) };
                let offset = if anchor.pinned { 0.0 } else { scroll };
                let (window_flying, published) = self.follow(pid, id, &bundle, frame, !hidden, anchor, offset);
                if hidden {
                    self.shown.remove(&id);
                } else {
                    self.shown.insert(id);
                    self.last_on_screen.insert(id, published);
                }
                display_flying |= window_flying;
            }
            if display_flying {
                flying = true;
                active_displays.push(display.cg_id);
            }
        }
        self.animating = flying;
        if flying {
            for display in active_displays {
                self.driver.start(display);
            }
        } else {
            self.driver.stop();
        }
        if !self.tracking {
            self.store_metrics();
        }
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
            let mut state = self.state.lock().unwrap_or_else(|err| err.into_inner());
            if *state != text {
                *state = text;
                self.state_version.set(self.state_version.get().wrapping_add(1));
            }
        }
    }

    fn display_json(&self, display: &DisplayInfo) -> serde_json::Value {
        let active = self.active_space();
        let mut strips = Vec::new();
        if !self.strips.keys().any(|key| key.display == display.uuid && key.space == active) {
            strips.push(self.strip_body(active, None));
        }
        let mut owned: Vec<(u64, &Strip)> = self
            .strips
            .iter()
            .filter(|(key, _)| key.display == display.uuid)
            .map(|(key, strip)| (key.space, strip))
            .collect();
        owned.sort_by_key(|(space, _)| if *space == active { 0u8 } else { 1 });
        for (space, strip) in owned {
            strips.push(self.strip_body(space, Some(strip)));
        }
        serde_json::json!({
            "uuid": display.uuid,
            "name": display.name,
            "frame": frame_json(display.frame),
            "visible_frame": frame_json(display.visible),
            "ignored": self.config.displays.ignored_displays.iter().any(|id| id == &display.uuid),
            "active_space": active,
            "strips": strips,
        })
    }

    fn strip_body(&self, space: u64, strip: Option<&Strip>) -> serde_json::Value {
        let focused = strip
            .map(|strip| match strip.focus {
                Focus::Column(index) => index as i64,
                _ => 0,
            })
            .unwrap_or(0);
        let columns = strip.map(|strip| self.columns_json(strip)).unwrap_or_default();
        serde_json::json!({
            "space": space,
            "scroll_x": strip.map(|strip| strip.scroll_x).unwrap_or(0.0),
            "focused_column": focused,
            "focus": strip.map(|strip| focus_name(strip.focus)).unwrap_or("none"),
            "focused_window": strip.and_then(focused_window_id).map_or(serde_json::Value::Null, serde_json::Value::from),
            "pinned_left": strip.map(|strip| pin_json(self, strip.pinned_left.as_ref())).unwrap_or(serde_json::Value::Null),
            "pinned_right": strip.map(|strip| pin_json(self, strip.pinned_right.as_ref())).unwrap_or(serde_json::Value::Null),
            "columns": columns,
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
            "visible": self.shown.contains(&window.id.0),
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

impl Runtime {
    /// `target` is the screen frame for this layout. Motions run in strip
    /// coordinates (`screen x + offset`), so scrolling moves no spring but the
    /// strip's own, and a window keeps its place relative to its neighbours.
    #[allow(clippy::too_many_arguments)]
    fn follow(&mut self, pid: i32, id: u64, bundle: &str, target: Frame, animate: bool, anchor: Anchor, offset: f64) -> (bool, Frame) {
        let on_strip = Frame { x: target.x + offset, ..target };
        let duration = f64::from(self.config.animation.animation_duration_ms) / 1000.0;
        let Some(motion) = self.motions.get_mut(&id) else {
            self.pool.publish(pid, id, target, bundle);
            let mut motion = Motion::rest(pid, bundle, on_strip);
            motion.anchor = Some(anchor);
            motion.offset = offset;
            motion.written = Some((target.x, target.y));
            if !animate {
                motion.park = Some(target);
            }
            self.motions.insert(id, motion);
            return (false, target);
        };
        motion.pid = pid;
        if motion.bundle_id != bundle {
            motion.bundle_id = bundle.to_string();
        }
        if motion.anchor.as_ref() != Some(&anchor) {
            // A new strip or pin has its own offset; shift so the window does not jump on screen.
            let shift = offset - motion.offset;
            motion.x.x += shift;
            motion.x.target += shift;
            motion.anchor = Some(anchor);
        }
        motion.offset = offset;
        let differs = |a: (f64, f64), b: Frame| (a.0 - b.x).abs() > 0.1 || (a.1 - b.y).abs() > 0.1;
        // While fingers are tracking, a size write costs tens of milliseconds
        // on Chromium-class apps and queues ahead of every position write on
        // the per-pid worker, so sizes wait for the settle after release.
        let defer_size = self.tracking && !self.settle;
        if !animate {
            let size_changed = (motion.width - target.width).abs() > 0.5 || (motion.height - target.height).abs() > 0.5;
            let moved = motion.park.is_none_or(|park| differs((park.x, park.y), target) || (park.width - target.width).abs() > 0.5 || (park.height - target.height).abs() > 0.5);
            motion.snap(on_strip);
            motion.size_due = defer_size && (motion.size_due || size_changed);
            motion.park = Some(target);
            motion.written = Some((target.x, target.y));
            if !defer_size && (size_changed || self.settle) {
                self.pool.publish(pid, id, target, bundle);
            } else if moved {
                self.pool.publish_position(pid, id, target, bundle);
            }
            return (false, target);
        }
        let resized = (motion.width - target.width).abs() > 0.5 || (motion.height - target.height).abs() > 0.5;
        // Leaving a park, the window starts at its own place on the strip and the
        // strip's spring carries it in with the rest.
        let unparked = motion.park.take().is_some();
        if unparked || self.tracking || self.settle || duration <= 0.0 {
            motion.snap(on_strip);
            motion.size_due = defer_size && (motion.size_due || resized);
            let moved = motion.written.is_none_or(|written| differs(written, target));
            motion.written = Some((target.x, target.y));
            if defer_size {
                if moved || resized {
                    self.pool.publish_position(pid, id, target, bundle);
                }
            } else if resized || self.settle {
                self.pool.publish(pid, id, target, bundle);
            } else if moved {
                self.pool.publish_position(pid, id, target, bundle);
            }
            return (false, target);
        }
        let size_delta = (motion.width - target.width).abs().max((motion.height - target.height).abs());
        let pos_delta = (motion.x.target - on_strip.x).abs().max((motion.y.target - on_strip.y).abs());
        if pos_delta <= 0.5 && size_delta > 0.5 && motion.settled() {
            motion.snap(on_strip);
            motion.size_due = false;
            motion.written = Some((target.x, target.y));
            self.pool.publish(pid, id, target, bundle);
            return (false, target);
        }
        let mut sized = std::mem::take(&mut motion.size_due);
        if size_delta > 0.5 {
            let shrank = target.width * target.height + 1.0 < motion.width * motion.height;
            if shrank {
                // A smaller size lands once the move ends, so the window never
                // shows a gap while it travels.
                motion.pending_shrink = Some((target.width, target.height));
            } else {
                motion.width = target.width;
                motion.height = target.height;
                motion.pending_shrink = None;
                sized = true;
            }
        }
        if pos_delta > 0.5 {
            motion.x.retarget(on_strip.x);
            motion.y.retarget(on_strip.y);
        }
        let current = Frame { x: motion.x.x - offset, y: motion.y.x, width: motion.width, height: motion.height };
        let moved = motion.written.is_none_or(|written| differs(written, current));
        motion.written = Some((current.x, current.y));
        let flying = !motion.settled();
        if sized {
            self.pool.publish(pid, id, current, bundle);
        } else if moved {
            self.pool.publish_position(pid, id, current, bundle);
        }
        (flying, current)
    }

    fn integrate_springs(&mut self, dt: f64) {
        if !self.animating {
            return;
        }
        let zeta = self.config.animation.animation_damping;
        let omega = Spring::omega(f64::from(self.config.animation.animation_duration_ms) / 1000.0);
        for view in self.views.values_mut() {
            if (view.x - view.target).abs() < 0.5 && view.v.abs() < 5.0 {
                continue;
            }
            let before = view.x - view.target;
            if view.step(dt, zeta.max(1.0), omega) || before * (view.x - view.target) <= 0.0 {
                *view = Spring::launch(view.target, view.target);
            }
        }
        for motion in self.motions.values_mut() {
            if motion.park.is_some() || motion.settled() {
                continue;
            }
            let x_done = motion.x.step(dt, zeta, omega);
            let y_done = motion.y.step(dt, zeta, omega);
            if x_done && y_done {
                motion.x = Spring::launch(motion.x.target, motion.x.target);
                motion.y = Spring::launch(motion.y.target, motion.y.target);
                if let Some((width, height)) = motion.pending_shrink.take() {
                    motion.width = width;
                    motion.height = height;
                    motion.size_due = true;
                }
            }
        }
    }

    fn step_animation(&mut self, dt: f64) {
        if !self.animating {
            return;
        }
        self.integrate_springs(dt);
        // Each frame lays the strip out again at the moving scroll, so columns
        // that slide fully off screen are parked and ones that slide on are placed.
        let screens = self.screens.clone();
        let params = layout_params(&self.config);
        self.publish_layouts(&screens, &params);
    }
    fn store_metrics(&self) {
        // publish_layouts calls this per animation frame; percentiles over
        // every recorded write do not need rebuilding at frame rate.
        let now = Instant::now();
        if self.animating && self.metrics_at.get().is_some_and(|then| now.duration_since(then) < Duration::from_millis(250)) {
            return;
        }
        self.metrics_at.set(Some(now));
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
                let known = self.screens.iter().any(|screen| screen.uuid == uuid) || self.has_display(uuid);
                if !known {
                    return Err(CommandError::DisplayNotFound);
                }
                return Ok(uuid.to_string());
            }
        }
        self.active_display().ok_or(CommandError::NoFocusedWindow)
    }

    fn active_display(&self) -> Option<String> {
        let space = self.active_space();
        let occupied = self.screens.iter().find_map(|screen| {
            let key = StripKey { display: screen.uuid.clone(), space };
            let strip = self.strips.get(&key)?;
            let occupied = !strip.columns.is_empty() || strip.pinned_left.is_some() || strip.pinned_right.is_some();
            occupied.then(|| screen.uuid.clone())
        });
        occupied.or_else(|| self.screens.first().map(|screen| screen.uuid.clone()))
    }

    /// A swipe scrolls the strip on the display under the pointer.
    fn swipe_display(&self) -> Option<String> {
        let under = self.screen_at(self.pointer_x, self.pointer_y).filter(|uuid| self.strip(uuid).is_some_and(|strip| !strip.columns.is_empty()));
        under.or_else(|| self.active_display())
    }

    fn viewport_of(&self, uuid: &str) -> Viewport {
        viewport_for_display(&self.screens, uuid)
    }

    fn active_space(&self) -> u64 {
        active_space_id(self.skylight, self.space_id, self.space_switched)
    }

    fn key_of(&self, display: &str) -> StripKey {
        StripKey { display: display.to_string(), space: self.active_space() }
    }

    fn strip(&self, display: &str) -> Option<&Strip> {
        let key = self.key_of(display);
        self.strips.get(&key)
    }

    fn strip_mut(&mut self, display: &str) -> Option<&mut Strip> {
        let key = self.key_of(display);
        self.strips.get_mut(&key)
    }

    fn entry_active(&mut self, display: &str) -> &mut Strip {
        let key = self.key_of(display);
        self.strips.entry(key).or_default()
    }

    fn has_display(&self, uuid: &str) -> bool {
        self.strips.keys().any(|key| key.display == uuid)
    }

    fn raise_window(&self, id: u64) {
        let Some(pid) = self.meta.get(&id).map(|meta| meta.pid) else {
            return;
        };
        self.pool.with_api_ref(|api| {
            let mut host = crate::focus::AxHost { api };
            crate::focus::raise_focused(&mut host, id, pid);
        });
    }

    fn locate_window(&self, id: u64) -> Option<(String, usize)> {
        let active = self.active_space();
        let mut fallback = None;
        for (key, strip) in &self.strips {
            let pinned = strip
                .pinned_left
                .as_ref()
                .into_iter()
                .chain(strip.pinned_right.as_ref())
                .any(|pin| pin.column.windows.iter().any(|window| window.id.0 == id));
            let index = strip
                .columns
                .iter()
                .position(|column| column.windows.iter().any(|window| window.id.0 == id))
                .or_else(|| pinned.then_some(0));
            if let Some(index) = index {
                let found = (key.display.clone(), index);
                if key.space == active {
                    return Some(found);
                }
                if fallback.is_none() {
                    fallback = Some(found);
                }
            }
        }
        fallback
    }

    fn focus_snap(&self, display: &str) -> Result<FocusSnap, CommandError> {
        let strip = self.strip(display).ok_or(CommandError::NoFocusedWindow)?;
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
        if kind == 1 {
            self.drag = self.window_under(x, y, GRAB_MARGIN).map(|(id, display, column, frame)| {
                let on_edge = (x - frame.x).abs() <= GRAB_MARGIN + 2.0
                    || (x - frame.right()).abs() <= GRAB_MARGIN + 2.0
                    || (y - frame.y).abs() <= GRAB_MARGIN + 2.0
                    || (y - frame.bottom()).abs() <= GRAB_MARGIN + 2.0;
                Drag {
                    id,
                    display,
                    column,
                    x,
                    y,
                    title_bar: x >= frame.x && x < frame.right() && y >= frame.y && y <= frame.y + 52.0,
                    on_edge,
                }
            });
            return;
        }
        if kind != 2 {
            return;
        }
        let Some(drag) = self.drag.take() else {
            return;
        };
        if (x - drag.x).hypot(y - drag.y) < DRAG_SLOP {
            self.focus_clicked_window(&drag);
            return;
        }
        if drag.on_edge && self.adopt_hand_resize(&drag) {
            return;
        }
        if !drag.title_bar && !drag.on_edge {
            return;
        }
        let actual = self.pool.with_api_ref(|api| api.read_frame(drag.id));
        if actual.width >= 1.0 && actual.height >= 1.0 {
            if let Some(motion) = self.motions.get_mut(&drag.id) {
                motion.x.x = actual.x + motion.offset;
                motion.y.x = actual.y;
                motion.written = Some((actual.x, actual.y));
            }
        } else {
            self.motions.remove(&drag.id);
        }
        let params = layout_params(&self.config);
        let screens = self.screens.clone();
        if !drag.title_bar || !self.config.focus.drag_to_reorder {
            self.publish_layouts(&screens, &params);
            self.publish_state(&screens);
            return;
        }
        let Some(target) = self.screen_at(x, y) else {
            self.publish_layouts(&screens, &params);
            self.publish_state(&screens);
            return;
        };
        if target != drag.display {
            self.move_window_to_display(drag.id, &drag.display, &target, &params);
            return;
        }
        let frames = self.column_spans(&drag.display);
        let Some(dest) = restore::drop_index(&frames, x) else {
            self.publish_layouts(&screens, &params);
            self.publish_state(&screens);
            return;
        };
        let visible = self.viewport_of(&drag.display);
        if let Some(strip) = self.strip_mut(&drag.display) {
            let to = dest.min(strip.columns.len().saturating_sub(1));
            if to != drag.column {
                strip.move_column(drag.column, to);
                strip.reveal(&params, visible);
            }
        }
        self.publish_layouts(&screens, &params);
        self.publish_state(&screens);
    }

    fn focus_clicked_window(&mut self, drag: &Drag) {
        let params = layout_params(&self.config);
        let visible = self.viewport_of(&drag.display);
        let mut changed = false;
        if let Some(strip) = self.strip_mut(&drag.display) {
            let before_focus = strip.focus;
            let before_scroll = strip.scroll_x;
            if let Some(column) = strip.columns.get_mut(drag.column) {
                if select_window(Some(column), drag.id) {
                    strip.focus = Focus::Column(drag.column);
                    strip.reveal(&params, visible);
                    changed = strip.focus != before_focus || (strip.scroll_x - before_scroll).abs() > 0.5;
                }
            }
        }
        if changed {
            let screens = self.screens.clone();
            self.publish_layouts(&screens, &params);
            if self.animating {
                self.step_animation(1.0 / 120.0);
            }
            self.publish_state(&screens);
        }
    }

    /// A window resized by hand keeps its new width: its column takes that
    /// width and the strip reflows around it at once, instead of the next tick
    /// writing the old size back.
    fn adopt_hand_resize(&mut self, drag: &Drag) -> bool {
        let Some(motion) = self.motions.get(&drag.id) else {
            return false;
        };
        let (want_w, want_h) = (motion.width, motion.height);
        let actual = self.pool.with_api_ref(|api| api.read_frame(drag.id));
        if actual.width < 1.0 || actual.height < 1.0 || ((actual.width - want_w).abs() <= 4.0 && (actual.height - want_h).abs() <= 4.0) {
            return false;
        }
        let visible = self.viewport_of(&drag.display);
        let params = layout_params(&self.config);
        let vw = self.strip(&drag.display).map_or((visible.width - 2.0 * params.gap_outer).max(1.0), |strip| strip_core::scroll_width(strip, &params, visible).max(1.0));
        let ratio = ((actual.width + params.gap_inner) / (vw + params.gap_inner).max(1.0)).clamp(0.1, 1.0);
        let Some(strip) = self.strip_mut(&drag.display) else {
            return false;
        };
        let Some(column) = strip.columns.get_mut(drag.column).filter(|column| column.windows.iter().any(|window| window.id.0 == drag.id)) else {
            return false;
        };
        column.width_ratio = ratio;
        for window in &mut column.windows {
            window.size_override = None;
        }
        strip.focus = Focus::Column(drag.column);
        strip.reveal(&params, visible);
        if let Some(motion) = self.motions.get_mut(&drag.id) {
            motion.snap(Frame { x: actual.x + motion.offset, ..actual });
            motion.written = Some((actual.x, actual.y));
        }
        self.size_misses.remove(&drag.id);
        let screens = self.screens.clone();
        self.publish_layouts(&screens, &params);
        self.publish_state(&screens);
        true
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
            let strip = self.strip(&display).ok_or(CommandError::NoFocusedWindow)?;
            focused_window(strip).cloned().ok_or(CommandError::NoFocusedWindow)?
        };
        let id = window.id.0;
        let meta = self.meta.get(&id).cloned();
        if let Some(strip) = self.strip_mut(&display) {
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

    /// The system-focused window, when it is one of the floating windows.
    fn args_window_floating(&self) -> Option<u64> {
        let front = ax::frontmost_pid();
        if front <= 0 {
            return None;
        }
        let id = self.pool.with_api_ref(|api| ax::focused_window(api, front))?;
        self.floating.contains_key(&id).then_some(id)
    }

    fn unfloat(&mut self, id: u64) -> Result<(), CommandError> {
        let info = self.floating.remove(&id).ok_or(CommandError::NoFocusedWindow)?;
        let display = self.active_display().ok_or(CommandError::NoFocusedWindow)?;
        let params = layout_params(&self.config);
        let visible = self.viewport_of(&display);
        let window = Window::new(id, info.pid);
        let space = self.active_space();
        // Passing the window's own pid as frontmost focuses the unfloated column.
        self.entry_active(&display).insert(window, info.pid, None, &params, visible);
        self.meta.insert(
            id,
            Meta {
                pid: info.pid,
                bundle_id: info.bundle_id.clone(),
                title: info.title.clone(),
                owner: info.bundle_id.clone(),
                display,
                space,
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
            let strip = self.strip(from).ok_or(CommandError::NoFocusedWindow)?;
            focused_window(strip).cloned().ok_or(CommandError::NoFocusedWindow)?
        };
        let id = window.id.0;
        if let Some(strip) = self.strip_mut(from) {
            strip.remove(WindowId(id), params, visible_from);
        }
        let visible = self.viewport_of(target);
        let pid = window.pid;
        self.entry_active(target).insert(window, pid, None, params, visible);
        if let Some(meta) = self.meta.get_mut(&id) {
            meta.display = target.to_string();
        }
        let screens = self.screens.clone();
        self.publish_layouts(&screens, params);
        self.publish_state(&screens);
        Ok(())
    }

    fn move_window_to_display(&mut self, id: u64, from: &str, target: &str, params: &LayoutParams) {
        let Some(window) = self.strip(from).and_then(|strip| located(strip, id)).map(|(_, window)| window) else {
            return;
        };
        let visible_from = self.viewport_of(from);
        if let Some(strip) = self.strip_mut(from) {
            strip.remove(WindowId(id), params, visible_from);
        }
        let visible = self.viewport_of(target);
        let pid = window.pid;
        self.entry_active(target).insert(window, pid, None, params, visible);
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
            self.strip(&saved.display).and_then(|strip| {
                strip.columns.iter().position(|column| column.windows.iter().any(|window| siblings.contains(&window.id.0)))
            })
        };
        let display = saved.display.clone();
        let role = saved.role.clone();
        let subrole = saved.subrole.clone();
        if let Some(index) = appended {
            if let Some(strip) = self.strip_mut(&display) {
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
                let strip = self.entry_active(&display);
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
            let strip = self.entry_active(&uuid);
            strip.insert(model_from(window), front, slot, params, visible);
        }
        self.remember(window, &uuid, &role, &subrole);
        uuid
    }

    fn push_tab(&mut self, display: &str, leader: u64, peer: &ListedWindow, traits: &Traits) {
        let role = traits.role.clone();
        let subrole = traits.subrole.clone();
        let pushed = self.strip_mut(display).and_then(|strip| {
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
        self.entry_active(&display).insert(model, front, Some(slot), params, visible);
        self.remember(window, &display, "", "");
    }

    fn remember(&mut self, window: &ListedWindow, display: &str, role: &str, subrole: &str) {
        let bundle = config::shown_bundle(&ax::bundle_id(window.pid), &window.owner);
        let space = if window.space == 0 { self.active_space() } else { window.space };
        self.meta.insert(
            window.id,
            Meta {
                pid: window.pid,
                bundle_id: bundle,
                title: window.title.clone(),
                owner: window.owner.clone(),
                display: display.to_string(),
                space,
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
        let Some((display, space)) = self.meta.get(&id).map(|meta| (meta.display.clone(), meta.space)) else {
            return;
        };
        let key = StripKey { display: display.clone(), space };
        let visible = viewport_for_display(&self.screens, &display);
        if let Some(strip) = self.strips.get_mut(&key) {
            strip.remove(WindowId(id), params, visible);
        }
        self.meta.remove(&id);
        self.motions.remove(&id);
        self.park_repaired.remove(&id);
    }

    fn off_space_catalog(&self, on_screen: &[ListedWindow]) -> Vec<ListedWindow> {
        if !self.skylight {
            return Vec::new();
        }
        let active = self.active_space();
        let on_ids: Vec<u64> = on_screen.iter().map(|window| window.id).collect();
        let outside: Vec<(u64, u64)> = crate::private_api::windows_outside(&on_ids)
            .into_iter()
            .filter(|(_, space)| *space != active)
            .collect();
        if outside.is_empty() {
            return Vec::new();
        }
        let mut by_id: HashMap<u64, ListedWindow> = self.eligible(&ax::off_screen_windows()).into_iter().map(|window| (window.id, window)).collect();
        let mut off = Vec::new();
        for (id, space) in outside {
            let Some(mut window) = by_id.remove(&id).or_else(|| self.synthesized_off_screen(id)) else {
                continue;
            };
            if window.pid == 0 {
                continue;
            }
            window.space = crate::private_api::assign_strip_space(true, active, Some(space), false);
            off.push(window);
        }
        off
    }

    fn synthesized_off_screen(&self, id: u64) -> Option<ListedWindow> {
        let meta = self.meta.get(&id)?;
        if meta.pid == 0 {
            return None;
        }
        Some(ListedWindow {
            id,
            pid: meta.pid,
            title: meta.title.clone(),
            owner: meta.owner.clone(),
            bounds: self.last_on_screen.get(&id).copied().unwrap_or(Frame { x: 0.0, y: 0.0, width: 0.0, height: 0.0 }),
            space: meta.space,
        })
    }

    fn place_off_space(&mut self, windows: &[ListedWindow], front: i32, params: &LayoutParams) {
        let own_pid = std::process::id() as i32;
        for window in windows {
            if crate::private_api::space_is_fullscreen(window.space) {
                self.depart_fullscreen(window, params);
                continue;
            }
            if self.contains_id(window.id) || self.floating.contains_key(&window.id) || self.fullscreen.contains_key(&window.id) {
                continue;
            }
            let Some(display) = display_for(&self.screens, window.bounds).map(|screen| screen.uuid.clone()) else {
                continue;
            };
            if self.config.displays.ignored_displays.iter().any(|id| id == &display) {
                continue;
            }
            let mut traits = self.pool.with_api_ref(|api| api.describe(window.id));
            if traits.role.is_empty() && traits.subrole.is_empty() {
                let mut cursor = self.remote_cursors.get(&window.id).copied().unwrap_or(0);
                let found = self.pool.with_api(|api| ax::resolve_remote(api, window.id, window.pid, &mut cursor, 128));
                self.remote_cursors.insert(window.id, cursor);
                if !found {
                    continue;
                }
                traits = self.pool.with_api_ref(|api| api.describe(window.id));
                if traits.role.is_empty() && traits.subrole.is_empty() {
                    traits = standard_window(traits);
                }
            }
            if traits.fullscreen {
                self.depart_fullscreen(window, params);
                continue;
            }
            match self.decide(window, &traits, own_pid) {
                Placement::Float => self.remember_float(window),
                Placement::Fullscreen => self.depart_fullscreen(window, params),
                Placement::Manage { .. } => self.insert_on_space(window, &display, window.space, front, params),
            }
        }
    }

    fn insert_on_space(&mut self, window: &ListedWindow, display: &str, space: u64, front: i32, params: &LayoutParams) {
        let visible = viewport_for_display(&self.screens, display);
        let key = StripKey { display: display.to_string(), space };
        self.strips.entry(key).or_default().insert(model_from(window), front, None, params, visible);
        self.remember(window, display, "", "");
    }

    fn depart_fullscreen(&mut self, window: &ListedWindow, params: &LayoutParams) {
        if self.fullscreen.contains_key(&window.id) && !self.meta.contains_key(&window.id) {
            return;
        }
        let (display, space, index, model) = if let Some(meta) = self.meta.get(&window.id) {
            let display = meta.display.clone();
            let space = meta.space;
            let key = StripKey { display: display.clone(), space };
            let located_window = self.strips.get(&key).and_then(|strip| located(strip, window.id));
            let index = located_window.as_ref().map(|(index, _)| *index).unwrap_or(0);
            let model = located_window.map(|(_, model)| model).unwrap_or_else(|| model_from(window));
            (display, space, index, model)
        } else {
            (String::new(), window.space, 0, model_from(window))
        };
        if !display.is_empty() {
            let visible = viewport_for_display(&self.screens, &display);
            let key = StripKey { display: display.clone(), space };
            if let Some(strip) = self.strips.get_mut(&key) {
                strip.remove(WindowId(window.id), params, visible);
            }
        }
        self.meta.remove(&window.id);
        self.motions.remove(&window.id);
        self.fullscreen.insert(window.id, Departed { window: model, index, display });
    }

    fn transfer_moved(&mut self, windows: &[ListedWindow], params: &LayoutParams) {
        let planned: Vec<(u64, String, u64)> = windows
            .iter()
            .filter_map(|window| {
                let meta = self.meta.get(&window.id)?;
                let display = display_for(&self.screens, window.bounds)
                    .map(|screen| screen.uuid.clone())
                    .filter(|uuid| !uuid.is_empty())
                    .unwrap_or_else(|| meta.display.clone());
                if display.is_empty() || (meta.display == display && meta.space == window.space) {
                    return None;
                }
                Some((window.id, display, window.space))
            })
            .collect();
        for (id, display, space) in planned {
            self.transfer_window(id, &display, space, params);
        }
    }

    fn transfer_window(&mut self, id: u64, display: &str, space: u64, params: &LayoutParams) {
        let Some((from_display, from_space)) = self.meta.get(&id).map(|meta| (meta.display.clone(), meta.space)) else {
            return;
        };
        let from = StripKey { display: from_display.clone(), space: from_space };
        let Some(window) = self.strips.get(&from).and_then(|strip| located(strip, id)).map(|(_, window)| window) else {
            return;
        };
        let visible_from = viewport_for_display(&self.screens, &from_display);
        if let Some(strip) = self.strips.get_mut(&from) {
            strip.remove(WindowId(id), params, visible_from);
        }
        let visible = viewport_for_display(&self.screens, display);
        let pid = window.pid;
        let key = StripKey { display: display.to_string(), space };
        self.strips.entry(key).or_default().insert(window, pid, None, params, visible);
        if let Some(meta) = self.meta.get_mut(&id) {
            meta.display = display.to_string();
            meta.space = space;
        }
    }

    /// Some apps accept a size write and then put their own size back later, so
    /// the read-back after the write cannot catch them. The window list shows
    /// the real size each tick: a mismatch is written again once, and a window
    /// that still keeps its own size gets it, with its column sized to fit.
    fn check_sizes(&mut self, listed: &[ListedWindow]) {
        if self.animating || self.drag.is_some() || self.gesture.is_open() {
            return;
        }
        for window in listed {
            let id = window.id;
            let Some(motion) = self.motions.get(&id) else {
                continue;
            };
            if !self.shown.contains(&id) || motion.park.is_some() || !motion.settled() || motion.pending_shrink.is_some() {
                self.size_misses.remove(&id);
                continue;
            }
            let want = (motion.width, motion.height);
            let got = (window.bounds.width, window.bounds.height);
            let close = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() <= 2.0 && (a.1 - b.1).abs() <= 2.0;
            if close(want, got) || got.0 < 1.0 || got.1 < 1.0 {
                self.size_misses.remove(&id);
                continue;
            }
            let frame = Frame { x: motion.x.x - motion.offset, y: motion.y.x, width: want.0, height: want.1 };
            let (pid, bundle) = (motion.pid, motion.bundle_id.clone());
            let entry = self.size_misses.entry(id).or_insert((0, got));
            if !close(entry.1, got) {
                *entry = (0, got);
            }
            entry.0 += 1;
            if entry.0 >= 2 {
                self.size_misses.remove(&id);
                for strip in self.strips.values_mut() {
                    if set_size_override(strip, id, got) {
                        break;
                    }
                }
            } else {
                self.pool.publish(pid, id, frame, &bundle);
            }
        }
    }

    fn apply_size_overrides(&mut self) {
        let overrides = self.pool.take_size_overrides();
        for (id, size) in overrides {
            for strip in self.strips.values_mut() {
                if set_size_override(strip, id, size) {
                    break;
                }
            }
        }
    }

    /// Read the parked frame back. A center on another display flips the corner
    /// and exposes 1 pt until the window is visible again. Later layouts still
    /// ask for `sliver_used(gap_outer, 5)`, so a window already repaired keeps
    /// returning the 1 pt frame instead of that 5 pt corner.
    fn repair_park(&mut self, id: u64, display: &DisplayInfo, parked: Frame) -> Option<Frame> {
        if self.park_repaired.contains(&id) {
            return Some(force_park_sliver(parked, display.frame.x, display.frame.right(), display.frame.bottom(), 1.0));
        }
        let actual = self.pool.read_back(id)?;
        if actual.width < 1.0 || actual.height < 1.0 {
            return None;
        }
        let home = self.meta.get(&id).map(|meta| meta.display.clone()).unwrap_or_default();
        let landed = display_for(&self.screens, actual).map(|screen| screen.uuid.clone())?;
        let corner = park_corner(parked, display.frame.x);
        relocated_repair(&home, &landed, corner)?;
        let (flipped, sliver) = repair_relocated(true, corner);
        match corner {
            Corner::BottomLeft => {
                self.extra_left.insert(display.uuid.clone());
            }
            Corner::BottomRight => {
                self.extra_right.insert(display.uuid.clone());
            }
        }
        self.park_repaired.insert(id);
        let width = parked.width.max(sliver);
        let height = parked.height.max(sliver);
        let (x, y) = match flipped {
            Corner::BottomRight => (display.frame.right() - sliver, parked.y),
            Corner::BottomLeft => (display.frame.x - width + sliver, parked.y),
        };
        Some(Frame { x, y, width, height })
    }

    fn focus_follows_pointer(&mut self) {
        if !self.config.focus.focus_follows_mouse || !self.pointer_seen || self.pointer_at.elapsed() < Duration::from_millis(100) {
            return;
        }
        // One decision per pointer rest, so keyboard focus changes are not undone
        // while the mouse sits still over another window.
        self.pointer_seen = false;
        let Some((id, display, column, _)) = self.window_under(self.pointer_x, self.pointer_y, 0.0) else {
            return;
        };
        let Some(strip) = self.strip_mut(&display) else {
            return;
        };
        if strip.focus == Focus::Column(column) {
            return;
        }
        strip.focus = Focus::Column(column);
        if let Some(column) = strip.columns.get_mut(column) {
            if let Some(index) = column.windows.iter().position(|window| window.id.0 == id) {
                column.focused_window = index;
            }
        }
        self.raise_window(id);
        let screens = self.screens.clone();
        self.publish_state(&screens);
    }

    fn window_under(&self, x: f64, y: f64, margin: f64) -> Option<(u64, String, usize, Frame)> {
        let active = self.active_space();
        for (key, strip) in &self.strips {
            if key.space != active {
                continue;
            }
            for (index, column) in strip.columns.iter().enumerate() {
                for window in &column.windows {
                    let Some(frame) = self.last_on_screen.get(&window.id.0).copied().filter(|_| self.shown.contains(&window.id.0)) else {
                        continue;
                    };
                    if x >= frame.x - margin && x < frame.right() + margin && y >= frame.y - margin && y < frame.bottom() + margin {
                        return Some((window.id.0, key.display.clone(), index, frame));
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
        let Some(strip) = self.strip(display) else {
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

fn focus_name(focus: Focus) -> &'static str {
    match focus {
        Focus::None => "none",
        Focus::Column(_) => "column",
        Focus::PinnedLeft => "pinned_left",
        Focus::PinnedRight => "pinned_right",
    }
}

fn focused_window_id(strip: &Strip) -> Option<u64> {
    let column = match strip.focus {
        Focus::Column(index) => strip.columns.get(index),
        Focus::PinnedLeft => strip.pinned_left.as_ref().map(|pin| &pin.column),
        Focus::PinnedRight => strip.pinned_right.as_ref().map(|pin| &pin.column),
        Focus::None => None,
    }?;
    let window = column.windows.get(column.focused_window).or_else(|| column.windows.first())?;
    Some(window.id.0)
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

/// A tile or tab click focuses that window inside its column.
fn select_window(column: Option<&mut strip_core::Column>, id: u64) -> bool {
    let Some(column) = column else {
        return false;
    };
    let Some(index) = column.windows.iter().position(|window| window.id.0 == id) else {
        return false;
    };
    column.focused_window = index;
    column.active_tab = index;
    true
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
            if select_window(strip.pinned_left.as_mut().map(|pin| &mut pin.column), id) {
                strip.focus = Focus::PinnedLeft;
            } else if select_window(strip.pinned_right.as_mut().map(|pin| &mut pin.column), id) {
                strip.focus = Focus::PinnedRight;
            } else {
                let index = strip
                    .columns
                    .iter()
                    .position(|column| column.windows.iter().any(|window| window.id.0 == id))
                    .ok_or(CommandError::NoFocusedWindow)?;
                select_window(Some(&mut strip.columns[index]), id);
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

fn viewport_for_display(screens: &[DisplayInfo], display: &str) -> Viewport {
    screens
        .iter()
        .find(|screen| screen.uuid == display)
        .map(|screen| viewport(&screen.visible))
        .unwrap_or(Viewport { x: 0.0, y: 0.0, width: 1.0, height: 1.0 })
}

fn active_space_id(skylight: bool, space_id: u64, switched: bool) -> u64 {
    if skylight {
        space_id.max(1)
    } else if switched {
        space_id
    } else {
        0
    }
}

/// Wake keeps the fallback hash when SkyLight is missing. Replacing it with
/// `current_space()` (which is `1`) would publish the wrong strip.
fn space_id_after_wake(skylight: bool, live_space: u64, fallback_hash: u64) -> u64 {
    if skylight {
        live_space.max(1)
    } else {
        fallback_hash
    }
}

/// Later layouts of a relocated park stay at `s = 1`. The ordinary layout
/// republishes `sliver_used(gap_outer, 5)`, which is 5 whenever the outer gap
/// is non-zero, even after the corner has been flipped.
fn force_park_sliver(frame: Frame, min_x: f64, max_x: f64, max_y: f64, sliver: f64) -> Frame {
    let width = frame.width.max(sliver);
    let height = frame.height.max(sliver);
    let _ = max_y;
    if frame.x >= min_x {
        Frame { x: max_x - sliver, y: frame.y, width, height }
    } else {
        Frame { x: min_x - width + sliver, y: frame.y, width, height }
    }
}

/// A remote token matched this window, but Accessibility has not filled a role yet.
fn standard_window(mut traits: Traits) -> Traits {
    if traits.role.is_empty() {
        traits.role = "AXWindow".to_string();
    }
    if traits.subrole.is_empty() {
        traits.subrole = "AXStandardWindow".to_string();
    }
    traits
}

fn park_corner(frame: Frame, display_min_x: f64) -> Corner {
    if frame.x >= display_min_x {
        Corner::BottomRight
    } else {
        Corner::BottomLeft
    }
}

fn set_size_override(strip: &mut Strip, id: u64, size: (f64, f64)) -> bool {
    for column in &mut strip.columns {
        if let Some(window) = column.windows.iter_mut().find(|window| window.id.0 == id) {
            window.size_override = Some(size);
            return true;
        }
    }
    for pin in [&mut strip.pinned_left, &mut strip.pinned_right] {
        let Some(pin) = pin.as_mut() else {
            continue;
        };
        if let Some(window) = pin.column.windows.iter_mut().find(|window| window.id.0 == id) {
            window.size_override = Some(size);
            return true;
        }
    }
    false
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

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(width: f64) -> DisplayInfo {
        DisplayInfo {
            uuid: "main".into(),
            cg_id: 1,
            name: "main".into(),
            frame: Frame { x: 0.0, y: 0.0, width, height: 1117.0 },
            visible: Frame { x: 0.0, y: 33.0, width, height: 978.0 },
            neighbour_left: false,
            neighbour_right: false,
        }
    }

    #[test]
    fn viewport_for_display_uses_the_visible_frame() {
        let view = viewport_for_display(&[screen(1728.0)], "main");
        assert_eq!(view.width, 1728.0);
        assert!(view.width > 100.0);
        assert_eq!(viewport_for_display(&[], "missing").width, 1.0);
    }

    #[test]
    fn active_space_stays_zero_until_a_fallback_switch() {
        assert_eq!(active_space_id(false, 1, false), 0);
        assert_eq!(active_space_id(false, 99, true), 99);
        assert_eq!(active_space_id(true, 4, false), 4);
        assert_eq!(active_space_id(true, 0, false), 1);
    }

    #[test]
    fn park_corner_follows_the_visible_edge() {
        let left = Frame { x: -100.0, y: 0.0, width: 40.0, height: 40.0 };
        let right = Frame { x: 10.0, y: 0.0, width: 40.0, height: 40.0 };
        assert_eq!(park_corner(left, 0.0), Corner::BottomLeft);
        assert_eq!(park_corner(right, 0.0), Corner::BottomRight);
    }

    #[test]
    fn wake_keeps_the_fallback_space_hash() {
        let hash = crate::private_api::fallback_space_key(&[4, 9, 1]);
        assert_ne!(hash, 1);
        assert_eq!(space_id_after_wake(false, 1, hash), hash);
        assert_eq!(space_id_after_wake(true, 42, hash), 42);
        assert_eq!(space_id_after_wake(true, 0, hash), 1);
    }

    #[test]
    fn a_relocated_park_stays_at_one_point() {
        let layout = Frame { x: 1728.0 - 5.0, y: 41.0, width: 800.0, height: 600.0 };
        let held = force_park_sliver(layout, 0.0, 1728.0, 1117.0, 1.0);
        assert_eq!(held.x, 1727.0);
        assert_eq!(held.y, 41.0);
        assert!((1728.0 - held.x).abs() < 2.0);
        let left = Frame { x: 0.0 - 800.0 + 5.0, y: 41.0, width: 800.0, height: 600.0 };
        let held_left = force_park_sliver(left, 0.0, 1728.0, 1117.0, 1.0);
        assert_eq!(held_left.x, 1.0 - 800.0);
        assert_eq!(held_left.y, 41.0);
    }

    #[test]
    fn wake_keeps_the_hash_through_the_session_call() {
        let hash = crate::private_api::fallback_space_key(&[4, 9, 1]);
        assert_ne!(hash, 1);
        let mut runtime = Runtime::blank(false);
        runtime.space_id = hash;
        runtime.space_switched = true;
        runtime.apply_wake_space();
        runtime.tick();
        assert_eq!(runtime.space_id, hash);
        assert_eq!(runtime.active_space(), hash);

        let mut live = Runtime::blank(true);
        live.space_id = 0;
        live.apply_wake_space();
        assert!(live.space_id >= 1);
    }

    #[test]
    fn a_second_layout_keeps_the_relocated_park_at_one_point() {
        let mut runtime = Runtime::blank(false);
        runtime.space_switched = false;
        runtime.space_id = 0;
        let mut main = screen(1728.0);
        main.uuid = "main".into();
        main.name = "main".into();
        let mut side = screen(1728.0);
        side.uuid = "side".into();
        side.name = "side".into();
        side.frame.x = 1728.0;
        side.visible.x = 1728.0;
        runtime.screens = vec![main, side];
        let id = 900_001u64;
        let mut strip = Strip::new();
        strip.insert_at(Window::new(id, 424_242), 0, 0.5);
        strip.scroll_x = 8000.0;
        runtime.strips.insert(StripKey { display: "main".into(), space: 0 }, strip);
        runtime.meta.insert(
            id,
            Meta {
                pid: 424_242,
                bundle_id: "example.park".into(),
                title: "Park".into(),
                owner: "Park".into(),
                display: "main".into(),
                space: 0,
                role: "AXWindow".into(),
                subrole: "AXStandardWindow".into(),
            },
        );
        runtime.pool.testing_set_read_back(
            id,
            Frame { x: 1800.0, y: 100.0, width: 400.0, height: 300.0 },
        );
        let params = layout_params(&runtime.config);
        runtime.publish_layouts(&runtime.screens.clone(), &params);
        assert!(runtime.park_repaired.contains(&id));
        runtime.screens[0].frame.width = 1600.0;
        runtime.publish_layouts(&runtime.screens.clone(), &params);
        let park = runtime.motions.get(&id).and_then(|motion| motion.park).expect("parked motion");
        assert_eq!(park.x, 1599.0);
        assert!((1600.0 - park.x - 1.0).abs() < 0.01);
    }

    #[test]
    fn a_pinned_window_resolves_to_its_display() {
        let mut runtime = Runtime::blank(false);
        runtime.screens = vec![screen(1728.0)];
        let mut strip = Strip::new();
        strip.insert_at(Window::new(7, 1), 0, 0.5);
        strip.focus = Focus::Column(0);
        let params = layout_params(&runtime.config);
        let visible = viewport_for_display(&runtime.screens, "main");
        strip.pin_left(&params, visible);
        assert!(matches!(strip.focus, Focus::PinnedLeft));
        runtime.strips.insert(StripKey { display: "main".into(), space: 0 }, strip);
        let (display, _) = runtime.locate_window(7).expect("pinned window");
        assert_eq!(display, "main");
        runtime.publish_state(&runtime.screens);
        let text = runtime.snapshot();
        let focus = text["displays"][0]["strips"][0]["focus"].as_str();
        let window = text["displays"][0]["strips"][0]["focused_window"].as_u64();
        assert_eq!(focus, Some("pinned_left"));
        assert_eq!(window, Some(7));
    }

    fn swipe_runtime() -> Runtime {
        let mut runtime = Runtime::blank(false);
        runtime.screens = vec![screen(1728.0)];
        let mut strip = Strip::new();
        for (index, id) in [910_001u64, 910_002, 910_003, 910_004].into_iter().enumerate() {
            strip.insert_at(Window::new(id, 424_243), index, 0.5);
            runtime.meta.insert(
                id,
                Meta {
                    pid: 424_243,
                    bundle_id: "example.swipe".into(),
                    title: format!("Swipe {index}"),
                    owner: "Swipe".into(),
                    display: "main".into(),
                    space: 0,
                    role: "AXWindow".into(),
                    subrole: "AXStandardWindow".into(),
                },
            );
        }
        strip.focus = Focus::Column(0);
        runtime.strips.insert(StripKey { display: "main".into(), space: 0 }, strip);
        let params = layout_params(&runtime.config);
        runtime.publish_layouts(&runtime.screens.clone(), &params);
        runtime
    }

    #[test]
    fn swipe_frames_track_the_fingers_without_a_spring() {
        let mut runtime = swipe_runtime();
        let before = runtime.last_on_screen[&910_002].x;
        assert!(!runtime.drive_swipe(4, 0.0, 0.0, "began").updated);
        // The first frame arms the swipe without jumping the strip.
        let arming = runtime.drive_swipe(4, -0.02, 0.0, "moved");
        assert!(arming.started && !arming.updated);
        let notice = runtime.drive_swipe(4, -0.1, 0.0, "moved");
        assert!(notice.updated);
        let scroll = runtime.strip("main").expect("strip").scroll_x;
        assert!(scroll > 100.0, "scroll {scroll}");
        let shown = runtime.last_on_screen[&910_002].x;
        assert!(runtime.motions[&910_002].settled());
        assert!((shown - (before - scroll)).abs() < 1.0, "x {shown} before {before} scroll {scroll}");
        assert!(!runtime.is_animating());
    }

    #[test]
    fn tracking_defers_size_writes_until_release() {
        let mut runtime = swipe_runtime();
        runtime.strip_mut("main").expect("strip").columns[0].width_ratio = 0.4;
        runtime.drive_swipe(4, 0.0, 0.0, "began");
        runtime.drive_swipe(4, -0.02, 0.0, "moved");
        runtime.drive_swipe(4, -0.05, 0.0, "moved");
        assert!(runtime.motions[&910_001].size_due, "size write should be deferred while tracking");
        runtime.drive_swipe(4, 0.0, 0.0, "ended");
        let mut frames = 0;
        while runtime.is_animating() && frames < 600 {
            runtime.step_animation(1.0 / 120.0);
            frames += 1;
        }
        assert!(!runtime.motions[&910_001].size_due, "size write should flush after release");
    }

    #[test]
    fn a_fling_hands_its_velocity_to_the_snap_spring() {
        let mut runtime = swipe_runtime();
        runtime.drive_swipe(4, 0.0, 0.0, "began");
        runtime.drive_swipe(4, -0.05, 0.0, "moved");
        runtime.drive_swipe(4, -0.05, 0.0, "moved");
        let notice = runtime.drive_swipe(4, 0.0, 0.0, "ended");
        assert!(notice.updated);
        let view = runtime.views[&runtime.key_of("main")];
        assert!(view.v > 0.0 && view.v <= MAX_FLING, "velocity {}", view.v);
        assert!(runtime.is_animating());
    }

    #[test]
    fn a_window_that_keeps_its_own_width_gets_a_column_that_fits() {
        let mut runtime = swipe_runtime();
        let params = layout_params(&runtime.config);
        let first = runtime.last_on_screen[&910_001];
        let listed = vec![ListedWindow {
            id: 910_001,
            pid: 424_243,
            title: "Swipe 0".into(),
            owner: "Swipe".into(),
            bounds: Frame { x: first.x, y: first.y, width: 1200.0, height: first.height },
            space: 0,
        }];
        runtime.check_sizes(&listed);
        assert!(runtime.size_misses.contains_key(&910_001));
        runtime.check_sizes(&listed);
        runtime.publish_layouts(&runtime.screens.clone(), &params);
        while runtime.is_animating() {
            runtime.step_animation(1.0 / 120.0);
        }
        let wide = runtime.last_on_screen[&910_001];
        assert_eq!(wide.width, 1200.0);
        let next = runtime.motions[&910_002].x.target - runtime.motions[&910_002].offset;
        assert!((next - (wide.x + 1200.0 + params.gap_inner)).abs() < 0.5, "next {next} wide {wide:?}");
    }

    #[test]
    fn a_swipe_that_goes_quiet_snaps_and_resumes() {
        let mut runtime = swipe_runtime();
        let params = layout_params(&runtime.config);
        let visible = runtime.viewport_of("main");
        runtime.drive_swipe(4, 0.0, 0.0, "began");
        runtime.drive_swipe(4, -0.1, 0.0, "moved");
        runtime.swipe_at = Some(Instant::now() - Duration::from_secs(1));
        runtime.expire_swipe();
        assert!(!runtime.gesture.is_open());
        assert!(runtime.swipe_deadline().is_none());
        let strip = runtime.strip("main").expect("strip");
        let scroll = strip.scroll_x;
        let spans = column_spans(strip, &params, visible);
        let inner = strip_core::scroll_width(strip, &params, visible);
        let (snapped, _) = strip_core::snap_column(scroll, &spans, inner, false);
        assert!((scroll - snapped).abs() < 1e-6, "scroll {scroll} is not a column snap");
        let notice = runtime.drive_swipe(4, -0.1, 0.0, "moved");
        assert!(runtime.gesture.is_open());
        assert!(!notice.updated || runtime.strip("main").expect("strip").scroll_x != scroll);
    }

    #[test]
    fn a_keyboard_scroll_moves_the_strip_as_one_piece() {
        let mut runtime = swipe_runtime();
        let params = layout_params(&runtime.config);
        let first = runtime.last_on_screen[&910_001].x;
        let col_w = 852.0;
        let step = col_w + params.gap_inner;
        let target_scroll = 2.0 * step;
        runtime.strip_mut("main").expect("strip").scroll_x = target_scroll;
        runtime.publish_layouts(&runtime.screens.clone(), &params);
        assert!(runtime.is_animating());
        // The column that is leaving is still where it was on the first frame.
        assert!(runtime.motions[&910_001].park.is_none());
        assert!((runtime.last_on_screen[&910_001].x - first).abs() < 0.5);
        let mut frames = 0;
        let mut checked = 0;
        while runtime.is_animating() && frames < 600 {
            runtime.step_animation(1.0 / 120.0);
            frames += 1;
            // Every fully visible column sits at its strip place minus the shared scroll.
            let scroll = runtime.views[&runtime.key_of("main")].x;
            for (index, id) in [910_001u64, 910_002, 910_003, 910_004].into_iter().enumerate() {
                if runtime.motions[&id].park.is_some() {
                    continue;
                }
                let frame = runtime.last_on_screen[&id];
                if frame.x >= 0.0 && frame.right() <= 1728.0 {
                    let expected = 8.0 + index as f64 * step - scroll;
                    assert!((frame.x - expected).abs() < 0.5, "frame {frames} window {id} x {} expected {expected}", frame.x);
                    checked += 1;
                }
            }
        }
        assert!(!runtime.is_animating(), "still animating after {frames} frames");
        assert!(frames > 10);
        assert!(checked > 0);
        assert!(runtime.motions[&910_001].park.is_some());
        let landed = runtime.last_on_screen[&910_003].x;
        assert!((landed - (8.0 + 2.0 * step - target_scroll)).abs() < 0.5, "landed {landed}");
    }

    #[test]
    fn a_column_scrolled_into_view_starts_at_its_strip_place() {
        let mut runtime = swipe_runtime();
        let params = layout_params(&runtime.config);
        let step = 852.0 + params.gap_inner;
        assert!(runtime.motions[&910_003].park.is_some());
        runtime.strip_mut("main").expect("strip").scroll_x = step;
        runtime.publish_layouts(&runtime.screens.clone(), &params);
        runtime.step_animation(1.0 / 120.0);
        let scroll = runtime.views[&runtime.key_of("main")].x;
        assert!(scroll > 0.0 && scroll < step, "scroll {scroll}");
        if let Some(entered) = runtime.last_on_screen.get(&910_003).filter(|_| runtime.motions[&910_003].park.is_none()) {
            let expected = 8.0 + 2.0 * step - scroll;
            assert!((entered.x - expected).abs() < 6.0, "x {} expected {expected}", entered.x);
        }
        while runtime.is_animating() {
            runtime.step_animation(1.0 / 120.0);
        }
        let entered = runtime.last_on_screen[&910_003];
        assert!(runtime.motions[&910_003].park.is_none());
        assert!((entered.x - (8.0 + 2.0 * step - step)).abs() < 0.5, "x {}", entered.x);
    }

    #[test]
    fn a_hand_resized_window_keeps_its_width_and_the_strip_reflows() {
        let mut runtime = swipe_runtime();
        let params = layout_params(&runtime.config);
        let first = runtime.last_on_screen[&910_001];
        runtime.pool.with_api(|api| {
            api.frames.insert(910_001, Frame { width: 1100.0, ..first });
        });
        runtime.note_pointer(1, first.right() + 1.0, first.y + 300.0);
        runtime.note_pointer(2, first.x + 1100.0, first.y + 300.0);
        let ratio = runtime.strip("main").expect("strip").columns[0].width_ratio;
        let vw = 1728.0 - 2.0 * params.gap_outer;
        assert!((ratio - (1100.0 + params.gap_inner) / (vw + params.gap_inner)).abs() < 1e-6, "ratio {ratio}");
        while runtime.is_animating() {
            runtime.step_animation(1.0 / 120.0);
        }
        let scroll = runtime.strip("main").expect("strip").scroll_x;
        let second = runtime.last_on_screen[&910_002];
        let expected = first.x + 1100.0 + params.gap_inner - scroll;
        assert!((second.x - expected).abs() < 0.5, "x {} expected {expected}", second.x);
        assert!((runtime.motions[&910_001].width - 1100.0).abs() < 0.5);
    }

    #[test]
    fn a_click_inside_a_window_changes_no_width() {
        let mut runtime = swipe_runtime();
        let first = runtime.last_on_screen[&910_001];
        runtime.note_pointer(1, first.x + 100.0, first.y + 300.0);
        runtime.note_pointer(2, first.x + 101.0, first.y + 300.0);
        assert_eq!(runtime.strip("main").expect("strip").columns[0].width_ratio, 0.5);
    }

    #[test]
    fn state_changes_are_taken_once() {
        let runtime = swipe_runtime();
        runtime.publish_state(&runtime.screens);
        let version = runtime.state_version();
        assert!(runtime.take_state_change().is_some());
        assert!(runtime.take_state_change().is_none());
        runtime.publish_state(&runtime.screens);
        assert_eq!(runtime.state_version(), version);
        assert!(runtime.take_state_change().is_none());
    }

    #[test]
    fn off_space_remote_token_inserts_the_window() {
        let mut runtime = Runtime::blank(true);
        runtime.screens = vec![screen(1728.0)];
        let id = 900_002u64;
        runtime.pool.with_api(|api| api.remote_match = Some(id));
        let before = crate::private_api::REMOTE_TOKEN_CALLS.load(std::sync::atomic::Ordering::Relaxed);
        let window = ListedWindow {
            id,
            pid: 99999,
            title: "Other Space".into(),
            owner: "Other".into(),
            bounds: Frame { x: 100.0, y: 100.0, width: 400.0, height: 300.0 },
            space: 99,
        };
        let params = layout_params(&runtime.config);
        runtime.place_off_space(std::slice::from_ref(&window), 0, &params);
        let after = crate::private_api::REMOTE_TOKEN_CALLS.load(std::sync::atomic::Ordering::Relaxed);
        assert!(after > before);
        assert!(runtime.contains_id(id));
        let key = StripKey { display: "main".into(), space: 99 };
        assert!(runtime.strips.contains_key(&key));
    }
}


