//! The apps using the most CPU, grouped by executable name like Task Manager groups an app's processes.

use std::collections::HashMap;

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

/// How many apps the details list.
pub const TOP_COUNT: usize = 5;

#[derive(Debug, Clone, PartialEq)]
pub struct TopProcess {
    /// Executable name without `.exe`.
    pub name: String,
    /// Share of the whole CPU, 0–100 (sysinfo reports per core, so this is divided by core count).
    pub cpu: f32,
    pub memory: u64,
    /// Number of processes grouped under this name.
    pub count: usize,
}

/// Needs a previous call at least `MINIMUM_CPU_UPDATE_INTERVAL` ago for CPU usage to be meaningful.
pub fn sample_processes(sys: &mut System) -> Vec<TopProcess> {
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cpu().with_memory(),
    );
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as f32;
    let processes = sys
        .processes()
        .iter()
        .filter(|(pid, _)| **pid != Pid::from(0)) // "System Idle Process" is idle time, not work
        .map(|(_, p)| (p.name().to_string_lossy().into_owned(), p.cpu_usage() / cores, p.memory()));
    top_by_name(processes, TOP_COUNT)
}

fn top_by_name(processes: impl Iterator<Item = (String, f32, u64)>, count: usize) -> Vec<TopProcess> {
    let mut groups: HashMap<String, TopProcess> = HashMap::new();
    for (name, cpu, memory) in processes {
        let name = name.strip_suffix(".exe").map(str::to_owned).unwrap_or(name);
        let group =
            groups.entry(name.clone()).or_insert(TopProcess { name, cpu: 0.0, memory: 0, count: 0 });
        group.cpu += cpu;
        group.memory += memory;
        group.count += 1;
    }
    let mut top: Vec<TopProcess> = groups.into_values().collect();
    top.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then(b.memory.cmp(&a.memory)));
    top.truncate(count);
    top
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_by_name_and_sorts_by_cpu() {
        let processes = [
            ("chrome.exe".to_owned(), 5.0, 100),
            ("chrome.exe".to_owned(), 7.0, 200),
            ("code.exe".to_owned(), 10.0, 50),
            ("idle-ish.exe".to_owned(), 0.0, 999),
        ];
        let top = top_by_name(processes.into_iter(), 2);
        assert_eq!(
            top,
            [
                TopProcess { name: "chrome".into(), cpu: 12.0, memory: 300, count: 2 },
                TopProcess { name: "code".into(), cpu: 10.0, memory: 50, count: 1 },
            ]
        );
    }
}
