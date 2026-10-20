use std::{fs, io, path::PathBuf};

use serde::{Deserialize, Serialize};

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
}

impl Default for Config {
    fn default() -> Self {
        Self { position: None, opacity: 0.85, refresh_ms: 1000 }
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
        let config = Config { position: Some([12.0, 34.0]), opacity: 0.5, refresh_ms: 500 };
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
    fn out_of_range_values_are_clamped() {
        let config = Config::from_toml("opacity = 7.0\nrefresh_ms = 1");
        assert_eq!(config.opacity, 1.0);
        assert_eq!(config.refresh_ms, 250);
    }
}
