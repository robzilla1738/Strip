use crate::layout::metrics;
use crate::params::{LayoutParams, Viewport};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ColumnId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayMode {
    Stacked,
    Tabbed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    None,
    Column(usize),
    PinnedLeft,
    PinnedRight,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    pub id: WindowId,
    pub pid: i32,
    pub height_ratio: f64,
    pub original_height: f64,
    pub size_override: Option<(f64, f64)>,
}

impl Window {
    pub fn new(id: u64, pid: i32) -> Self {
        Self {
            id: WindowId(id),
            pid,
            height_ratio: 1.0,
            original_height: 600.0,
            size_override: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub id: ColumnId,
    pub windows: Vec<Window>,
    pub width_ratio: f64,
    pub mode: DisplayMode,
    pub active_tab: usize,
    pub focused_window: usize,
    pub previous_width: Option<f64>,
}

impl Column {
    fn single(id: ColumnId, window: Window, width_ratio: f64) -> Self {
        Self {
            id,
            windows: vec![window],
            width_ratio,
            mode: DisplayMode::Stacked,
            active_tab: 0,
            focused_window: 0,
            previous_width: None,
        }
    }

    pub fn clear_overrides(&mut self) {
        for window in &mut self.windows {
            window.size_override = None;
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pinned {
    pub column: Column,
    pub previous_index: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Strip {
    pub columns: Vec<Column>,
    pub scroll_x: f64,
    pub focus: Focus,
    pub pinned_left: Option<Pinned>,
    pub pinned_right: Option<Pinned>,
    next_column: u64,
}

impl Strip {
    pub fn new() -> Self {
        Self {
            columns: Vec::new(),
            scroll_x: 0.0,
            focus: Focus::None,
            pinned_left: None,
            pinned_right: None,
            next_column: 1,
        }
    }

    pub fn contains(&self, id: WindowId) -> bool {
        self.columns
            .iter()
            .chain(self.pinned_left.iter().map(|pin| &pin.column))
            .chain(self.pinned_right.iter().map(|pin| &pin.column))
            .any(|column| column.windows.iter().any(|window| window.id == id))
    }

    pub fn width_ratios(&self) -> Vec<(ColumnId, f64)> {
        self.columns
            .iter()
            .chain(self.pinned_left.iter().map(|pin| &pin.column))
            .chain(self.pinned_right.iter().map(|pin| &pin.column))
            .map(|column| (column.id, column.width_ratio))
            .collect()
    }

    pub fn insert(
        &mut self,
        mut window: Window,
        frontmost_pid: i32,
        slot: Option<u32>,
        params: &LayoutParams,
        visible: Viewport,
    ) {
        if self.contains(window.id) {
            return;
        }
        window.height_ratio = 1.0;
        let index = match slot {
            Some(slot) => (slot.saturating_sub(1) as usize).min(self.columns.len()),
            None => match self.focus {
                Focus::Column(index) => (index + 1).min(self.columns.len()),
                Focus::PinnedLeft => 0,
                Focus::PinnedRight | Focus::None => self.columns.len(),
            },
        };
        let pid = window.pid;
        let column = Column::single(self.alloc_column(), window, params.default_column_width);
        self.columns.insert(index, column);
        if let Focus::Column(focus) = self.focus {
            if index <= focus {
                self.focus = Focus::Column(focus + 1);
            }
        }
        if pid != frontmost_pid {
            return;
        }
        self.focus = Focus::Column(index);
        self.reveal(params, visible);
    }

    pub fn remove(&mut self, id: WindowId, params: &LayoutParams, visible: Viewport) {
        if self.take_from_pin(true, id) || self.take_from_pin(false, id) {
            self.reveal(params, visible);
            return;
        }
        let Some(pos) = self.column_index(id) else {
            return;
        };
        remove_window(&mut self.columns[pos], id);
        if self.columns[pos].windows.is_empty() {
            self.columns.remove(pos);
            self.retarget_after_column_removed(pos);
        }
        self.reveal(params, visible);
    }

    pub fn focus_left(&mut self, params: &LayoutParams, visible: Viewport) {
        self.step_focus(-1, params, visible);
    }

    pub fn focus_right(&mut self, params: &LayoutParams, visible: Viewport) {
        self.step_focus(1, params, visible);
    }

    pub fn focus_first(&mut self, params: &LayoutParams, visible: Viewport) {
        if let Some(focus) = self.focus_order().first().copied() {
            self.focus = focus;
            self.reveal(params, visible);
        }
    }

    pub fn focus_last(&mut self, params: &LayoutParams, visible: Viewport) {
        if let Some(focus) = self.focus_order().last().copied() {
            self.focus = focus;
            self.reveal(params, visible);
        }
    }

    /// `n` is 1-based. `9` focuses the last scroll column.
    pub fn focus_index(&mut self, n: u32, params: &LayoutParams, visible: Viewport) {
        if self.columns.is_empty() {
            return;
        }
        let index = column_number(n, self.columns.len());
        self.focus = Focus::Column(index);
        self.reveal(params, visible);
    }

    pub fn focus_up(&mut self, params: &LayoutParams, visible: Viewport) {
        self.step_window(-1);
        self.reveal(params, visible);
    }

    pub fn focus_down(&mut self, params: &LayoutParams, visible: Viewport) {
        self.step_window(1);
        self.reveal(params, visible);
    }

    pub fn move_left(&mut self, params: &LayoutParams, visible: Viewport) {
        let Focus::Column(index) = self.focus else {
            return;
        };
        if index == 0 {
            return;
        }
        self.columns.swap(index - 1, index);
        self.focus = Focus::Column(index - 1);
        self.reveal(params, visible);
    }

    pub fn move_right(&mut self, params: &LayoutParams, visible: Viewport) {
        let Focus::Column(index) = self.focus else {
            return;
        };
        if index + 1 >= self.columns.len() {
            return;
        }
        self.columns.swap(index, index + 1);
        self.focus = Focus::Column(index + 1);
        self.reveal(params, visible);
    }

    pub fn move_to_index(&mut self, n: u32, params: &LayoutParams, visible: Viewport) {
        let Focus::Column(from) = self.focus else {
            return;
        };
        if self.columns.is_empty() {
            return;
        }
        let to = column_number(n, self.columns.len());
        self.move_column(from, to);
        self.focus = Focus::Column(to.min(self.columns.len().saturating_sub(1)));
        self.reveal(params, visible);
    }

    pub fn move_column(&mut self, from: usize, to: usize) {
        if from >= self.columns.len() || to >= self.columns.len() || from == to {
            return;
        }
        let column = self.columns.remove(from);
        let dest = to.min(self.columns.len());
        self.columns.insert(dest, column);
        if let Focus::Column(focus) = self.focus {
            self.focus = Focus::Column(remap_index(focus, from, dest));
        }
    }

    pub fn move_up(&mut self, params: &LayoutParams, visible: Viewport) {
        self.reorder_window(-1);
        self.reveal(params, visible);
    }

    pub fn move_down(&mut self, params: &LayoutParams, visible: Viewport) {
        self.reorder_window(1);
        self.reveal(params, visible);
    }

    pub fn cycle_width(&mut self, params: &LayoutParams, visible: Viewport) {
        let Some(current) = self.focused_ratio() else {
            return;
        };
        let next = next_preset(current, &params.presets);
        self.set_focused_ratio(next);
        self.reveal(params, visible);
    }

    pub fn wider(&mut self, params: &LayoutParams, visible: Viewport) {
        self.nudge_width(params.width_step, params, visible);
    }

    pub fn narrower(&mut self, params: &LayoutParams, visible: Viewport) {
        self.nudge_width(-params.width_step, params, visible);
    }

    pub fn full_width(&mut self, params: &LayoutParams, visible: Viewport) {
        let Some(column) = self.focused_column_mut() else {
            return;
        };
        if (column.width_ratio - 1.0).abs() < 1e-6 {
            column.width_ratio = column.previous_width.unwrap_or(params.default_column_width);
            column.previous_width = None;
        } else {
            column.previous_width = Some(column.width_ratio);
            column.width_ratio = 1.0;
        }
        column.clear_overrides();
        self.reveal(params, visible);
    }

    pub fn center(&mut self, params: &LayoutParams, visible: Viewport) {
        let geom = metrics(self, params, visible);
        let Focus::Column(index) = self.focus else {
            return;
        };
        let Some(prefix) = geom.prefixes.get(index) else {
            return;
        };
        let Some(width) = geom.widths.get(index) else {
            return;
        };
        if geom.vw <= 0.0 {
            return;
        }
        self.scroll_x = prefix + width / 2.0 - geom.vw / 2.0;
    }

    pub fn taller(&mut self, params: &LayoutParams, visible: Viewport) {
        self.nudge_height(0.10, params, visible);
    }

    pub fn shorter(&mut self, params: &LayoutParams, visible: Viewport) {
        self.nudge_height(-0.10, params, visible);
    }

    pub fn equalize_heights(&mut self, params: &LayoutParams, visible: Viewport) {
        let Some(column) = self.focused_column_mut() else {
            return;
        };
        equalize(&mut column.windows);
        column.clear_overrides();
        self.reveal(params, visible);
    }

    pub fn stack_left(&mut self, params: &LayoutParams, visible: Viewport) {
        self.stack_toward(-1, params, visible);
    }

    pub fn stack_right(&mut self, params: &LayoutParams, visible: Viewport) {
        self.stack_toward(1, params, visible);
    }

    pub fn unstack_left(&mut self, params: &LayoutParams, visible: Viewport) {
        self.unstack_toward(-1, params, visible);
    }

    pub fn unstack_right(&mut self, params: &LayoutParams, visible: Viewport) {
        self.unstack_toward(1, params, visible);
    }

    pub fn toggle_tabbed(&mut self) {
        let Some(column) = self.focused_column_mut() else {
            return;
        };
        column.mode = match column.mode {
            DisplayMode::Stacked => {
                column.active_tab = column.focused_window.min(column.windows.len().saturating_sub(1));
                DisplayMode::Tabbed
            }
            DisplayMode::Tabbed => DisplayMode::Stacked,
        };
    }

    pub fn pin_left(&mut self, params: &LayoutParams, visible: Viewport) {
        if self.focus == Focus::PinnedLeft {
            self.unpin(true, params, visible);
            return;
        }
        self.pin(true, params, visible);
    }

    pub fn pin_right(&mut self, params: &LayoutParams, visible: Viewport) {
        if self.focus == Focus::PinnedRight {
            self.unpin(false, params, visible);
            return;
        }
        self.pin(false, params, visible);
    }

    pub fn reveal(&mut self, params: &LayoutParams, visible: Viewport) {
        let geom = metrics(self, params, visible);
        let Focus::Column(index) = self.focus else {
            return;
        };
        let Some(prefix) = geom.prefixes.get(index).copied() else {
            return;
        };
        let Some(width) = geom.widths.get(index).copied() else {
            return;
        };
        if geom.vw <= 0.0 {
            return;
        }
        if width >= geom.vw {
            self.scroll_x = prefix;
        } else {
            let left = prefix - self.scroll_x;
            let right = left + width;
            if left < 0.0 {
                self.scroll_x = prefix;
            } else if right > geom.vw {
                self.scroll_x = prefix + width - geom.vw;
            }
        }
        let overflow = geom.content > geom.vw + 1e-6;
        if matches!(params.center, crate::CenterMode::Always)
            || (matches!(params.center, crate::CenterMode::OnOverflow) && overflow)
        {
            self.scroll_x = prefix + width / 2.0 - geom.vw / 2.0;
        }
    }

    fn alloc_column(&mut self) -> ColumnId {
        let id = ColumnId(self.next_column);
        self.next_column += 1;
        id
    }

    fn column_index(&self, id: WindowId) -> Option<usize> {
        self.columns
            .iter()
            .position(|column| column.windows.iter().any(|window| window.id == id))
    }

    fn focus_order(&self) -> Vec<Focus> {
        let mut order = Vec::new();
        if self.pinned_left.is_some() {
            order.push(Focus::PinnedLeft);
        }
        for index in 0..self.columns.len() {
            order.push(Focus::Column(index));
        }
        if self.pinned_right.is_some() {
            order.push(Focus::PinnedRight);
        }
        order
    }

    fn step_focus(&mut self, delta: isize, params: &LayoutParams, visible: Viewport) {
        let order = self.focus_order();
        if order.is_empty() {
            return;
        }
        let current = order.iter().position(|focus| *focus == self.focus).unwrap_or(0);
        let next = current as isize + delta;
        if next < 0 || next as usize >= order.len() {
            return;
        }
        self.focus = order[next as usize];
        self.reveal(params, visible);
    }

    fn step_window(&mut self, delta: isize) {
        let Some(column) = self.focused_column_mut() else {
            return;
        };
        if column.windows.is_empty() {
            return;
        }
        if column.mode == DisplayMode::Tabbed {
            let len = column.windows.len() as isize;
            let current = column.active_tab as isize;
            let next = (current + delta).rem_euclid(len) as usize;
            column.active_tab = next;
            column.focused_window = next;
            return;
        }
        let next = column.focused_window as isize + delta;
        if next >= 0 && (next as usize) < column.windows.len() {
            column.focused_window = next as usize;
        }
    }

    fn reorder_window(&mut self, delta: isize) {
        let Some(column) = self.focused_column_mut() else {
            return;
        };
        if column.windows.len() < 2 {
            return;
        }
        let from = column.focused_window.min(column.windows.len() - 1);
        let to = from as isize + delta;
        if to < 0 || to as usize >= column.windows.len() {
            return;
        }
        column.windows.swap(from, to as usize);
        column.focused_window = to as usize;
    }

    fn nudge_width(&mut self, delta: f64, params: &LayoutParams, visible: Viewport) {
        let Some(current) = self.focused_ratio() else {
            return;
        };
        self.set_focused_ratio((current + delta).clamp(0.1, 1.0));
        self.reveal(params, visible);
    }

    fn focused_ratio(&self) -> Option<f64> {
        self.focused_column().map(|column| column.width_ratio)
    }

    fn set_focused_ratio(&mut self, ratio: f64) {
        if let Some(column) = self.focused_column_mut() {
            column.width_ratio = ratio;
            column.clear_overrides();
        }
    }

    fn nudge_height(&mut self, delta: f64, params: &LayoutParams, visible: Viewport) {
        let Some(column) = self.focused_column_mut() else {
            return;
        };
        let index = column.focused_window.min(column.windows.len().saturating_sub(1));
        let mut ratios: Vec<f64> = column.windows.iter().map(|window| window.height_ratio).collect();
        nudge_ratios(&mut ratios, index, delta);
        for (window, ratio) in column.windows.iter_mut().zip(ratios) {
            window.height_ratio = ratio;
            window.size_override = None;
        }
        self.reveal(params, visible);
    }

    fn stack_toward(&mut self, direction: isize, params: &LayoutParams, visible: Viewport) {
        let Focus::Column(index) = self.focus else {
            return;
        };
        let Some(target) = index.checked_add_signed(direction) else {
            return;
        };
        if target >= self.columns.len() {
            return;
        }
        let Some(window) = self.take_focused_window(index) else {
            return;
        };
        push_share(&mut self.columns[target].windows, window);
        let last = self.columns[target].windows.len() - 1;
        self.columns[target].focused_window = last;
        if self.columns[index].windows.is_empty() {
            self.columns.remove(index);
            let landed = if direction < 0 { target } else { index };
            self.focus = Focus::Column(landed);
        } else {
            self.focus = Focus::Column(target);
        }
        self.reveal(params, visible);
    }

    fn unstack_toward(&mut self, direction: isize, params: &LayoutParams, visible: Viewport) {
        let Focus::Column(index) = self.focus else {
            return;
        };
        if self.columns.get(index).is_none_or(|column| column.windows.len() < 2) {
            return;
        }
        let width = self.columns[index].width_ratio;
        let Some(window) = self.take_focused_window(index) else {
            return;
        };
        let column = Column::single(self.alloc_column(), window, width);
        let insert_at = if direction < 0 { index } else { index + 1 };
        self.columns.insert(insert_at, column);
        self.focus = Focus::Column(insert_at);
        self.reveal(params, visible);
    }

    fn take_focused_window(&mut self, index: usize) -> Option<Window> {
        let column = self.columns.get_mut(index)?;
        if column.windows.is_empty() {
            return None;
        }
        let at = column.focused_window.min(column.windows.len() - 1);
        let window = column.windows.remove(at);
        if column.windows.is_empty() {
            column.focused_window = 0;
            column.active_tab = 0;
        } else {
            rescale(&mut column.windows);
            if column.focused_window >= column.windows.len() {
                column.focused_window = column.windows.len() - 1;
            }
            if column.active_tab >= column.windows.len() {
                column.active_tab = column.windows.len() - 1;
            }
        }
        Some(window)
    }

    fn pin(&mut self, left: bool, params: &LayoutParams, visible: Viewport) {
        let Focus::Column(mut index) = self.focus else {
            return;
        };
        let Some(id) = self.columns.get(index).map(|column| column.id) else {
            return;
        };
        if left {
            if let Some(old) = self.pinned_left.take() {
                let at = old.previous_index.min(self.columns.len());
                self.columns.insert(at, old.column);
            }
        } else if let Some(old) = self.pinned_right.take() {
            let at = old.previous_index.min(self.columns.len());
            self.columns.insert(at, old.column);
        }
        index = self
            .columns
            .iter()
            .position(|column| column.id == id)
            .unwrap_or(index.min(self.columns.len().saturating_sub(1)));
        if index >= self.columns.len() {
            return;
        }
        let column = self.columns.remove(index);
        let pinned = Pinned {
            column,
            previous_index: index,
        };
        if left {
            self.pinned_left = Some(pinned);
            self.focus = Focus::PinnedLeft;
        } else {
            self.pinned_right = Some(pinned);
            self.focus = Focus::PinnedRight;
        }
        self.reveal(params, visible);
    }

    fn unpin(&mut self, left: bool, params: &LayoutParams, visible: Viewport) {
        let pinned = if left {
            self.pinned_left.take()
        } else {
            self.pinned_right.take()
        };
        let Some(pinned) = pinned else {
            return;
        };
        let at = pinned.previous_index.min(self.columns.len());
        self.columns.insert(at, pinned.column);
        self.focus = Focus::Column(at);
        self.reveal(params, visible);
    }

    fn take_from_pin(&mut self, left: bool, id: WindowId) -> bool {
        let pin = if left {
            self.pinned_left.as_mut()
        } else {
            self.pinned_right.as_mut()
        };
        let Some(pin) = pin else {
            return false;
        };
        if !pin.column.windows.iter().any(|window| window.id == id) {
            return false;
        }
        remove_window(&mut pin.column, id);
        if pin.column.windows.is_empty() {
            if left {
                self.pinned_left = None;
                if self.focus == Focus::PinnedLeft {
                    self.focus = Focus::None;
                }
            } else {
                self.pinned_right = None;
                if self.focus == Focus::PinnedRight {
                    self.focus = Focus::None;
                }
            }
        }
        true
    }

    fn retarget_after_column_removed(&mut self, removed: usize) {
        self.focus = match self.focus {
            Focus::Column(index) if index == removed => {
                if removed > 0 {
                    Focus::Column(removed - 1)
                } else if !self.columns.is_empty() {
                    Focus::Column(0)
                } else {
                    Focus::None
                }
            }
            Focus::Column(index) if index > removed => Focus::Column(index - 1),
            other => other,
        };
    }

    fn focused_column(&self) -> Option<&Column> {
        match self.focus {
            Focus::Column(index) => self.columns.get(index),
            Focus::PinnedLeft => self.pinned_left.as_ref().map(|pin| &pin.column),
            Focus::PinnedRight => self.pinned_right.as_ref().map(|pin| &pin.column),
            Focus::None => None,
        }
    }

    fn focused_column_mut(&mut self) -> Option<&mut Column> {
        match self.focus {
            Focus::Column(index) => self.columns.get_mut(index),
            Focus::PinnedLeft => self.pinned_left.as_mut().map(|pin| &mut pin.column),
            Focus::PinnedRight => self.pinned_right.as_mut().map(|pin| &mut pin.column),
            Focus::None => None,
        }
    }
}

impl Default for Strip {
    fn default() -> Self {
        Self::new()
    }
}

fn column_number(n: u32, len: usize) -> usize {
    if n == 9 {
        len - 1
    } else {
        (n.saturating_sub(1) as usize).min(len - 1)
    }
}

fn remap_index(focus: usize, from: usize, dest: usize) -> usize {
    if focus == from {
        dest
    } else if from < focus && dest >= focus {
        focus - 1
    } else if from > focus && dest <= focus {
        focus + 1
    } else {
        focus
    }
}

fn remove_window(column: &mut Column, id: WindowId) {
    column.windows.retain(|window| window.id != id);
    if column.windows.is_empty() {
        column.focused_window = 0;
        column.active_tab = 0;
        return;
    }
    rescale(&mut column.windows);
    if column.focused_window >= column.windows.len() {
        column.focused_window = column.windows.len() - 1;
    }
    if column.active_tab >= column.windows.len() {
        column.active_tab = column.windows.len() - 1;
    }
}

fn rescale(windows: &mut [Window]) {
    let sum: f64 = windows.iter().map(|window| window.height_ratio).sum();
    if sum <= f64::EPSILON {
        equalize(windows);
        return;
    }
    for window in windows {
        window.height_ratio /= sum;
    }
}

fn equalize(windows: &mut [Window]) {
    if windows.is_empty() {
        return;
    }
    let share = 1.0 / windows.len() as f64;
    for window in windows {
        window.height_ratio = share;
    }
}

fn push_share(windows: &mut Vec<Window>, mut window: Window) {
    let count = windows.len() as f64;
    let next = count + 1.0;
    if count == 0.0 {
        window.height_ratio = 1.0;
        windows.push(window);
        return;
    }
    for existing in windows.iter_mut() {
        existing.height_ratio *= count / next;
    }
    window.height_ratio = 1.0 / next;
    windows.push(window);
}

fn next_preset(current: f64, presets: &[f64]) -> f64 {
    let Some(first) = presets.first().copied() else {
        return current;
    };
    if let Some(pos) = presets.iter().position(|preset| (preset - current).abs() < 1e-6) {
        presets[(pos + 1) % presets.len()]
    } else if let Some(preset) = presets.iter().copied().find(|preset| *preset > current + 1e-6) {
        preset
    } else {
        first
    }
}

const MIN_RATIO: f64 = 0.1;

fn nudge_ratios(ratios: &mut [f64], index: usize, delta: f64) {
    if ratios.len() < 2 || index >= ratios.len() {
        return;
    }
    if delta > 0.0 {
        let cap = 1.0 - MIN_RATIO * (ratios.len() - 1) as f64;
        let room = (cap - ratios[index]).max(0.0);
        let slack: f64 = ratios
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != index)
            .map(|(_, ratio)| (ratio - MIN_RATIO).max(0.0))
            .sum();
        let give = 0.10_f64.min(room).min(slack);
        if give <= 1e-9 {
            return;
        }
        ratios[index] += give;
        let mut left = give;
        for other in neighbor_order(ratios.len(), index) {
            if left <= 1e-9 {
                break;
            }
            let can = (ratios[other] - MIN_RATIO).max(0.0);
            let take = can.min(left);
            ratios[other] -= take;
            left -= take;
        }
        return;
    }
    let drop = 0.10_f64.min((ratios[index] - MIN_RATIO).max(0.0));
    if drop <= 1e-9 {
        return;
    }
    ratios[index] -= drop;
    let recipient = if index + 1 < ratios.len() { index + 1 } else { index - 1 };
    ratios[recipient] += drop;
}

fn neighbor_order(len: usize, index: usize) -> Vec<usize> {
    let mut order = Vec::new();
    for distance in 1..len {
        if index + distance < len {
            order.push(index + distance);
        }
        if distance <= index {
            order.push(index - distance);
        }
    }
    order
}
