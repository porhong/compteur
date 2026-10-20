use std::{fs, io, path::PathBuf};

use serde::{Deserialize, Serialize};

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
    TopProcess,
}

impl Metric {
    /// Menu order.
    pub const ALL: [Self; 9] = [
        Self::Cpu,
        Self::CpuTemp,
        Self::Memory,
        Self::Gpu,
        Self::GpuTemp,
        Self::Vram,
        Self::Network,
        Self::Disk,
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
    pub shown: Shown,
}

impl Default for Config {
    fn default() -> Self {
        Self { position: None, opacity: 0.85, refresh_ms: 1000, locked: false, shown: Shown::default() }
    }
}

impl Config {
    fn path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "compteur").map(|dirs| dirs.config_dir().join("config.toml"))
    }

    /// Loads the config, falling back to defaults if the file is missing or unreadable.
    pub fn load() -> Self {
        Self::path()
            .and_then(|path| fs::read_to_string(path).ok())
            .map(|text| Self::from_toml(&text))
            .unwrap_or_default()
    }

    fn from_toml(text: &str) -> Self {
        let mut config: Self = toml::from_str(text).unwrap_or_default();
        config.opacity = config.opacity.clamp(0.2, 1.0);
        config.refresh_ms = config.refresh_ms.clamp(250, 5000);
        if config.shown.count() == 0 {
            config.shown = Shown::default(); // an empty strip would be invisible and unclickable
        }
        config
    }

    /// Writes to a temp file then renames it, so a crash never leaves a half-written config.
    pub fn save(&self) -> io::Result<()> {
        let path = Self::path().ok_or_else(|| io::Error::other("no config directory"))?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self).map_err(io::Error::other)?;
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, text)?;
        fs::rename(tmp, path)
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
    fn out_of_range_values_are_clamped() {
        let config = Config::from_toml("opacity = 7.0\nrefresh_ms = 1");
        assert_eq!(config.opacity, 1.0);
        assert_eq!(config.refresh_ms, 250);
    }
}
