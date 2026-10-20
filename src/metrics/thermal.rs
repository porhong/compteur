//! CPU temperature from ACPI thermal zones.
//!
//! Readable without admin rights, but many desktop boards expose no zone at all, and a zone is a
//! board sensor near the CPU rather than the die itself. Die temperature needs a kernel driver,
//! which this app deliberately avoids.

use crate::pdh::{Counter, Query};

const KELVIN_OFFSET: f64 = 273.15;

pub struct ThermalSampler {
    counter: Option<(Query, Counter)>,
}

impl ThermalSampler {
    pub fn new() -> Self {
        let counter = Query::new().and_then(|query| {
            // Tenths of a kelvin.
            let counter = query.add(r"\Thermal Zone Information(*)\High Precision Temperature")?;
            Some((query, counter))
        });
        Self { counter }
    }

    /// Hottest zone in °C, or `None` when this PC exposes no thermal zone.
    pub fn sample(&mut self) -> Option<f32> {
        let (query, counter) = self.counter.as_ref()?;
        if !query.collect() {
            return None;
        }
        query
            .values(*counter)
            .into_iter()
            .map(|(_, tenths_kelvin)| tenths_kelvin / 10.0 - KELVIN_OFFSET)
            .filter(|celsius| *celsius > 0.0) // unpopulated zones report 0 K
            .reduce(f64::max)
            .map(|celsius| celsius as f32)
    }
}
