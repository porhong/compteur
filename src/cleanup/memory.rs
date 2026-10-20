//! Trims the memory of apps the user isn't looking at. Windows moves those pages out of RAM and
//! brings them back on demand, so a trimmed app may pause briefly when it is next used.

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

use crate::platform;

/// PIDs below this are the System Idle Process and System, which are never trimmed.
const FIRST_USER_PID: u32 = 8;

/// Trims every process without an on-screen window, except this app. Returns how many were
/// trimmed; system and elevated processes can't be opened without admin rights and are skipped.
pub fn trim_background_apps() -> usize {
    let on_screen = platform::on_screen_process_ids();
    let own = std::process::id();
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    sys.processes()
        .keys()
        .map(|pid| pid.as_u32())
        .filter(|pid| *pid >= FIRST_USER_PID && *pid != own && !on_screen.contains(pid))
        .filter(|pid| platform::trim_working_set(*pid))
        .count()
}
