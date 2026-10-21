//! One serial worker per pid. Each worker writes the newest target and drops stale ones.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use strip_core::Frame;

use crate::enhanced_ui::EnhancedGuard;
use crate::metrics::Metrics;
use crate::window_api::{write_resize_move, WindowApi};

pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

#[derive(Clone)]
pub struct ManualClock {
    origin: Instant,
    offset_ms: Arc<AtomicU64>,
}

impl Default for ManualClock {
    fn default() -> Self {
        Self::new()
    }
}

impl ManualClock {
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
            offset_ms: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn advance(&self, ms: u64) {
        self.offset_ms.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Instant {
        self.origin + Duration::from_millis(self.offset_ms.load(Ordering::SeqCst))
    }
}

#[derive(Clone, Copy)]
struct Published {
    generation: u64,
    frame: Frame,
    /// Bumps only when a structural size change must be written.
    size_generation: u64,
    /// True for a position tick. Structural publishes stay false.
    animation_frame: bool,
}

struct Slot {
    window: u64,
    bundle_id: String,
    value: Arc<ArcSwap<Published>>,
}

struct WorkerState {
    slots: Mutex<Vec<Slot>>,
    wake: Condvar,
    stop: AtomicBool,
    idle: AtomicBool,
}

struct Shared<A> {
    api: Mutex<A>,
    guard: Mutex<EnhancedGuard>,
    metrics: Mutex<Metrics>,
    commits: AtomicU64,
    last_frames: Mutex<HashMap<u64, Frame>>,
    clock: Arc<dyn Clock>,
}

struct Worker {
    state: Arc<WorkerState>,
    thread: Option<JoinHandle<()>>,
}

pub struct WriterPool<A> {
    shared: Arc<Shared<A>>,
    workers: HashMap<i32, Worker>,
}

impl<A: WindowApi + 'static> WriterPool<A> {
    pub fn new(api: A, clock: Arc<dyn Clock>) -> Self {
        Self {
            shared: Arc::new(Shared {
                api: Mutex::new(api),
                guard: Mutex::new(EnhancedGuard::default()),
                metrics: Mutex::new(Metrics::default()),
                commits: AtomicU64::new(0),
                last_frames: Mutex::new(HashMap::new()),
                clock,
            }),
            workers: HashMap::new(),
        }
    }

    pub fn publish(&mut self, pid: i32, window: u64, frame: Frame, bundle_id: &str) {
        self.enqueue(pid, window, frame, bundle_id, true);
    }

    /// Position tick. Does not request a size write.
    pub fn publish_position(&mut self, pid: i32, window: u64, frame: Frame, bundle_id: &str) {
        self.enqueue(pid, window, frame, bundle_id, false);
    }

    fn enqueue(&mut self, pid: i32, window: u64, frame: Frame, bundle_id: &str, write_size: bool) {
        let worker = self.workers.entry(pid).or_insert_with(|| spawn_worker(pid, Arc::clone(&self.shared)));
        {
            let mut slots = lock(&worker.state.slots);
            if let Some(slot) = slots.iter().find(|slot| slot.window == window) {
                let previous = **slot.value.load();
                let size_generation = if write_size {
                    previous.size_generation.saturating_add(1)
                } else {
                    previous.size_generation
                };
                slot.value.store(Arc::new(Published {
                    generation: previous.generation.saturating_add(1),
                    frame,
                    size_generation,
                    animation_frame: !write_size,
                }));
            } else {
                slots.push(Slot {
                    window,
                    bundle_id: bundle_id.to_string(),
                    value: Arc::new(ArcSwap::from_pointee(Published {
                        generation: 1,
                        frame,
                        size_generation: u64::from(write_size),
                        animation_frame: !write_size,
                    })),
                });
            }
        }
        worker.state.idle.store(false, Ordering::SeqCst);
        worker.state.wake.notify_one();
    }

    pub fn poll_enhanced_ui(&self) {
        let now = self.shared.clock.now();
        let mut api = lock(&self.shared.api);
        lock(&self.shared.guard).restore_due(&mut *api, now);
    }

    pub fn commits(&self) -> u64 {
        self.shared.commits.load(Ordering::SeqCst)
    }

    pub fn last_frame(&self, window: u64) -> Option<Frame> {
        lock(&self.shared.last_frames).get(&window).copied()
    }

    pub fn is_idle(&self) -> bool {
        self.workers.values().all(|worker| worker.state.idle.load(Ordering::SeqCst))
    }

    pub fn metrics(&self) -> Metrics {
        lock(&self.shared.metrics).clone()
    }

    pub fn with_api<R>(&self, body: impl FnOnce(&mut A) -> R) -> R {
        let mut api = lock(&self.shared.api);
        body(&mut api)
    }

    pub fn shutdown(&mut self) {
        for worker in self.workers.values() {
            worker.state.stop.store(true, Ordering::SeqCst);
            worker.state.wake.notify_one();
        }
        for worker in self.workers.values_mut() {
            if let Some(thread) = worker.thread.take() {
                let _ = thread.join();
            }
        }
    }
}

impl<A> Drop for WriterPool<A> {
    fn drop(&mut self) {
        for worker in self.workers.values() {
            worker.state.stop.store(true, Ordering::SeqCst);
            worker.state.wake.notify_one();
        }
        for worker in self.workers.values_mut() {
            if let Some(thread) = worker.thread.take() {
                let _ = thread.join();
            }
        }
    }
}

fn spawn_worker<A: WindowApi + 'static>(pid: i32, shared: Arc<Shared<A>>) -> Worker {
    let state = Arc::new(WorkerState {
        slots: Mutex::new(Vec::new()),
        wake: Condvar::new(),
        stop: AtomicBool::new(false),
        idle: AtomicBool::new(false),
    });
    let thread_state = Arc::clone(&state);
    let thread = thread::Builder::new()
        .name(format!("ax-{pid}"))
        .spawn(move || worker_loop(pid, thread_state, shared))
        .expect("start ax worker");
    Worker {
        state,
        thread: Some(thread),
    }
}

fn pending_generation(slots: &[Slot], applied: &HashMap<u64, u64>) -> bool {
    slots.iter().all(|slot| {
        let generation = slot.value.load().generation;
        generation == 0 || applied.get(&slot.window).copied() == Some(generation)
    })
}

struct Pending {
    window: u64,
    bundle_id: String,
    generation: u64,
    frame: Frame,
    size_generation: u64,
    animation_frame: bool,
    slot: Arc<ArcSwap<Published>>,
}

fn worker_loop<A: WindowApi>(pid: i32, state: Arc<WorkerState>, shared: Arc<Shared<A>>) {
    let mut applied = HashMap::<u64, u64>::new();
    let mut applied_size = HashMap::<u64, u64>::new();
    let mut guard = lock(&state.slots);
    loop {
        if state.stop.load(Ordering::SeqCst) {
            break;
        }
        let pending: Vec<Pending> = guard
            .iter()
            .filter_map(|slot| {
                let published = **slot.value.load();
                let seen = applied.get(&slot.window).copied().unwrap_or(0);
                if published.generation == 0 || published.generation == seen {
                    None
                } else {
                    Some(Pending {
                        window: slot.window,
                        bundle_id: slot.bundle_id.clone(),
                        generation: published.generation,
                        frame: published.frame,
                        size_generation: published.size_generation,
                        animation_frame: published.animation_frame,
                        slot: Arc::clone(&slot.value),
                    })
                }
            })
            .collect();
        if pending.is_empty() {
            state.idle.store(true, Ordering::SeqCst);
            let (next, result) = state
                .wake
                .wait_timeout(guard, Duration::from_millis(50))
                .unwrap_or_else(|err| err.into_inner());
            guard = next;
            if result.timed_out() && pending_generation(&guard, &applied) {
                continue;
            }
            state.idle.store(false, Ordering::SeqCst);
            continue;
        }
        drop(guard);
        for item in pending {
            let latest = **item.slot.load();
            let generation = latest.generation.max(item.generation);
            let frame = if latest.generation == 0 { item.frame } else { latest.frame };
            let size_generation = if latest.generation == 0 { item.size_generation } else { latest.size_generation };
            let animation_frame = if latest.generation == 0 { item.animation_frame } else { latest.animation_frame };
            let seen_size = applied_size.get(&item.window).copied().unwrap_or(0);
            apply_one(&shared, pid, item.window, &item.bundle_id, frame, size_generation != seen_size, animation_frame);
            applied.insert(item.window, generation);
            applied_size.insert(item.window, size_generation);
        }
        guard = lock(&state.slots);
    }
    state.idle.store(true, Ordering::SeqCst);
}

fn apply_one<A: WindowApi>(
    shared: &Shared<A>,
    pid: i32,
    window: u64,
    bundle: &str,
    frame: Frame,
    write_size: bool,
    _animation_frame: bool,
) {
    let started = Instant::now();
    {
        let mut api = lock(&shared.api);
        let now = shared.clock.now();
        lock(&shared.guard).before_write(&mut *api, pid, now);
        if write_size {
            write_resize_move(&mut *api, window, frame);
        } else {
            api.write_position(window, frame.x, frame.y);
        }
        let _read_back = api.read_frame(window);
        lock(&shared.guard).before_write(&mut *api, pid, shared.clock.now());
    }
    lock(&shared.last_frames).insert(window, frame);
    shared.commits.fetch_add(1, Ordering::SeqCst);
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    lock(&shared.metrics).record_write(bundle, elapsed_ms);
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window_api::WindowApi;
    use mockall::mock;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    mock! {
        pub Api {}
        impl WindowApi for Api {
            fn write_size(&mut self, window: u64, width: f64, height: f64);
            fn write_position(&mut self, window: u64, x: f64, y: f64);
            fn read_frame(&mut self, window: u64) -> Frame;
            fn enhanced_ui(&mut self, pid: i32) -> bool;
            fn set_enhanced_ui(&mut self, pid: i32, enabled: bool);
        }
    }

    fn frame(x: f64) -> Frame {
        Frame {
            x,
            y: 10.0,
            width: 400.0,
            height: 300.0,
        }
    }

    #[test]
    fn slow_writer_coalesces_one_hundred_targets() {
        let written = Arc::new(Mutex::new(Vec::<Frame>::new()));
        let positions = written.clone();
        let mut api = MockApi::new();
        api.expect_write_size().returning(|_, _, _| {
            thread::sleep(Duration::from_millis(20));
        });
        api.expect_write_position().returning(move |_, x, y| {
            positions.lock().unwrap_or_else(|err| err.into_inner()).push(Frame {
                x,
                y,
                width: 400.0,
                height: 300.0,
            });
        });
        api.expect_read_frame().returning(|_| frame(0.0));
        api.expect_enhanced_ui().returning(|_| false);
        api.expect_set_enhanced_ui().returning(|_, _| {});

        let mut pool = WriterPool::new(api, Arc::new(SystemClock));
        for index in 0..100 {
            pool.publish(9, 100, frame(index as f64), "com.example.Slow");
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while pool.commits() == 0 || !pool.is_idle() {
            assert!(Instant::now() < deadline, "writer did not go idle");
            thread::sleep(Duration::from_millis(5));
        }
        // One extra beat so a commit that started as we observed idle can finish.
        thread::sleep(Duration::from_millis(30));
        let commits = pool.commits();
        let last = pool.last_frame(100);
        pool.shutdown();
        assert!(commits <= 3, "commits = {commits}");
        assert!(commits >= 1);
        assert_eq!(last, Some(frame(99.0)));
        let positions = written.lock().unwrap_or_else(|err| err.into_inner());
        assert!(positions.len() <= 3);
        assert_eq!(positions.last().map(|frame| frame.x), Some(99.0));
    }

    #[test]
    fn pool_restores_enhanced_ui_after_the_injected_delay() {
        let enabled = Arc::new(AtomicBool::new(true));
        let read = enabled.clone();
        let write = enabled.clone();
        let mut api = MockApi::new();
        api.expect_write_size().returning(|_, _, _| {});
        api.expect_write_position().returning(|_, _, _| {});
        api.expect_read_frame().returning(|_| frame(1.0));
        api.expect_enhanced_ui()
            .returning(move |_| read.load(Ordering::SeqCst));
        api.expect_set_enhanced_ui()
            .returning(move |_, on| write.store(on, Ordering::SeqCst));

        let clock = Arc::new(ManualClock::new());
        let mut pool = WriterPool::new(api, clock.clone());
        pool.publish(4, 8, frame(3.0), "com.example.App");
        let deadline = Instant::now() + Duration::from_secs(2);
        while pool.commits() == 0 {
            assert!(Instant::now() < deadline, "write did not happen");
            thread::sleep(Duration::from_millis(5));
        }
        assert!(!enabled.load(Ordering::SeqCst));
        clock.advance(249);
        pool.poll_enhanced_ui();
        assert!(!enabled.load(Ordering::SeqCst));
        clock.advance(1);
        pool.poll_enhanced_ui();
        assert!(enabled.load(Ordering::SeqCst));
        pool.shutdown();
    }

    #[test]
    fn animation_frames_write_position_without_size() {
        let sizes = Arc::new(AtomicU64::new(0));
        let positions = Arc::new(AtomicU64::new(0));
        let size_count = Arc::clone(&sizes);
        let position_count = Arc::clone(&positions);
        let mut api = MockApi::new();
        api.expect_write_size().returning(move |_, _, _| {
            size_count.fetch_add(1, Ordering::SeqCst);
        });
        api.expect_write_position().returning(move |_, _, _| {
            position_count.fetch_add(1, Ordering::SeqCst);
        });
        api.expect_read_frame().returning(|_| frame(0.0));
        api.expect_enhanced_ui().returning(|_| false);
        api.expect_set_enhanced_ui().returning(|_, _| {});

        let mut pool = WriterPool::new(api, Arc::new(SystemClock));
        pool.publish(3, 7, frame(1.0), "com.example.App");
        let deadline = Instant::now() + Duration::from_secs(2);
        while pool.commits() == 0 {
            assert!(Instant::now() < deadline, "structural write did not happen");
            thread::sleep(Duration::from_millis(5));
        }
        let sizes_after_structure = sizes.load(Ordering::SeqCst);
        assert!(sizes_after_structure >= 1);
        for index in 2..8 {
            pool.publish_position(3, 7, frame(index as f64), "com.example.App");
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while !pool.is_idle() || pool.last_frame(7).map(|frame| frame.x) != Some(7.0) {
            assert!(Instant::now() < deadline, "position frames did not settle");
            thread::sleep(Duration::from_millis(5));
        }
        thread::sleep(Duration::from_millis(30));
        pool.shutdown();
        assert_eq!(sizes.load(Ordering::SeqCst), sizes_after_structure);
        assert!(positions.load(Ordering::SeqCst) > 1);
        assert_eq!(pool.metrics().size_writes_during_animation, 0);
    }
}
