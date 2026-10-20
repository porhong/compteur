//! Compact, fixed-width text for the strip. Fixed widths keep the strip from jittering as values change.

/// How alarming a value is; the strip colors values by this. Ordered, so `max` picks the worse one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Normal,
    Warn,
    Hot,
}

impl Level {
    pub fn for_percent(percent: f32) -> Self {
        match percent {
            p if p >= 90.0 => Self::Hot,
            p if p >= 70.0 => Self::Warn,
            _ => Self::Normal,
        }
    }

    pub fn for_celsius(celsius: f32) -> Self {
        match celsius {
            c if c >= 90.0 => Self::Hot,
            c if c >= 75.0 => Self::Warn,
            _ => Self::Normal,
        }
    }
}

/// `" 7%"`, `"42%"`, `"100%"`; `"--%"` when unknown.
pub fn percent(value: Option<f32>) -> String {
    match value {
        Some(v) => format!("{:>2.0}%", v.clamp(0.0, 100.0)),
        None => "--%".to_owned(),
    }
}

/// `"45°"`; `"--°"` when unknown.
pub fn celsius(value: Option<f32>) -> String {
    value.map_or_else(|| "--°".to_owned(), |c| format!("{c:>2.0}°"))
}

/// Bytes with a binary unit suffix, at most 4 characters of number: `"512B"`, `"80K"`, `"9.8G"`, `"16G"`.
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 || value >= 10.0 {
        format!("{:.0}{}", value, UNITS[unit])
    } else {
        format!("{:.1}{}", value, UNITS[unit])
    }
}

/// A transfer rate padded to 4 characters, e.g. `"1.2M"`, `" 80K"`; `"  --"` when unknown.
pub fn rate(bytes_per_sec: Option<f64>) -> String {
    let text = bytes_per_sec.map_or_else(|| "--".to_owned(), |v| bytes(v.max(0.0).round() as u64));
    format!("{text:>4}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_formats_and_clamps() {
        assert_eq!(percent(Some(7.4)), " 7%");
        assert_eq!(percent(Some(42.6)), "43%");
        assert_eq!(percent(Some(130.0)), "100%");
        assert_eq!(percent(Some(-1.0)), " 0%");
        assert_eq!(percent(None), "--%");
    }

    #[test]
    fn bytes_picks_compact_units() {
        assert_eq!(bytes(0), "0B");
        assert_eq!(bytes(999), "999B");
        assert_eq!(bytes(1000), "1.0K");
        assert_eq!(bytes(80 * 1024), "80K");
        assert_eq!(bytes(1536 * 1024), "1.5M");
        assert_eq!(bytes(9_800 * 1024 * 1024), "9.6G");
        assert_eq!(bytes(16 * 1024 * 1024 * 1024), "16G");
    }

    #[test]
    fn rate_is_padded() {
        assert_eq!(rate(Some(80.0 * 1024.0)), " 80K");
        assert_eq!(rate(Some(1.25 * 1024.0 * 1024.0)), "1.2M");
        assert_eq!(rate(None), "  --");
    }

    #[test]
    fn levels_follow_thresholds() {
        assert_eq!(Level::for_percent(69.9), Level::Normal);
        assert_eq!(Level::for_percent(70.0), Level::Warn);
        assert_eq!(Level::for_percent(95.0), Level::Hot);
        assert_eq!(Level::for_celsius(74.0), Level::Normal);
        assert_eq!(Level::for_celsius(80.0), Level::Warn);
    }
}
