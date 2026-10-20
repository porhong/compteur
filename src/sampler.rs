//! Background thread that samples metrics and wakes the UI only when new data arrives.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use eframe::egui;
use sysinfo::System;

use crate::metrics::{
    Snapshot, cpu_mem,
    disk_net::{DiskSampler, NetSampler},
};

/// Seconds of history kept for sparklines (one sample per tick).
pub const HISTORY_LEN: usize = 60;

/// Fixed-capacity buffer that drops the oldest value when full.
#[derive(Debug, Clone)]
pub struct History {
    values: VecDeque<f32>,
    capacity: usize,
}

impl History {
    pub fn new(capacity: usize) -> Self {
        Self { values: VecDeque::with_capacity(capacity), capacity }
    }

    pub fn push(&mut self, value: f32) {
        if self.values.len() == self.capacity {
            self.values.pop_front();
        }
        self.values.push_back(value);
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = f32> + '_ {
        self.values.iter().copied()
    }
}

/// State shared between the sampler thread and the UI.
pub struct Shared {
    pub snapshot: Snapshot,
    pub cpu_history: History,
    pub memory_history: History,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            snapshot: Snapshot::default(),
            cpu_history: History::new(HISTORY_LEN),
            memory_history: History::new(HISTORY_LEN),
        }
    }
}

/// Starts the sampler. The thread lives for the whole process, so it is never joined.
pub fn spawn(ctx: egui::Context, interval: Duration) -> Arc<Mutex<Shared>> {
    let shared = Arc::new(Mutex::new(Shared::default()));
    let interval = interval.max(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    let thread_shared = Arc::clone(&shared);

    thread::Builder::new()
        .name("sampler".into())
        .spawn(move || {
            let mut sys = System::new();
            // CPU usage is a delta between two refreshes; take the baseline now.
            sys.refresh_cpu_usage();
            let mut disk_sampler = DiskSampler::new();
            let mut net_sampler = NetSampler::new();
            loop {
                thread::sleep(interval);
                let cpu = cpu_mem::sample_cpu(&mut sys);
                let memory = cpu_mem::sample_memory(&mut sys);
                let disk = disk_sampler.sample();
                let network = net_sampler.sample();

                // Sample outside the lock, so the UI never waits on a slow system call.
                let mut shared = thread_shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                shared.cpu_history.push(cpu.usage);
                shared.memory_history.push(memory.percent());
                shared.snapshot =
                    Snapshot { cpu: Some(cpu), memory: Some(memory), disk: Some(disk), network: Some(network) };
                drop(shared);
                ctx.request_repaint();
            }
        })
        .expect("failed to spawn sampler thread");

    shared
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_drops_oldest_when_full() {
        let mut history = History::new(3);
        for v in [1.0, 2.0, 3.0, 4.0] {
            history.push(v);
        }
        assert_eq!(history.iter().collect::<Vec<_>>(), [2.0, 3.0, 4.0]);
    }
}
