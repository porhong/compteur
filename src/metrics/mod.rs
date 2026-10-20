//! Metric collectors. Each one samples a single area of the system and returns plain data.

pub mod cpu_mem;
pub mod disk_net;
pub mod gpu;
pub mod processes;
pub mod thermal;

pub use cpu_mem::{CpuStats, MemStats};
pub use disk_net::{DiskStats, NetStats};
pub use gpu::GpuStats;
pub use processes::TopProcess;

/// The latest reading of every enabled metric. `None` means not yet known or unavailable.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub cpu: Option<CpuStats>,
    /// °C from ACPI thermal zones; `None` on PCs that expose none.
    pub cpu_temp: Option<f32>,
    pub memory: Option<MemStats>,
    pub gpu: Option<GpuStats>,
    pub disk: Option<DiskStats>,
    pub network: Option<NetStats>,
    /// Busiest apps first.
    pub processes: Option<Vec<TopProcess>>,
}
