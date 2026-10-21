use crate::model::{DisplayMode, Strip, Window, WindowId};
use crate::params::{DisplayFrame, Frame, LayoutParams, Viewport};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    Visible,
    Partial,
    Hidden,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    BottomLeft,
    BottomRight,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub window: WindowId,
    pub frame: Frame,
    pub visibility: Visibility,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SizeWatch {
    pub target: Option<(f64, f64)>,
    pub mismatches: u8,
}

impl SizeWatch {
    pub fn new() -> Self {
        Self {
            target: None,
            mismatches: 0,
        }
    }
}

impl Default for SizeWatch {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns `Some(actual)` once two consecutive writes of the same target
/// both differ from the read-back by more than 2 pt.
pub fn observe_size(watch: &mut SizeWatch, target: (f64, f64), actual: (f64, f64)) -> Option<(f64, f64)> {
    let differs = (actual.0 - target.0).abs() > 2.0 || (actual.1 - target.1).abs() > 2.0;
    if !differs {
        watch.target = Some(target);
        watch.mismatches = 0;
        return None;
    }
    if watch.target == Some(target) {
        watch.mismatches = watch.mismatches.saturating_add(1);
    } else {
        watch.target = Some(target);
        watch.mismatches = 1;
    }
    if watch.mismatches >= 2 {
        Some(actual)
    } else {
        None
    }
}

pub fn sliver_used(gap_outer: f64, configured: f64) -> f64 {
    if gap_outer == 0.0 { 1.0 } else { configured }
}

pub fn parked_corner(
    side: Side,
    neighbour_left: bool,
    neighbour_right: bool,
    gap_outer: f64,
    sliver: f64,
) -> (Corner, f64) {
    if neighbour_left && neighbour_right {
        return (Corner::BottomLeft, 1.0);
    }
    let mut corner = match side {
        Side::Left => Corner::BottomLeft,
        Side::Right => Corner::BottomRight,
    };
    let blocked = match side {
        Side::Left => neighbour_left,
        Side::Right => neighbour_right,
    };
    if blocked {
        corner = match corner {
            Corner::BottomLeft => Corner::BottomRight,
            Corner::BottomRight => Corner::BottomLeft,
        };
    }
    (corner, sliver_used(gap_outer, sliver))
}

/// A window that keeps a wider size than its column gets a column that fits
/// it, so the strip lays out around it instead of overlapping the neighbour.
pub fn column_width(column: &crate::model::Column, viewport_width: f64) -> f64 {
    let base = column.width_ratio * viewport_width;
    column
        .windows
        .iter()
        .filter_map(|window| window.size_override.map(|(width, _)| width))
        .fold(base, f64::max)
}

/// Width the columns scroll through: the viewport less outer gaps and pins.
pub fn scroll_width(strip: &Strip, params: &LayoutParams, visible: Viewport) -> f64 {
    metrics(strip, params, visible).vw
}

pub(crate) struct Metrics {
    pub vx: f64,
    pub vy: f64,
    pub vw: f64,
    pub vh: f64,
    pub widths: Vec<f64>,
    pub prefixes: Vec<f64>,
    pub content: f64,
}

pub(crate) fn metrics(strip: &Strip, params: &LayoutParams, visible: Viewport) -> Metrics {
    let inner_w = (visible.width - 2.0 * params.gap_outer).max(0.0);
    let inner_h = (visible.height - 2.0 * params.gap_outer).max(0.0);
    let mut vx = visible.x + params.gap_outer;
    let vy = visible.y + params.gap_outer;
    let mut vw = inner_w;
    if let Some(pin) = &strip.pinned_left {
        let width = column_width(&pin.column, visible.width);
        vx += width + params.gap_inner;
        vw = (vw - width - params.gap_inner).max(0.0);
    }
    if let Some(pin) = &strip.pinned_right {
        let width = column_width(&pin.column, visible.width);
        vw = (vw - width - params.gap_inner).max(0.0);
    }
    let widths: Vec<f64> = strip
        .columns
        .iter()
        .map(|column| column_width(column, visible.width))
        .collect();
    let mut prefixes = Vec::with_capacity(widths.len());
    let mut cursor = 0.0;
    for (index, width) in widths.iter().enumerate() {
        prefixes.push(cursor);
        cursor += width;
        if index + 1 != widths.len() {
            cursor += params.gap_inner;
        }
    }
    Metrics {
        vx,
        vy,
        vw,
        vh: inner_h,
        widths,
        prefixes,
        content: cursor,
    }
}

pub fn layout(
    strip: &Strip,
    visible: Viewport,
    display: DisplayFrame,
    params: &LayoutParams,
) -> Vec<Placement> {
    let geom = metrics(strip, params, visible);
    let view = Frame {
        x: geom.vx,
        y: geom.vy,
        width: geom.vw,
        height: geom.vh,
    };
    let mut placed = Vec::new();
    if let Some(pin) = &strip.pinned_left {
        let frame = pin_frame(Side::Left, column_width(&pin.column, visible.width), visible, params);
        place_column(&mut placed, &pin.column, frame, Visibility::Visible, Side::Left, display, params);
    }
    for (index, column) in strip.columns.iter().enumerate() {
        let natural = Frame {
            x: geom.vx + geom.prefixes[index] - strip.scroll_x,
            y: geom.vy,
            width: geom.widths[index],
            height: geom.vh,
        };
        let visibility = classify(natural, view);
        let side = hidden_side(natural, view);
        let frame = match visibility {
            Visibility::Visible => natural,
            Visibility::Partial => clip_partial(natural, display, sliver_used(params.gap_outer, params.sliver)),
            Visibility::Hidden => natural,
        };
        place_column(&mut placed, column, frame, visibility, side, display, params);
    }
    if let Some(pin) = &strip.pinned_right {
        let frame = pin_frame(Side::Right, column_width(&pin.column, visible.width), visible, params);
        place_column(&mut placed, &pin.column, frame, Visibility::Visible, Side::Right, display, params);
    }
    placed
}

fn place_column(
    out: &mut Vec<Placement>,
    column: &crate::model::Column,
    frame: Frame,
    visibility: Visibility,
    side: Side,
    display: DisplayFrame,
    params: &LayoutParams,
) {
    if visibility == Visibility::Hidden {
        for window in &column.windows {
            out.push(parked_placement(window, frame, side, display, params));
        }
        return;
    }
    if column.mode == DisplayMode::Tabbed {
        for (index, window) in column.windows.iter().enumerate() {
            if index == column.active_tab {
                out.push(Placement {
                    window: window.id,
                    frame: sized_frame(frame, window, params),
                    visibility,
                });
            } else {
                out.push(parked_placement(window, frame, side, display, params));
            }
        }
        return;
    }
    for (index, window_frame) in stack_frames(frame, &column.windows, params).into_iter().enumerate() {
        out.push(Placement {
            window: column.windows[index].id,
            frame: window_frame,
            visibility,
        });
    }
}

fn parked_placement(
    window: &Window,
    natural: Frame,
    side: Side,
    display: DisplayFrame,
    params: &LayoutParams,
) -> Placement {
    let (corner, sliver) = parked_corner(
        side,
        display.neighbour_left,
        display.neighbour_right,
        params.gap_outer,
        params.sliver,
    );
    let width = window.size_override.map(|(w, _)| w).unwrap_or(natural.width).max(sliver);
    let height = window.size_override.map(|(_, h)| h).unwrap_or(natural.height).max(sliver);
    // Keep the window on the strip's row. Parking at the bottom corner makes
    // macOS clamp the title bar back on screen, so the full window jumps down
    // and piles on top of the others.
    let y = natural.y;
    let x = match corner {
        Corner::BottomRight => display.max_x - sliver,
        Corner::BottomLeft => display.min_x - width + sliver,
    };
    Placement {
        window: window.id,
        frame: Frame { x, y, width, height },
        visibility: Visibility::Hidden,
    }
}

fn pin_frame(side: Side, width: f64, visible: Viewport, params: &LayoutParams) -> Frame {
    let height = (visible.height - 2.0 * params.gap_outer).max(0.0);
    let y = visible.y + params.gap_outer;
    let x = match side {
        Side::Left => visible.x + params.gap_outer,
        Side::Right => visible.x + visible.width - params.gap_outer - width,
    };
    Frame { x, y, width, height }
}

fn stack_frames(column: Frame, windows: &[Window], params: &LayoutParams) -> Vec<Frame> {
    if windows.is_empty() {
        return Vec::new();
    }
    let gaps = params.gap_inner * (windows.len() - 1) as f64;
    let available = (column.height - gaps).max(0.0);
    let sum = windows.iter().map(|window| window.height_ratio).sum::<f64>().max(1e-6);
    let mut y = column.y;
    let mut frames = Vec::with_capacity(windows.len());
    for window in windows {
        let frame = sized_frame(
            Frame {
                x: column.x,
                y,
                width: column.width,
                height: available * window.height_ratio / sum,
            },
            window,
            params,
        );
        y += frame.height + params.gap_inner;
        frames.push(frame);
    }
    frames
}

fn sized_frame(base: Frame, window: &Window, params: &LayoutParams) -> Frame {
    let mut frame = base;
    if params.keep_original_height {
        frame.height = frame.height.min(window.original_height);
    }
    if let Some((width, height)) = window.size_override {
        frame.width = width;
        frame.height = height;
    }
    frame
}

fn classify(frame: Frame, view: Frame) -> Visibility {
    if frame.width <= 0.0 || frame.height <= 0.0 || view.width <= 0.0 || view.height <= 0.0 {
        return Visibility::Hidden;
    }
    if contains(view, frame) {
        Visibility::Visible
    } else if intersects(view, frame) {
        Visibility::Partial
    } else {
        Visibility::Hidden
    }
}

fn contains(outer: Frame, inner: Frame) -> bool {
    inner.x >= outer.x - 1e-6
        && inner.y >= outer.y - 1e-6
        && inner.right() <= outer.right() + 1e-6
        && inner.bottom() <= outer.bottom() + 1e-6
}

fn intersects(a: Frame, b: Frame) -> bool {
    a.x < b.right() && a.right() > b.x && a.y < b.bottom() && a.bottom() > b.y
}

fn hidden_side(natural: Frame, view: Frame) -> Side {
    if natural.right() <= view.x + 0.5 || natural.x < view.x {
        Side::Left
    } else {
        Side::Right
    }
}

fn clip_partial(mut frame: Frame, display: DisplayFrame, sliver: f64) -> Frame {
    if frame.right() > display.max_x && display.max_x - frame.x < sliver {
        frame.x = display.max_x - sliver;
    }
    if frame.x < display.min_x && frame.right() - display.min_x < sliver {
        frame.x = display.min_x - frame.width + sliver;
    }
    frame
}

pub fn overlap(frame: Frame, display: DisplayFrame) -> (f64, f64) {
    let width = (frame.right().min(display.max_x) - frame.x.max(display.min_x)).max(0.0);
    let height = (frame.bottom().min(display.max_y) - frame.y.max(display.min_y)).max(0.0);
    (width, height)
}

pub fn crosses_display(frame: Frame, display: DisplayFrame) -> bool {
    frame.x < display.min_x - 1e-6
        || frame.y < display.min_y - 1e-6
        || frame.right() > display.max_x + 1e-6
        || frame.bottom() > display.max_y + 1e-6
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Strip, Window};
    use crate::params::{DisplayFrame, LayoutParams, Viewport};
    use crate::CenterMode;

    fn stage() -> (LayoutParams, Viewport, DisplayFrame) {
        let params = LayoutParams {
            gap_inner: 0.0,
            gap_outer: 0.0,
            sliver: 5.0,
            center: CenterMode::Never,
            keep_original_height: false,
            presets: vec![0.25, 0.5, 0.75],
            width_step: 0.1,
            default_column_width: 0.5,
        };
        let visible = Viewport { x: 0.0, y: 0.0, width: 1000.0, height: 800.0 };
        (params, visible, DisplayFrame::from_origin(1000.0, 800.0))
    }

    fn three(params: &LayoutParams, visible: Viewport) -> Strip {
        let mut strip = Strip::new();
        for id in 1..=3 {
            strip.insert(Window::new(id, 1), 1, None, params, visible);
        }
        strip
    }

    #[test]
    fn center_mode_never_on_overflow_always() {
        let (mut params, visible, _) = stage();
        let mut strip = three(&params, visible);
        strip.focus_index(3, &params, visible);
        assert!((strip.scroll_x - 500.0).abs() < 1e-6, "never {}", strip.scroll_x);

        params.center = CenterMode::OnOverflow;
        strip.scroll_x = 0.0;
        strip.focus = crate::model::Focus::Column(2);
        strip.reveal(&params, visible);
        assert!((strip.scroll_x - 750.0).abs() < 1e-6, "overflow {}", strip.scroll_x);

        params.center = CenterMode::Always;
        strip.scroll_x = 0.0;
        strip.reveal(&params, visible);
        assert!((strip.scroll_x - 750.0).abs() < 1e-6, "always {}", strip.scroll_x);

        let narrow = LayoutParams {
            default_column_width: 0.2,
            center: CenterMode::Never,
            ..params.clone()
        };
        let mut fitting = Strip::new();
        for id in 1..=3 {
            fitting.insert(Window::new(id, 1), 1, None, &narrow, visible);
        }
        fitting.focus_index(2, &narrow, visible);
        assert!(fitting.scroll_x.abs() < 1e-6);

        let mut on_overflow = narrow.clone();
        on_overflow.center = CenterMode::OnOverflow;
        fitting.scroll_x = 0.0;
        fitting.focus = crate::model::Focus::Column(1);
        fitting.reveal(&on_overflow, visible);
        assert!(fitting.scroll_x.abs() < 1e-6, "fits so on_overflow stays {}", fitting.scroll_x);

        let mut always = narrow.clone();
        always.center = CenterMode::Always;
        fitting.reveal(&always, visible);
        assert!((fitting.scroll_x - (-200.0)).abs() < 1e-6, "always center {}", fitting.scroll_x);
    }

    #[test]
    fn pin_swaps_occupied_side_and_unpins_to_previous_index() {
        let (params, visible, _) = stage();
        let mut strip = three(&params, visible);
        strip.focus_index(1, &params, visible);
        strip.pin_left(&params, visible);
        strip.focus_index(2, &params, visible);
        strip.pin_left(&params, visible);
        let pinned = strip.pinned_left.as_ref().unwrap();
        assert_eq!(pinned.column.windows[0].id, WindowId(3));
        assert_eq!(pinned.previous_index, 2);
        assert_eq!(strip.columns.len(), 2);
        strip.pin_left(&params, visible);
        assert!(strip.pinned_left.is_none());
        assert_eq!(strip.columns[2].windows[0].id, WindowId(3));
    }

    #[test]
    fn stack_both_directions_keep_other_widths() {
        let (params, visible, _) = stage();
        let mut strip = three(&params, visible);
        strip.columns[0].width_ratio = 0.4;
        strip.columns[1].width_ratio = 0.5;
        strip.columns[2].width_ratio = 0.6;
        strip.focus = crate::model::Focus::Column(1);
        strip.stack_left(&params, visible);
        assert_eq!(strip.columns.len(), 2);
        assert_eq!(strip.columns[0].windows.len(), 2);
        assert!((strip.columns[0].width_ratio - 0.4).abs() < 1e-9);
        assert!((strip.columns[1].width_ratio - 0.6).abs() < 1e-9);

        let mut strip = three(&params, visible);
        strip.columns[0].width_ratio = 0.4;
        strip.columns[1].width_ratio = 0.5;
        strip.columns[2].width_ratio = 0.6;
        strip.focus = crate::model::Focus::Column(1);
        strip.stack_right(&params, visible);
        assert_eq!(strip.columns[1].windows.len(), 2);
        assert!((strip.columns[0].width_ratio - 0.4).abs() < 1e-9);
        assert!((strip.columns[1].width_ratio - 0.6).abs() < 1e-9);
    }

    #[test]
    fn tabbed_hides_inactive_tabs() {
        let (params, visible, display) = stage();
        let mut strip = Strip::new();
        strip.insert(Window::new(1, 1), 1, None, &params, visible);
        strip.columns[0].windows.push(Window::new(2, 1));
        strip.columns[0].windows.push(Window::new(3, 1));
        strip.columns[0].mode = DisplayMode::Tabbed;
        strip.columns[0].active_tab = 1;
        let placed = layout(&strip, visible, display, &params);
        let by_id = |id| placed.iter().find(|item| item.window == WindowId(id)).unwrap();
        assert_eq!(by_id(2).visibility, Visibility::Visible);
        assert_eq!(by_id(1).visibility, Visibility::Hidden);
        assert_eq!(by_id(3).visibility, Visibility::Hidden);
        let (w, _) = overlap(by_id(1).frame, display);
        assert!(w <= 5.0 + 1e-6);
        assert!((by_id(1).frame.y - by_id(2).frame.y).abs() < 1e-6);
    }

    #[test]
    fn stubborn_override_after_two_identical_misses() {
        let mut watch = SizeWatch::new();
        let target = (400.0, 700.0);
        assert!(observe_size(&mut watch, target, (410.0, 700.0)).is_none());
        assert_eq!(observe_size(&mut watch, target, (410.0, 700.0)), Some((410.0, 700.0)));
        let mut exact = SizeWatch::new();
        assert!(observe_size(&mut exact, target, (402.0, 700.0)).is_none());
        assert!(observe_size(&mut exact, target, (402.0, 700.0)).is_none());
    }

    #[test]
    fn parked_corner_follows_neighbours_and_gap() {
        assert_eq!(
            parked_corner(Side::Left, false, false, 8.0, 5.0),
            (Corner::BottomLeft, 5.0)
        );
        assert_eq!(
            parked_corner(Side::Right, false, false, 8.0, 5.0),
            (Corner::BottomRight, 5.0)
        );
        assert_eq!(
            parked_corner(Side::Left, true, false, 8.0, 5.0),
            (Corner::BottomRight, 5.0)
        );
        assert_eq!(
            parked_corner(Side::Left, true, true, 8.0, 5.0),
            (Corner::BottomLeft, 1.0)
        );
        assert_eq!(
            parked_corner(Side::Right, false, false, 0.0, 5.0),
            (Corner::BottomRight, 1.0)
        );
    }

    #[test]
    fn unstack_keeps_column_width() {
        let (params, visible, _) = stage();
        let mut strip = Strip::new();
        strip.insert(Window::new(1, 1), 1, None, &params, visible);
        strip.columns[0].windows.push(Window::new(2, 1));
        strip.columns[0].width_ratio = 0.4;
        strip.columns[0].focused_window = 1;
        strip.focus = crate::model::Focus::Column(0);
        strip.unstack_right(&params, visible);
        assert_eq!(strip.columns.len(), 2);
        assert!((strip.columns[0].width_ratio - 0.4).abs() < 1e-9);
        assert!((strip.columns[1].width_ratio - 0.4).abs() < 1e-9);
        assert_eq!(strip.columns[1].windows[0].id, WindowId(2));
    }
}
