//! Section 2.11. Accessibility raise first, SkyLight front-process if that fails.

pub trait FocusHost {
    fn ax_raise(&mut self, window: u64, pid: i32) -> bool;
    fn slps_raise(&mut self, window: u64, pid: i32);
    fn activate(&mut self, pid: i32);
}

/// Raise the window. AX wins. The SkyLight path runs only when AX fails, and
/// the process is activated after that path.
pub fn raise_focused<H: FocusHost + ?Sized>(host: &mut H, window: u64, pid: i32) {
    if host.ax_raise(window, pid) {
        return;
    }
    host.slps_raise(window, pid);
    host.activate(pid);
}

/// Production host. `AXRaise` + `AXFrontmost`, else the SkyLight front-process calls.
pub struct AxHost<'a> {
    pub api: &'a mut crate::ax::AxApi,
}

impl FocusHost for AxHost<'_> {
    fn ax_raise(&mut self, window: u64, pid: i32) -> bool {
        crate::ax::raise(self.api, window, pid)
    }

    fn slps_raise(&mut self, window: u64, pid: i32) {
        let Some(window) = u32::try_from(window).ok() else {
            return;
        };
        crate::private_api::slps_raise(pid, window);
    }

    fn activate(&mut self, pid: i32) {
        crate::ax::activate(pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Script {
        ax: bool,
        slps: u32,
        activated: u32,
    }

    impl FocusHost for Script {
        fn ax_raise(&mut self, _window: u64, _pid: i32) -> bool {
            self.ax
        }

        fn slps_raise(&mut self, _window: u64, _pid: i32) {
            self.slps += 1;
        }

        fn activate(&mut self, _pid: i32) {
            self.activated += 1;
        }
    }

    #[test]
    fn ax_raise_does_not_call_skylight() {
        let mut host = Script { ax: true, slps: 0, activated: 0 };
        raise_focused(&mut host, 7, 4);
        assert_eq!(host.slps, 0);
        assert_eq!(host.activated, 0);
    }

    #[test]
    fn failed_ax_raise_uses_slps_then_activate() {
        let mut host = Script { ax: false, slps: 0, activated: 0 };
        raise_focused(&mut host, 7, 4);
        assert_eq!(host.slps, 1);
        assert_eq!(host.activated, 1);
    }
}
