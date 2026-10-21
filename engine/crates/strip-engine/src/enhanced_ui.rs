//! Turns `AXEnhancedUserInterface` off around writes and restores it 250 ms after the last one.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::window_api::WindowApi;

const RESTORE_AFTER: Duration = Duration::from_millis(250);

#[derive(Default)]
pub struct EnhancedGuard {
    restore: HashMap<i32, bool>,
    last_write: HashMap<i32, Instant>,
}

impl EnhancedGuard {
    /// Reads the attribute once per burst of writes. A pid whose attribute was
    /// already off is remembered as `false` so later writes skip the AX read.
    pub fn before_write<A: WindowApi + ?Sized>(&mut self, api: &A, pid: i32, now: Instant) {
        if let std::collections::hash_map::Entry::Vacant(slot) = self.restore.entry(pid) {
            let enabled = api.enhanced_ui(pid);
            if enabled {
                api.set_enhanced_ui(pid, false);
            }
            slot.insert(enabled);
        }
        self.last_write.insert(pid, now);
    }

    pub fn note_write(&mut self, pid: i32, now: Instant) {
        self.last_write.insert(pid, now);
    }

    pub fn restore_due<A: WindowApi + ?Sized>(&mut self, api: &A, now: Instant) {
        let due: Vec<i32> = self
            .last_write
            .iter()
            .filter(|(_, then)| now.saturating_duration_since(**then) >= RESTORE_AFTER)
            .map(|(pid, _)| *pid)
            .collect();
        for pid in due {
            self.last_write.remove(&pid);
            if self.restore.remove(&pid).unwrap_or(false) {
                api.set_enhanced_ui(pid, true);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window_api::WindowApi;
    use mockall::mock;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use strip_core::Frame;

    mock! {
        pub Api {}
        impl WindowApi for Api {
            fn write_size(&self, window: u64, width: f64, height: f64);
            fn write_position(&self, window: u64, x: f64, y: f64);
            fn read_frame(&self, window: u64) -> Frame;
            fn enhanced_ui(&self, pid: i32) -> bool;
            fn set_enhanced_ui(&self, pid: i32, enabled: bool);
        }
    }

    #[test]
    fn restores_enhanced_ui_250ms_after_the_last_write() {
        let enabled = Arc::new(AtomicBool::new(true));
        let read = enabled.clone();
        let write = enabled.clone();
        let mut api = MockApi::new();
        api.expect_enhanced_ui()
            .returning(move |_| read.load(Ordering::SeqCst));
        api.expect_set_enhanced_ui()
            .returning(move |_, on| write.store(on, Ordering::SeqCst));

        let start = Instant::now();
        let mut guard = EnhancedGuard::default();
        guard.before_write(&api, 7, start);
        assert!(!enabled.load(Ordering::SeqCst));

        guard.before_write(&api, 7, start + Duration::from_millis(100));
        guard.restore_due(&api, start + Duration::from_millis(349));
        assert!(!enabled.load(Ordering::SeqCst));

        guard.restore_due(&api, start + Duration::from_millis(350));
        assert!(enabled.load(Ordering::SeqCst));
    }

    #[test]
    fn leaves_enhanced_ui_alone_when_it_was_already_off() {
        let enabled = Arc::new(AtomicBool::new(false));
        let read = enabled.clone();
        let mut api = MockApi::new();
        api.expect_enhanced_ui()
            .returning(move |_| read.load(Ordering::SeqCst));
        api.expect_set_enhanced_ui().never();

        let start = Instant::now();
        let mut guard = EnhancedGuard::default();
        guard.before_write(&api, 7, start);
        guard.restore_due(&api, start + Duration::from_millis(250));
        assert!(!enabled.load(Ordering::SeqCst));
    }
}
