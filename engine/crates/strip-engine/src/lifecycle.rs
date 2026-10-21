//! Panic and termination both un-park. The signal handler only writes one byte.

use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, OnceLock};

static STOP: AtomicBool = AtomicBool::new(false);
static WAKE_FD: AtomicI32 = AtomicI32::new(-1);
static UNPARK: OnceLock<Arc<dyn Fn() + Send + Sync>> = OnceLock::new();

/// Runs the un-park callback. The panic hook calls this on the panicking thread.
/// The main thread calls it after a signal wakes the self-pipe.
pub fn handle_termination(unpark: &dyn Fn()) {
    unpark();
}

pub fn stop_requested() -> bool {
    STOP.load(Ordering::SeqCst)
}

pub fn request_stop() {
    STOP.store(true, Ordering::SeqCst);
    poke();
}

/// Installs the panic hook and the SIGTERM/SIGINT wake. Returns the read end
/// the main loop polls. Dropping it does not uninstall the hook.
pub fn install(unpark: Arc<dyn Fn() + Send + Sync>) -> Option<Wakeup> {
    let for_panic = Arc::clone(&unpark);
    let _ = UNPARK.set(unpark);
    std::panic::set_hook(Box::new(move |_| {
        handle_termination(&*for_panic);
    }));
    let wakeup = Wakeup::arm()?;
    install_signals();
    Some(wakeup)
}

pub struct Wakeup {
    read: UnixStream,
}

impl Wakeup {
    pub fn arm() -> Option<Self> {
        let (read, write) = UnixStream::pair().ok()?;
        read.set_nonblocking(true).ok()?;
        let fd = write.as_raw_fd();
        // The write end has to outlive the process; the handler uses the raw fd.
        std::mem::forget(write);
        WAKE_FD.store(fd, Ordering::SeqCst);
        Some(Self { read })
    }

    pub fn poked(&mut self) -> bool {
        let mut buf = [0u8; 8];
        matches!(self.read.read(&mut buf), Ok(n) if n > 0)
    }
}

fn poke() {
    let fd = WAKE_FD.load(Ordering::SeqCst);
    if fd < 0 {
        return;
    }
    let buf = [1u8];
    unsafe {
        let _ = libc::write(fd, buf.as_ptr().cast(), 1);
    }
}

fn install_signals() {
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = handler as *const () as libc::sighandler_t;
        libc::sigemptyset(std::ptr::addr_of_mut!(action.sa_mask));
        libc::sigaction(libc::SIGTERM, std::ptr::addr_of!(action), std::ptr::null_mut());
        libc::sigaction(libc::SIGINT, std::ptr::addr_of!(action), std::ptr::null_mut());
    }
}

extern "C" fn handler(_signal: i32) {
    STOP.store(true, Ordering::SeqCst);
    poke();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn termination_runs_the_unpark_callback() {
        let calls = AtomicUsize::new(0);
        handle_termination(&|| {
            calls.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn wakeup_pipe_observes_request_stop() {
        let mut wake = Wakeup::arm().expect("self-pipe");
        STOP.store(false, Ordering::SeqCst);
        request_stop();
        assert!(stop_requested());
        assert!(wake.poked());
        STOP.store(false, Ordering::SeqCst);
    }
}
