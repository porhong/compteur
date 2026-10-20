use std::time::{Duration, Instant};

use sysinfo::{DiskRefreshKind, Disks, Networks};

use crate::pdh::{Counter, Query};

#[derive(Debug, Clone)]
pub struct DiskStats {
    pub read_per_sec: Option<f64>,
    pub write_per_sec: Option<f64>,
    /// Percent of time any physical disk was busy (Task Manager's "Active time"), 0–100.
    pub active_percent: Option<f32>,
    pub volumes: Vec<Volume>,
}

#[derive(Debug, Clone)]
pub struct Volume {
    pub mount: String,
    pub used: u64,
    pub total: u64,
}

#[derive(Debug, Clone)]
pub struct NetStats {
    pub down_per_sec: f64,
    pub up_per_sec: f64,
    pub adapters: Vec<Adapter>,
}

#[derive(Debug, Clone)]
pub struct Adapter {
    pub name: String,
    pub down_per_sec: f64,
    pub up_per_sec: f64,
}

/// Disk space changes slowly, so the volume list is re-read only this often.
const VOLUME_REFRESH: Duration = Duration::from_secs(30);

pub struct DiskSampler {
    counters: Option<DiskCounters>,
    volumes: Vec<Volume>,
    volumes_read_at: Option<Instant>,
}

struct DiskCounters {
    query: Query,
    read: Counter,
    write: Counter,
    idle: Counter,
}

impl DiskSampler {
    pub fn new() -> Self {
        let counters = Query::new().and_then(|query| {
            let read = query.add(r"\PhysicalDisk(_Total)\Disk Read Bytes/sec")?;
            let write = query.add(r"\PhysicalDisk(_Total)\Disk Write Bytes/sec")?;
            let idle = query.add(r"\PhysicalDisk(_Total)\% Idle Time")?;
            query.collect(); // baseline for the rate counters
            Some(DiskCounters { query, read, write, idle })
        });
        Self { counters, volumes: Vec::new(), volumes_read_at: None }
    }

    pub fn sample(&mut self) -> DiskStats {
        if self.volumes_read_at.is_none_or(|at| at.elapsed() >= VOLUME_REFRESH) {
            self.volumes = read_volumes();
            self.volumes_read_at = Some(Instant::now());
        }

        let (mut read, mut write, mut active) = (None, None, None);
        if let Some(c) = &self.counters
            && c.query.collect()
        {
            read = c.query.value(c.read);
            write = c.query.value(c.write);
            // `_Total` idle time is averaged over disks; clamp because PDH can overshoot slightly.
            active = c.query.value(c.idle).map(|idle| (100.0 - idle).clamp(0.0, 100.0) as f32);
        }
        DiskStats { read_per_sec: read, write_per_sec: write, active_percent: active, volumes: self.volumes.clone() }
    }
}

fn read_volumes() -> Vec<Volume> {
    let disks = Disks::new_with_refreshed_list_specifics(DiskRefreshKind::nothing().with_storage());
    let mut volumes: Vec<Volume> = disks
        .list()
        .iter()
        .filter(|disk| disk.total_space() > 0)
        .map(|disk| Volume {
            mount: disk.mount_point().display().to_string(),
            used: disk.total_space().saturating_sub(disk.available_space()),
            total: disk.total_space(),
        })
        .collect();
    volumes.sort_by(|a, b| a.mount.cmp(&b.mount));
    volumes
}

pub struct NetSampler {
    networks: Networks,
    last: Instant,
}

impl NetSampler {
    pub fn new() -> Self {
        Self { networks: Networks::new_with_refreshed_list(), last: Instant::now() }
    }

    pub fn sample(&mut self) -> NetStats {
        // `true` also drops adapters that disappeared; new ones are picked up by the refresh.
        self.networks.refresh(true);
        let now = Instant::now();
        let secs = now.duration_since(self.last).as_secs_f64().max(0.001);
        self.last = now;

        let mut adapters: Vec<Adapter> = self
            .networks
            .list()
            .iter()
            .filter(|(name, data)| is_physical(name, data.mac_address().is_unspecified()))
            .map(|(name, data)| Adapter {
                name: name.clone(),
                down_per_sec: data.received() as f64 / secs,
                up_per_sec: data.transmitted() as f64 / secs,
            })
            .collect();
        adapters.sort_by(|a, b| a.name.cmp(&b.name));
        NetStats {
            down_per_sec: adapters.iter().map(|a| a.down_per_sec).sum(),
            up_per_sec: adapters.iter().map(|a| a.up_per_sec).sum(),
            adapters,
        }
    }
}

/// Skips loopback and virtual switches, whose traffic mirrors a physical adapter's and would count twice.
fn is_physical(name: &str, mac_unspecified: bool) -> bool {
    const VIRTUAL: [&str; 6] = ["loopback", "vethernet", "virtualbox", "vmware", "hyper-v", "wsl"];
    let lower = name.to_lowercase();
    !mac_unspecified && !VIRTUAL.iter().any(|v| lower.contains(v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_virtual_adapters() {
        assert!(is_physical("Wi-Fi", false));
        assert!(is_physical("Ethernet 2", false));
        assert!(!is_physical("vEthernet (WSL)", false));
        assert!(!is_physical("Loopback Pseudo-Interface 1", false));
        assert!(!is_physical("Ethernet", true));
    }
}
