//! Metric collectors. Each one samples a single area of the system and returns plain data.

pub mod cpu_mem;
pub mod disk_net;

pub use cpu_mem::{CpuStats, MemStats};
pub use disk_net::{DiskStats, NetStats};

/// The latest reading of every enabled metric. `None` means not yet known or unavailable.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub cpu: Option<CpuStats>,
    pub memory: Option<MemStats>,
    pub disk: Option<DiskStats>,
    pub network: Option<NetStats>,
}
