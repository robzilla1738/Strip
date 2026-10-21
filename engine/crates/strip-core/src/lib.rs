//! Pure strip model: layout, parking, and the Section 1.2 edits.
//! No macOS types.

mod gesture;
mod layout;
mod model;
mod params;
mod spring;

pub use gesture::{decay_velocity, rubber_band, snap_column, tracking_delta, Gesture, SwipeUpdate, SwipeView};
pub use layout::{
    crosses_display, layout, observe_size, overlap, parked_corner, sliver_used, Corner, Placement,
    Side, SizeWatch, Visibility,
};
pub use model::{Column, ColumnId, DisplayMode, Focus, Pinned, Strip, Window, WindowId};
pub use params::{DisplayFrame, Frame, LayoutParams, Viewport};
pub use spring::Spring;

#[cfg(test)]
mod props;

pub const ENGINE_VERSION: &str = "1.0.0";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CenterMode {
    Never,
    OnOverflow,
    Always,
}
