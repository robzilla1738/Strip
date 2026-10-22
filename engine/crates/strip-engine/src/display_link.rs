//! Refresh-locked ticks. The callback only records the interval and wakes the main run loop.

use std::collections::{HashMap, HashSet};
use std::os::raw::{c_int, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::hotkeys;
use crate::metrics::percentile;

struct Inner {
    samples_ms: Mutex<Vec<f64>>,
    pending_dt: Mutex<Vec<f64>>,
    last_ns: AtomicU64,
    refresh_ns: AtomicU64,
    running: AtomicBool,
}

pub struct Pacing {
    pub frame_interval_p95_ms: f64,
    pub refresh_interval_ms: f64,
    pub running: bool,
}

pub struct DisplayDriver {
    links: HashMap<u32, *mut c_void>,
    started: HashSet<u32>,
    inner: std::sync::Arc<Inner>,
}

impl DisplayDriver {
    pub fn new() -> Self {
        Self {
            links: HashMap::new(),
            started: HashSet::new(),
            inner: std::sync::Arc::new(Inner {
                samples_ms: Mutex::new(Vec::new()),
                pending_dt: Mutex::new(Vec::new()),
                last_ns: AtomicU64::new(0),
                refresh_ns: AtomicU64::new(0),
                running: AtomicBool::new(false),
            }),
        }
    }

    pub fn start(&mut self, mut display: u32) {
        if display == 0 {
            display = unsafe { CGMainDisplayID() };
        }
        let link = self.ensure(display);
        if link.is_null() || self.started.contains(&display) {
            return;
        }
        self.inner.last_ns.store(0, Ordering::SeqCst);
        let status = unsafe { CVDisplayLinkStart(link) };
        if status != 0 {
            eprintln!("display link did not start");
            return;
        }
        let period = unsafe { CVDisplayLinkGetActualOutputVideoRefreshPeriod(link) };
        if period > 0.0 {
            self.inner.refresh_ns.store((period * 1000.0).to_bits(), Ordering::SeqCst);
        }
        self.started.insert(display);
        self.inner.running.store(true, Ordering::SeqCst);
    }

    pub fn stop(&mut self) {
        for link in self.links.values().copied() {
            unsafe { CVDisplayLinkStop(link) };
        }
        self.started.clear();
        self.inner.running.store(false, Ordering::SeqCst);
        self.inner.last_ns.store(0, Ordering::SeqCst);
    }

    pub fn take_dts(&self) -> Vec<f64> {
        let mut pending = self.inner.pending_dt.lock().unwrap_or_else(|err| err.into_inner());
        std::mem::take(&mut *pending)
    }

    pub fn snapshot(&self) -> Pacing {
        let samples = self.inner.samples_ms.lock().unwrap_or_else(|err| err.into_inner());
        let refresh_bits = self.inner.refresh_ns.load(Ordering::SeqCst);
        let refresh = f64::from_bits(refresh_bits);
        Pacing {
            frame_interval_p95_ms: percentile(&samples, 95.0),
            refresh_interval_ms: if refresh > 0.0 { refresh } else { 1000.0 / 60.0 },
            running: self.inner.running.load(Ordering::SeqCst),
        }
    }

    fn ensure(&mut self, display: u32) -> *mut c_void {
        if let Some(link) = self.links.get(&display).copied() {
            return link;
        }
        let mut link = ptr::null_mut();
        let created = unsafe { CVDisplayLinkCreateWithCGDisplay(display, &mut link) };
        if created != 0 || link.is_null() {
            eprintln!("display link unavailable");
            return ptr::null_mut();
        }
        let user = std::sync::Arc::as_ptr(&self.inner) as *mut c_void;
        let armed = unsafe { CVDisplayLinkSetOutputCallback(link, output_callback, user) };
        if armed != 0 {
            unsafe { CVDisplayLinkRelease(link) };
            eprintln!("display link callback rejected");
            return ptr::null_mut();
        }
        self.links.insert(display, link);
        link
    }
}

impl Drop for DisplayDriver {
    fn drop(&mut self) {
        self.stop();
        for link in self.links.values().copied() {
            unsafe { CVDisplayLinkRelease(link) };
        }
        self.links.clear();
    }
}

impl Default for DisplayDriver {
    fn default() -> Self {
        Self::new()
    }
}

extern "C" fn output_callback(
    _link: *mut c_void,
    _now: *const u8,
    output: *const u8,
    _flags_in: u64,
    _flags_out: *mut u64,
    user: *mut c_void,
) -> c_int {
    if user.is_null() || output.is_null() {
        return 0;
    }
    let host = unsafe { ptr::read_unaligned(output.add(16) as *const u64) };
    let inner = unsafe { &*(user as *const Inner) };
    inner.note(host);
    wake_main_loop();
    0
}

impl Inner {
    fn note(&self, host: u64) {
        let ns = host_to_ns(host);
        let previous = self.last_ns.swap(ns, Ordering::SeqCst);
        if previous == 0 {
            let refresh = f64::from_bits(self.refresh_ns.load(Ordering::SeqCst));
            let ms = if refresh > 0.0 && refresh < 80.0 { refresh } else { 1000.0 / 120.0 };
            let mut pending = self.pending_dt.lock().unwrap_or_else(|err| err.into_inner());
            if pending.len() < 60 {
                pending.push(ms / 1000.0);
            }
            return;
        }
        if ns <= previous {
            return;
        }
        let ms = (ns - previous) as f64 / 1_000_000.0;
        if !(ms > 0.0 && ms < 80.0) {
            return;
        }
        let mut samples = self.samples_ms.lock().unwrap_or_else(|err| err.into_inner());
        samples.push(ms);
        if samples.len() > 720 {
            let extra = samples.len() - 720;
            samples.drain(0..extra);
        }
        drop(samples);
        let mut pending = self.pending_dt.lock().unwrap_or_else(|err| err.into_inner());
        if pending.len() < 60 {
            pending.push(ms / 1000.0);
        }
    }
}

fn host_to_ns(host: u64) -> u64 {
    let scale = timebase();
    host.saturating_mul(u64::from(scale.0)) / u64::from(scale.1.max(1))
}

#[repr(C)]
struct MachTimebase {
    numer: u32,
    denom: u32,
}

fn timebase() -> (u32, u32) {
    static SCALE: OnceLock<(u32, u32)> = OnceLock::new();
    *SCALE.get_or_init(|| {
        let mut info = MachTimebase { numer: 0, denom: 1 };
        unsafe { mach_timebase_info(&mut info) };
        if info.denom == 0 {
            (1, 1)
        } else {
            (info.numer, info.denom)
        }
    })
}

extern "C" {
    fn mach_timebase_info(info: *mut MachTimebase) -> c_int;
}

fn wake_main_loop() {
    hotkeys::wake_main();
}

#[link(name = "CoreVideo", kind = "framework")]
extern "C" {
    fn CVDisplayLinkCreateWithCGDisplay(display: u32, link: *mut *mut c_void) -> c_int;
    fn CVDisplayLinkSetOutputCallback(
        link: *mut c_void,
        callback: extern "C" fn(*mut c_void, *const u8, *const u8, u64, *mut u64, *mut c_void) -> c_int,
        user: *mut c_void,
    ) -> c_int;
    fn CVDisplayLinkStart(link: *mut c_void) -> c_int;
    fn CVDisplayLinkStop(link: *mut c_void) -> c_int;
    fn CVDisplayLinkRelease(link: *mut c_void);
    fn CVDisplayLinkGetActualOutputVideoRefreshPeriod(link: *mut c_void) -> f64;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGMainDisplayID() -> u32;
}
