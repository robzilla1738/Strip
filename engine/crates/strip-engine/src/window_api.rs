//! The AX operations the writer pool needs. macOS implements this; tests use a mock.

use strip_core::Frame;

/// Per-pid writers call this concurrently, so implementations take `&self`.
pub trait WindowApi: Send + Sync {
    fn write_size(&self, window: u64, width: f64, height: f64);
    fn write_position(&self, window: u64, x: f64, y: f64);
    fn read_frame(&self, window: u64) -> Frame;
    fn enhanced_ui(&self, pid: i32) -> bool;
    fn set_enhanced_ui(&self, pid: i32, enabled: bool);
}

/// Resize then move: size, position, size. Animation frames call `write_position` alone.
pub fn write_resize_move<A: WindowApi + ?Sized>(api: &A, window: u64, frame: Frame) {
    api.write_size(window, frame.width, frame.height);
    api.write_position(window, frame.x, frame.y);
    api.write_size(window, frame.width, frame.height);
}
