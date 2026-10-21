use crate::CenterMode;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Frame {
    pub fn right(self) -> f64 {
        self.x + self.width
    }

    pub fn bottom(self) -> f64 {
        self.y + self.height
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct DisplayFrame {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
    pub neighbour_left: bool,
    pub neighbour_right: bool,
}

impl DisplayFrame {
    pub fn from_origin(width: f64, height: f64) -> Self {
        Self {
            min_x: 0.0,
            min_y: 0.0,
            max_x: width,
            max_y: height,
            neighbour_left: false,
            neighbour_right: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct LayoutParams {
    pub gap_inner: f64,
    pub gap_outer: f64,
    pub sliver: f64,
    pub center: CenterMode,
    pub keep_original_height: bool,
    pub presets: Vec<f64>,
    pub width_step: f64,
    pub default_column_width: f64,
}

impl LayoutParams {
    pub fn standard() -> Self {
        Self {
            gap_inner: 8.0,
            gap_outer: 8.0,
            sliver: 5.0,
            center: CenterMode::Never,
            keep_original_height: false,
            presets: vec![0.25, 0.33, 0.5, 0.66, 0.75],
            width_step: 0.10,
            default_column_width: 0.5,
        }
    }
}
