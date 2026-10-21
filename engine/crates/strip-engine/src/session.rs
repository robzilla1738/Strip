//! One strip per display. Publishes frames through the writer pool.

use std::collections::{HashMap, HashSet};
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
    only_owner: Option<String>,
    managing: bool,
}

struct Meta {
    pid: i32,
    bundle_id: String,
    title: String,
    owner: String,
    display: String,
}

impl Runtime {
    pub fn start(config: Config, private_api: &PrivateApi, state: Arc<Mutex<String>>) -> Self {
        ax::prepare_app();
        let only_owner = std::env::var("STRIP_ONLY_OWNER").ok().filter(|value| !value.is_empty());
        Self {
            config,
            pool: WriterPool::new(AxApi::new(private_api), Arc::new(SystemClock) as Arc<dyn Clock>),
            strips: HashMap::new(),
            meta: HashMap::new(),
            last_on_screen: HashMap::new(),
            state,
            only_owner,
            managing: true,
        }
    }

    pub fn tick(&mut self) {
        if !self.managing {
            self.publish_state(&[]);
            return;
        }
        let displays = ax::displays(self.config.layout.dock_aware);
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
        self.publish_state(&displays);
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


