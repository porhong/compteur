//! Background thread that samples metrics and wakes the UI only when new data arrives.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use eframe::egui;
use sysinfo::System;

use crate::config::{Config, Shown};
use crate::metrics::{
    Snapshot, cpu_mem,
    disk_net::{DiskSampler, NetSampler},
    gpu::GpuSampler,
    processes,
    thermal::ThermalSampler,
};

/// Seconds of history kept for sparklines (one sample per tick).
pub const HISTORY_LEN: usize = 60;
/// Listing every process costs ~10 ms, so the top-apps list refreshes less often than the rest.
const PROCESS_INTERVAL: Duration = Duration::from_secs(3);

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

/// What the sampler should do; written by the UI, read by the sampler at every tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub interval: Duration,
    pub shown: Shown,
}

impl Settings {
    pub fn from_config(config: &Config) -> Self {
        let interval = Duration::from_millis(config.refresh_ms).max(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
        Self { interval, shown: config.shown }
    }
}

/// State shared between the sampler thread and the UI.
pub struct Shared {
    pub settings: Settings,
    pub snapshot: Snapshot,
    pub cpu_history: History,
    pub memory_history: History,
    pub gpu_history: History,
}

impl Shared {
    fn new(settings: Settings) -> Self {
        Self {
            settings,
            snapshot: Snapshot::default(),
            cpu_history: History::new(HISTORY_LEN),
            memory_history: History::new(HISTORY_LEN),
            gpu_history: History::new(HISTORY_LEN),
        }
    }
}

/// Starts the sampler. The thread lives for the whole process, so it is never joined.
pub fn spawn(ctx: egui::Context, settings: Settings) -> Arc<Mutex<Shared>> {
    let shared = Arc::new(Mutex::new(Shared::new(settings)));
    let thread_shared = Arc::clone(&shared);

    thread::Builder::new()
        .name("sampler".into())
        .spawn(move || {
            let mut sys = System::new();
            // CPU usage is a delta between two refreshes; take the baseline now.
            sys.refresh_cpu_usage();
            let mut disk_sampler = DiskSampler::new();
            let mut net_sampler = NetSampler::new();
            let mut gpu_sampler = GpuSampler::new();
            let mut thermal_sampler = ThermalSampler::new();
            let mut processes: Option<(Instant, Vec<processes::TopProcess>)> = None;
            loop {
                let Settings { interval, shown } = lock(&thread_shared).settings;
                thread::sleep(interval);

                // Hidden metrics aren't sampled at all; GPU counters in particular cost CPU time.
                // Sample outside the lock, so the UI never waits on a slow system call.
                let cpu = shown.cpu.then(|| cpu_mem::sample_cpu(&mut sys));
                let cpu_temp = shown.cpu_temp.then(|| thermal_sampler.sample()).flatten();
                let memory = shown.memory.then(|| cpu_mem::sample_memory(&mut sys));
                let gpu = (shown.gpu || shown.vram || shown.gpu_temp).then(|| gpu_sampler.sample()).flatten();
                let network = shown.network.then(|| net_sampler.sample());
                let disk = shown.disk.then(|| disk_sampler.sample());
                if !shown.top_process {
                    processes = None;
                } else if processes.as_ref().is_none_or(|(at, _)| at.elapsed() >= PROCESS_INTERVAL) {
                    processes = Some((Instant::now(), processes::sample_processes(&mut sys)));
                }

                let mut shared = lock(&thread_shared);
                if let Some(cpu) = &cpu {
                    shared.cpu_history.push(cpu.usage);
                }
                if let Some(memory) = &memory {
                    shared.memory_history.push(memory.percent());
                }
                if let Some(usage) = gpu.as_ref().and_then(|g| g.usage()) {
                    shared.gpu_history.push(usage);
                }
                let processes = processes.as_ref().map(|(_, top)| top.clone());
                shared.snapshot = Snapshot { cpu, cpu_temp, memory, gpu, disk, network, processes };
                drop(shared);
                ctx.request_repaint();
            }
        })
        .expect("failed to spawn sampler thread");

    shared
}

/// Locks the shared state, recovering it if a panicking thread poisoned the lock.
pub fn lock(shared: &Mutex<Shared>) -> std::sync::MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
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

