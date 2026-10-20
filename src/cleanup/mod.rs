//! One-click cleanup: frees disk space and RAM without touching what the user is working with.
//! Scans and cleans run on a worker thread; the UI reads the shared state and is woken on changes.

mod disk;
mod memory;

use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    thread,
    time::{Duration, Instant},
};

use eframe::egui;
use serde::{Deserialize, Serialize};

use crate::{format, platform};

/// Something the cleanup can free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    TempFiles,
    ErrorReports,
    BrowserCaches,
    ShaderCaches,
    RecycleBin,
    Memory,
}

impl Category {
    pub const ALL: [Self; 6] = [
        Self::TempFiles,
        Self::ErrorReports,
        Self::BrowserCaches,
        Self::ShaderCaches,
        Self::RecycleBin,
        Self::Memory,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::TempFiles => "Temporary files",
            Self::ErrorReports => "Error reports and crash dumps",
            Self::BrowserCaches => "Browser caches",
            Self::ShaderCaches => "Graphics shader caches",
            Self::RecycleBin => "Recycle Bin",
            Self::Memory => "Memory of background apps",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::TempFiles => "Files in your Temp folder older than a day. Newer ones may still be in use, so they stay.",
            Self::ErrorReports => "Windows Error Reporting files and app crash dumps, kept only for diagnostics.",
            Self::BrowserCaches => {
                "Cached web files of Chrome, Edge, Brave, Vivaldi and Firefox. A browser that is open is skipped. \
                 History, cookies and logins are never touched."
            }
            Self::ShaderCaches => {
                "DirectX and GPU driver caches. They are rebuilt automatically, but games may stutter briefly on \
                 their next start."
            }
            Self::RecycleBin => "Your deleted files. Emptying it can't be undone.",
            Self::Memory => {
                "Moves memory of apps without a window on screen out of RAM. Windows brings it back when an app \
                 needs it, so that app may pause briefly. Apps you can see are never touched."
            }
        }
    }
}

/// Which categories a clean includes. Defaults to what is safe with no downside.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Choices {
    pub temp_files: bool,
    pub error_reports: bool,
    pub browser_caches: bool,
    pub shader_caches: bool,
    pub recycle_bin: bool,
    pub memory: bool,
}

impl Default for Choices {
    fn default() -> Self {
        Self {
            temp_files: true,
            error_reports: true,
            browser_caches: true,
            shader_caches: false, // costs games a stutter when rebuilt
            recycle_bin: false,   // the user's own files, so only on request
            memory: true,
        }
    }
}

impl Choices {
    fn flag(&mut self, category: Category) -> &mut bool {
        match category {
            Category::TempFiles => &mut self.temp_files,
            Category::ErrorReports => &mut self.error_reports,
            Category::BrowserCaches => &mut self.browser_caches,
            Category::ShaderCaches => &mut self.shader_caches,
            Category::RecycleBin => &mut self.recycle_bin,
            Category::Memory => &mut self.memory,
        }
    }

    pub fn get(mut self, category: Category) -> bool {
        *self.flag(category)
    }

    pub fn set(&mut self, category: Category, on: bool) {
        *self.flag(category) = on;
    }

    pub fn any(self) -> bool {
        Category::ALL.iter().any(|c| self.get(*c))
    }
}

/// What one category holds (in a scan) or freed (in a clean).
#[derive(Debug, Clone, Default)]
pub struct Outcome {
    /// Bytes on disk; for `Memory`, the RAM in use.
    pub bytes: u64,
    /// Files that can't be deleted: in use or protected.
    pub kept: u64,
    /// Short remarks, such as a browser left alone because it is open.
    pub notes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Scan {
    pub at: Instant,
    pub found: Vec<(Category, Outcome)>,
}

impl Scan {
    pub fn get(&self, category: Category) -> Option<&Outcome> {
        self.found.iter().find(|(c, _)| *c == category).map(|(_, outcome)| outcome)
    }

    /// Disk space a clean with `choices` would free.
    pub fn disk_bytes(&self, choices: Choices) -> u64 {
        self.found
            .iter()
            .filter(|(c, _)| *c != Category::Memory && choices.get(*c))
            .map(|(_, outcome)| outcome.bytes)
            .sum()
    }
}

#[derive(Debug, Clone)]
pub struct Report {
    pub freed: u64,
    pub kept: u64,
    /// RAM in use before and after trimming.
    pub memory: Option<(u64, u64)>,
    pub notes: Vec<String>,
}

impl Report {
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![format!("Freed {} of disk space", format::bytes(self.freed))];
        if let Some((before, after)) = self.memory {
            lines.push(format!("RAM in use {} → {}", format::bytes(before), format::bytes(after)));
        }
        if self.kept > 0 {
            lines.push(format!("{} files were kept (in use or protected)", self.kept));
        }
        lines.extend(self.notes.iter().cloned());
        lines
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Task {
    Scanning,
    Cleaning,
}

#[derive(Default)]
struct State {
    busy: Option<Task>,
    scan: Option<Scan>,
    report: Option<Report>,
    /// Counts finished cleans, so the UI can tell a new report from one it has shown.
    cleans: u64,
}

/// Handle to the cleanup worker; cheap to clone.
#[derive(Clone)]
pub struct Cleaner {
    state: Arc<Mutex<State>>,
    ctx: egui::Context,
}

impl Cleaner {
    pub fn new(ctx: egui::Context) -> Self {
        Self { state: Arc::default(), ctx }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn busy(&self) -> Option<Task> {
        self.lock().busy
    }

    pub fn scan(&self) -> Option<Scan> {
        self.lock().scan.clone()
    }

    /// The latest clean's report with its sequence number (1 for the first clean).
    pub fn report(&self) -> Option<(u64, Report)> {
        let state = self.lock();
        state.report.clone().map(|report| (state.cleans, report))
    }

    /// Scans unless busy or a scan younger than `max_age` exists.
    pub fn scan_if_older(&self, max_age: Duration) {
        let fresh = self.lock().scan.as_ref().is_some_and(|scan| scan.at.elapsed() < max_age);
        if !fresh {
            self.run(Task::Scanning, || None);
        }
    }

    pub fn clean(&self, choices: Choices) {
        if choices.any() {
            self.run(Task::Cleaning, move || Some(clean(choices)));
        }
    }

    /// Runs `job` on a worker thread unless one is already running, then rescans so the sizes
    /// shown stay current.
    fn run(&self, task: Task, job: impl FnOnce() -> Option<Report> + Send + 'static) {
        {
            let mut state = self.lock();
            if state.busy.is_some() {
                return;
            }
            state.busy = Some(task);
        }
        self.ctx.request_repaint();
        let cleaner = self.clone();
        let spawned = thread::Builder::new().name("cleanup".into()).spawn(move || {
            let report = job();
            let scan = scan();
            let mut state = cleaner.lock();
            if let Some(report) = report {
                state.report = Some(report);
                state.cleans += 1;
            }
            state.scan = Some(scan);
            state.busy = None;
            drop(state);
            cleaner.ctx.request_repaint();
        });
        if spawned.is_err() {
            self.lock().busy = None;
        }
    }
}

fn scan() -> Scan {
    let running = disk::running_processes();
    let found = Category::ALL.iter().map(|&category| (category, measure(category, &running, false))).collect();
    Scan { at: Instant::now(), found }
}

fn clean(choices: Choices) -> Report {
    let running = disk::running_processes();
    let mut report = Report { freed: 0, kept: 0, memory: None, notes: Vec::new() };
    for category in Category::ALL.into_iter().filter(|c| *c != Category::Memory && choices.get(*c)) {
        let outcome = measure(category, &running, true);
        report.freed += outcome.bytes;
        report.kept += outcome.kept;
        report.notes.extend(outcome.notes);
    }
    if choices.memory {
        let before = platform::memory_in_use();
        memory::trim_background_apps();
        // Give Windows a moment to move the trimmed pages before measuring again.
        thread::sleep(Duration::from_millis(500));
        report.memory = before.zip(platform::memory_in_use());
    }
    report
}

/// Measures one category, deleting what it holds when `delete` is set.
fn measure(category: Category, running: &HashSet<String>, delete: bool) -> Outcome {
    let sweep_all = |roots: Vec<PathBuf>, min_age: Duration| {
        let mut tally = disk::Tally::default();
        for root in roots {
            tally += disk::sweep(&root, min_age, delete);
        }
        Outcome { bytes: tally.bytes, kept: tally.kept, notes: Vec::new() }
    };
    match category {
        Category::TempFiles => sweep_all(disk::temp_roots(), disk::TEMP_MIN_AGE),
        Category::ErrorReports => sweep_all(disk::error_report_roots(), Duration::ZERO),
        Category::ShaderCaches => sweep_all(disk::shader_cache_roots(), Duration::ZERO),
        Category::BrowserCaches => {
            let mut outcome = Outcome::default();
            for browser in disk::BROWSERS.iter().filter(|b| b.is_installed()) {
                if browser.is_running(running) {
                    outcome.notes.push(format!("{} is open, so its cache is left alone", browser.name));
                    continue;
                }
                let swept = sweep_all(browser.cache_roots(), Duration::ZERO);
                outcome.bytes += swept.bytes;
                outcome.kept += swept.kept;
            }
            outcome
        }
        Category::RecycleBin => {
            let Some((bytes, items)) = platform::recycle_bin_size() else { return Outcome::default() };
            if !delete {
                let notes = if items > 0 { vec![format!("{items} items")] } else { Vec::new() };
                return Outcome { bytes, kept: 0, notes };
            }
            let emptied = items == 0 || platform::empty_recycle_bin();
            Outcome { bytes: if emptied { bytes } else { 0 }, kept: 0, notes: Vec::new() }
        }
        Category::Memory => Outcome { bytes: platform::memory_in_use().unwrap_or(0), kept: 0, notes: Vec::new() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_leave_the_users_files_alone() {
        let choices = Choices::default();
        assert!(!choices.recycle_bin);
        assert!(!choices.shader_caches);
        assert!(choices.temp_files && choices.memory);
    }

    #[test]
    fn choices_round_trip_and_fill_missing_keys() {
        let mut choices = Choices::default();
        choices.set(Category::RecycleBin, true);
        let text = toml::to_string(&choices).unwrap();
        assert_eq!(toml::from_str::<Choices>(&text).unwrap(), choices);
        assert_eq!(toml::from_str::<Choices>("memory = false").unwrap(), Choices { memory: false, ..Choices::default() });
    }

    #[test]
    fn scan_totals_only_chosen_disk_categories() {
        let outcome = |bytes| Outcome { bytes, ..Outcome::default() };
        let scan = Scan {
            at: Instant::now(),
            found: vec![
                (Category::TempFiles, outcome(100)),
                (Category::RecycleBin, outcome(1000)),
                (Category::Memory, outcome(5000)),
            ],
        };
        assert_eq!(scan.disk_bytes(Choices::default()), 100);
    }
}
