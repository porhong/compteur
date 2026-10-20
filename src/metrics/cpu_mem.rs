use sysinfo::System;

#[derive(Debug, Clone)]
pub struct CpuStats {
    /// Average usage across all logical cores, 0–100.
    pub usage: f32,
    pub per_core: Vec<f32>,
    /// Current clock of the first core in MHz (Windows reports one value for all cores).
    pub frequency_mhz: u64,
}

#[derive(Debug, Clone)]
pub struct MemStats {
    pub used: u64,
    pub total: u64,
    pub swap_used: u64,
    pub swap_total: u64,
}

impl MemStats {
    pub fn percent(&self) -> f32 {
        if self.total == 0 { 0.0 } else { self.used as f32 / self.total as f32 * 100.0 }
    }
}

/// Needs a previous `refresh_cpu_usage` at least `MINIMUM_CPU_UPDATE_INTERVAL` ago to be meaningful.
pub fn sample_cpu(sys: &mut System) -> CpuStats {
    sys.refresh_cpu_usage();
    sys.refresh_cpu_frequency();
    let cpus = sys.cpus();
    CpuStats {
        usage: sys.global_cpu_usage(),
        per_core: cpus.iter().map(|cpu| cpu.cpu_usage()).collect(),
        frequency_mhz: cpus.first().map_or(0, |cpu| cpu.frequency()),
    }
}

pub fn sample_memory(sys: &mut System) -> MemStats {
    sys.refresh_memory();
    MemStats {
        used: sys.used_memory(),
        total: sys.total_memory(),
        swap_used: sys.used_swap(),
        swap_total: sys.total_swap(),
    }
}
