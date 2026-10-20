use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::cleanup;

/// A metric that can be shown on the strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    Cpu,
    CpuTemp,
    Memory,
    Gpu,
    GpuTemp,
    Vram,
    Network,
    Disk,
    SystemDrive,
    TopProcess,
}

impl Metric {
    /// Menu order.
    pub const ALL: [Self; 10] = [
        Self::Cpu,
        Self::CpuTemp,
        Self::Memory,
        Self::Gpu,
        Self::GpuTemp,
        Self::Vram,
        Self::Network,
        Self::Disk,
        Self::SystemDrive,
        Self::TopProcess,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU load",
            Self::CpuTemp => "CPU temperature",
            Self::Memory => "RAM usage",
            Self::Gpu => "GPU load",
            Self::GpuTemp => "GPU temperature",
            Self::Vram => "GPU memory (VRAM)",
            Self::Network => "Network speed",
            Self::Disk => "Disk activity",
            Self::SystemDrive => "Drive C: space used",
            Self::TopProcess => "Top app by CPU",
        }
    }
}

/// Which metrics are on the strip. A struct rather than a list so the TOML reads `cpu = true`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shown {
    pub cpu: bool,
    pub cpu_temp: bool,
    pub memory: bool,
    pub gpu: bool,
    pub gpu_temp: bool,
    pub vram: bool,
    pub network: bool,
    pub disk: bool,
    pub system_drive: bool,
    pub top_process: bool,
}

impl Default for Shown {
    fn default() -> Self {
        Self {
            cpu: true,
            cpu_temp: true,
            memory: true,
            gpu: true,
            gpu_temp: true,
            vram: true,
            network: true,
            disk: true,
            // Off by default: free space changes slowly, so it's opt-in from the menu.
            system_drive: false,
            top_process: true,
        }
    }
}

impl Shown {
    fn flag(&mut self, metric: Metric) -> &mut bool {
        match metric {
            Metric::Cpu => &mut self.cpu,
            Metric::CpuTemp => &mut self.cpu_temp,
            Metric::Memory => &mut self.memory,
            Metric::Gpu => &mut self.gpu,
            Metric::GpuTemp => &mut self.gpu_temp,
            Metric::Vram => &mut self.vram,
            Metric::Network => &mut self.network,
            Metric::Disk => &mut self.disk,
            Metric::SystemDrive => &mut self.system_drive,
            Metric::TopProcess => &mut self.top_process,
        }
    }

    pub fn get(mut self, metric: Metric) -> bool {
        *self.flag(metric)
    }

    pub fn set(&mut self, metric: Metric, on: bool) {
        *self.flag(metric) = on;
    }

    pub fn count(self) -> usize {
        Metric::ALL.iter().filter(|m| self.get(**m)).count()
    }
}

/// A place on the strip. CPU load and temperature share one, as do GPU load and temperature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Slot {
    Cpu,
    Memory,
    Gpu,
    Vram,
    Network,
    Disk,
    SystemDrive,
    TopProcess,
    CleanButton,
    /// A name this version doesn't know (from a newer config); dropped when the config loads.
    #[serde(other)]
    Unknown,
}

impl Slot {
    /// Left to right.
    pub const DEFAULT_ORDER: [Self; 9] = [
        Self::Cpu,
        Self::Memory,
        Self::Gpu,
        Self::Vram,
        Self::Network,
        Self::Disk,
        Self::SystemDrive,
        Self::TopProcess,
        Self::CleanButton,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::Memory => "RAM",
            Self::Gpu => "GPU",
            Self::Vram => "VRAM",
            Self::Network => "Network",
            Self::Disk => "Disk activity",
            Self::SystemDrive => "Drive C:",
            Self::TopProcess => "Top app",
            Self::CleanButton => "Clean-up button",
            Self::Unknown => "?",
        }
    }
}

/// Keeps each known slot once, in the given order, and appends any that are missing (such as
/// ones added by a newer version), so the order always covers every slot.
fn normalize_order(order: &[Slot]) -> Vec<Slot> {
    let mut normalized: Vec<Slot> = Vec::with_capacity(Slot::DEFAULT_ORDER.len());
    for slot in order.iter().chain(&Slot::DEFAULT_ORDER) {
        if *slot != Slot::Unknown && !normalized.contains(slot) {
            normalized.push(*slot);
        }
    }
    normalized
}

/// User settings, persisted as TOML in `%APPDATA%\compteur\config\config.toml`.
///
/// `#[serde(default)]` lets older or hand-edited files load: missing keys fall back to defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Top-left corner of the strip in logical points. `None` lets Windows choose.
    pub position: Option<[f32; 2]>,
    /// Background opacity, 0.0–1.0.
    pub opacity: f32,
    /// How often metrics are sampled, in milliseconds.
    pub refresh_ms: u64,
    /// When set, the strip can't be dragged.
    pub locked: bool,
    /// When set, clicks pass through the strip to the window below.
    pub click_through: bool,
    pub shown: Shown,
    /// Show the one-click cleanup button at the end of the strip.
    pub show_clean_button: bool,
    /// Left-to-right order of the strip's segments and the cleanup button.
    pub order: Vec<Slot>,
    /// What the cleanup removes.
    pub cleanup: cleanup::Choices,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            position: None,
            opacity: 0.85,
            refresh_ms: 1000,
            locked: false,
            click_through: false,
            shown: Shown::default(),
            show_clean_button: true,
            order: Slot::DEFAULT_ORDER.to_vec(),
            cleanup: cleanup::Choices::default(),
        }
    }
}

impl Config {
    fn path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "compteur").map(|dirs| dirs.config_dir().join("config.toml"))
    }

    fn tmp_path(path: &Path) -> PathBuf {
        path.with_extension("toml.tmp")
    }

    /// Loads the config, falling back to defaults if the file is missing or unreadable.
    pub fn load() -> Self {
        let path = Self::path();
        if let Some(path) = &path {
            // Left behind only if the app died mid-save; the real config is still intact.
            let _ = fs::remove_file(Self::tmp_path(path));
        }
        path.and_then(|path| fs::read_to_string(path).ok()).map(|text| Self::from_toml(&text)).unwrap_or_default()
    }

    fn from_toml(text: &str) -> Self {
        let mut config: Self = toml::from_str(text).unwrap_or_default();
        config.opacity = config.opacity.clamp(0.2, 1.0);
        config.refresh_ms = config.refresh_ms.clamp(250, 5000);
        if config.shown.count() == 0 {
            config.shown = Shown::default(); // an empty strip would be invisible and unclickable
        }
        config.order = normalize_order(&config.order);
        config
    }

    /// Whether `slot` currently has anything on the strip.
    pub fn slot_shown(&self, slot: Slot) -> bool {
        let shown = self.shown;
        match slot {
            Slot::Cpu => shown.cpu || shown.cpu_temp,
            Slot::Memory => shown.memory,
            Slot::Gpu => shown.gpu || shown.gpu_temp,
            Slot::Vram => shown.vram,
            Slot::Network => shown.network,
            Slot::Disk => shown.disk,
            Slot::SystemDrive => shown.system_drive,
            Slot::TopProcess => shown.top_process,
            Slot::CleanButton => self.show_clean_button,
            Slot::Unknown => false,
        }
    }

    /// Writes to a temp file then renames it, so a crash never leaves a half-written config.
    pub fn save(&self) -> io::Result<()> {
        let path = Self::path().ok_or_else(|| io::Error::other("no config directory"))?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self).map_err(io::Error::other)?;
        let tmp = Self::tmp_path(&path);
        let result = fs::write(&tmp, text).and_then(|()| fs::rename(&tmp, path));
        if result.is_err() {
            let _ = fs::remove_file(tmp); // don't leave a stray file next to the config
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_toml() {
        let mut config = Config { position: Some([12.0, 34.0]), opacity: 0.5, refresh_ms: 500, locked: true, ..Config::default() };
        config.shown.set(Metric::Gpu, false);
        let text = toml::to_string_pretty(&config).unwrap();
        assert_eq!(Config::from_toml(&text), config);
    }

    #[test]
    fn corrupt_file_gives_defaults() {
        assert_eq!(Config::from_toml("this is = = not toml"), Config::default());
    }

    #[test]
    fn missing_keys_use_defaults() {
        let config = Config::from_toml("opacity = 0.6");
        assert_eq!(config.position, None);
        assert_eq!(config.opacity, 0.6);
    }

    #[test]
    fn all_metrics_hidden_resets_to_all_shown() {
        let mut shown = Shown::default();
        for metric in Metric::ALL {
            shown.set(metric, false);
        }
        let config = Config { shown, ..Config::default() };
        let text = toml::to_string_pretty(&config).unwrap();
        assert_eq!(Config::from_toml(&text).shown, Shown::default());
    }

    #[test]
    fn order_round_trips_and_is_repaired() {
        let mut config = Config::default();
        config.order.swap(0, 8); // cleanup button first, CPU last
        let text = toml::to_string_pretty(&config).unwrap();
        assert_eq!(Config::from_toml(&text).order, config.order);

        // Duplicates and unknown names are dropped; missing slots are appended in default order.
        let repaired = Config::from_toml(r#"order = ["clean_button", "gpu", "gpu", "from_the_future"]"#).order;
        assert_eq!(&repaired[..3], [Slot::CleanButton, Slot::Gpu, Slot::Cpu]);
        assert_eq!(repaired.len(), Slot::DEFAULT_ORDER.len());
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let config = Config::from_toml("opacity = 7.0\nrefresh_ms = 1");
        assert_eq!(config.opacity, 1.0);
        assert_eq!(config.refresh_ms, 250);
    }
}
