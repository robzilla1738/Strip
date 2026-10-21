//! Commands wait on the main thread. Subscribers receive events as lines.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Duration;

use strip_ipc::ClientMessage;

pub struct Job {
    pub message: ClientMessage,
    pub reply: Sender<String>,
}

struct Subscriber {
    id: u64,
    events: Vec<String>,
    tx: Sender<String>,
}

pub struct Bus {
    jobs: Mutex<Vec<Job>>,
    wake: Condvar,
    subscribers: Mutex<Vec<Subscriber>>,
    next_id: Mutex<u64>,
}

impl Bus {
    pub fn new() -> Self {
        Self {
            jobs: Mutex::new(Vec::new()),
            wake: Condvar::new(),
            subscribers: Mutex::new(Vec::new()),
            next_id: Mutex::new(1),
        }
    }

    pub fn post(&self, message: ClientMessage) -> Receiver<String> {
        let (reply, rx) = mpsc::channel();
        lock(&self.jobs).push(Job { message, reply });
        self.wake.notify_one();
        crate::hotkeys::wake_main();
        rx
    }

    pub fn poll(&self, timeout: Duration) -> Vec<Job> {
        let mut jobs = lock(&self.jobs);
        if jobs.is_empty() {
            let (next, _) = self.wake.wait_timeout(jobs, timeout).unwrap_or_else(|err| err.into_inner());
            jobs = next;
        }
        std::mem::take(&mut *jobs)
    }

    pub fn subscribe(&self, events: Vec<String>) -> (u64, Receiver<String>) {
        let (tx, rx) = mpsc::channel();
        let mut next = lock(&self.next_id);
        let id = *next;
        *next += 1;
        drop(next);
        lock(&self.subscribers).push(Subscriber { id, events, tx });
        (id, rx)
    }

    pub fn unsubscribe(&self, id: u64) {
        lock(&self.subscribers).retain(|subscriber| subscriber.id != id);
    }

    pub fn emit(&self, event: &str, data: serde_json::Value) {
        let line = strip_ipc::encode(&strip_ipc::ServerMessage::Event { event: event.to_string(), data });
        let line = line.trim_end().to_string();
        for subscriber in lock(&self.subscribers).iter() {
            if subscriber.events.iter().any(|name| name == event) {
                let _ = subscriber.tx.send(line.clone());
            }
        }
    }
}

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}
