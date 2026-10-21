//! The AX operations the writer pool needs. macOS implements this; tests use a mock.

use strip_core::Frame;

/// One serial writer talks to a pid through this trait.
pub trait WindowApi: Send {
    fn write_size(&mut self, window: u64, width: f64, height: f64);
    fn write_position(&mut self, window: u64, x: f64, y: f64);
    fn read_frame(&mut self, window: u64) -> Frame;
    fn enhanced_ui(&mut self, pid: i32) -> bool;
    fn set_enhanced_ui(&mut self, pid: i32, enabled: bool);
}

/// Resize then move: size, position, size. Animation frames call `write_position` alone.
pub fn write_resize_move<A: WindowApi + ?Sized>(api: &mut A, window: u64, frame: Frame) {
    api.write_size(window, frame.width, frame.height);
    api.write_position(window, frame.x, frame.y);
    api.write_size(window, frame.width, frame.height);
}
