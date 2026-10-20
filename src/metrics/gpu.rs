//! Vendor-neutral GPU load and VRAM, the same way Task Manager computes them.
//!
//! Load comes from `\GPU Engine(*)\Utilization Percentage`, whose instances look like
//! `pid_1124_luid_0x00000000_0x0000D1E8_phys_0_eng_3_engtype_VideoDecode`: one per process per engine.
//! An engine's load is the sum over processes; an adapter's load is its busiest engine.
//! Names and total VRAM come from DXGI, matched to the counters by adapter LUID.
//! Temperature comes from the kernel-mode adapter performance data that Task Manager also shows.

use std::{collections::HashMap, ffi::c_void};

use windows::{
    Wdk::Graphics::Direct3D::{
        D3DKMT_ADAPTER_PERFDATA, D3DKMT_CLOSEADAPTER, D3DKMT_OPENADAPTERFROMLUID, D3DKMT_QUERYADAPTERINFO,
        D3DKMTCloseAdapter, D3DKMTOpenAdapterFromLuid, D3DKMTQueryAdapterInfo, KMTQAITYPE_ADAPTERPERFDATA,
    },
    Win32::{
        Foundation::LUID,
        Graphics::Dxgi::{CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, IDXGIFactory1},
    },
};

use crate::pdh::{Counter, Query};

#[derive(Debug, Clone)]
pub struct GpuStats {
    pub adapters: Vec<GpuAdapter>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GpuAdapter {
    pub name: String,
    /// Load of the busiest engine, 0–100.
    pub usage: f32,
    /// Load per engine type (`3D`, `Copy`, `VideoDecode`, …), busiest first.
    pub engines: Vec<(String, f32)>,
    pub vram_used: u64,
    /// Dedicated VRAM; 0 when unknown.
    pub vram_total: u64,
    /// Degrees Celsius; `None` when the driver doesn't report it (common for integrated GPUs).
    pub temperature: Option<f32>,
}

impl GpuStats {
    /// Load of the busiest adapter.
    pub fn usage(&self) -> Option<f32> {
        self.adapters.iter().map(|a| a.usage).reduce(f32::max)
    }

    /// Temperature of the hottest adapter that reports one.
    pub fn temperature(&self) -> Option<f32> {
        self.adapters.iter().filter_map(|a| a.temperature).reduce(f32::max)
    }

    /// The adapter with the most dedicated memory, normally the discrete GPU.
    pub fn main_adapter(&self) -> Option<&GpuAdapter> {
        self.adapters.iter().max_by_key(|a| a.vram_total)
    }
}

impl GpuAdapter {
    pub fn vram_percent(&self) -> Option<f32> {
        (self.vram_total > 0).then(|| self.vram_used as f32 / self.vram_total as f32 * 100.0)
    }
}

/// An adapter reported by DXGI.
#[derive(Debug, Clone)]
struct KnownAdapter {
    luid: u64,
    name: String,
    vram_total: u64,
}

pub struct GpuSampler {
    counters: Option<(Query, Counter, Counter)>,
    known: Vec<KnownAdapter>,
    /// One per `known` adapter, for temperature.
    kernel_adapters: Vec<Option<KernelAdapter>>,
}

impl GpuSampler {
    pub fn new() -> Self {
        let known = known_adapters();
        // No hardware adapter means nothing to report; skip opening the counters.
        let counters = (!known.is_empty()).then(Query::new).flatten().and_then(|query| {
            let engines = query.add(r"\GPU Engine(*)\Utilization Percentage")?;
            let memory = query.add(r"\GPU Adapter Memory(*)\Dedicated Usage")?;
            query.collect(); // baseline for the utilization rate
            Some((query, engines, memory))
        });
        let kernel_adapters = known.iter().map(|a| KernelAdapter::open(a.luid)).collect();
        Self { counters, known, kernel_adapters }
    }

    /// `None` when there is no hardware GPU or its counters are unavailable (needs a WDDM 2 driver).
    pub fn sample(&mut self) -> Option<GpuStats> {
        let (query, engines, memory) = self.counters.as_ref()?;
        if !query.collect() {
            return None;
        }
        let mut adapters = aggregate(&query.values(*engines), &query.values(*memory), &self.known);
        for (adapter, kernel) in adapters.iter_mut().zip(&self.kernel_adapters) {
            adapter.temperature = kernel.as_ref().and_then(KernelAdapter::temperature);
        }
        Some(GpuStats { adapters })
    }
}

/// A kernel-mode (D3DKMT) adapter handle, used only to read the temperature.
struct KernelAdapter(u32);

impl KernelAdapter {
    fn open(luid: u64) -> Option<Self> {
        let luid = LUID { LowPart: luid as u32, HighPart: (luid >> 32) as i32 };
        let mut open = D3DKMT_OPENADAPTERFROMLUID { AdapterLuid: luid, hAdapter: 0 };
        // SAFETY: `open` is a valid, initialized in/out struct.
        let status = unsafe { D3DKMTOpenAdapterFromLuid(&mut open) };
        status.is_ok().then_some(Self(open.hAdapter))
    }

    fn temperature(&self) -> Option<f32> {
        let mut perf = D3DKMT_ADAPTER_PERFDATA::default(); // PhysicalAdapterIndex 0
        let mut query = D3DKMT_QUERYADAPTERINFO {
            hAdapter: self.0,
            Type: KMTQAITYPE_ADAPTERPERFDATA,
            pPrivateDriverData: (&raw mut perf).cast::<c_void>(),
            PrivateDriverDataSize: size_of::<D3DKMT_ADAPTER_PERFDATA>() as u32,
        };
        // SAFETY: the handle is open and `query` points at a correctly sized PERFDATA buffer.
        let status = unsafe { D3DKMTQueryAdapterInfo(&mut query) };
        // Reported in tenths of a degree Celsius; 0 means the driver doesn't support it.
        (status.is_ok() && perf.Temperature > 0).then(|| perf.Temperature as f32 / 10.0)
    }
}

impl Drop for KernelAdapter {
    fn drop(&mut self) {
        // SAFETY: the handle is open and closed exactly once. Nothing useful can be done if closing fails.
        let _ = unsafe { D3DKMTCloseAdapter(&D3DKMT_CLOSEADAPTER { hAdapter: self.0 }) };
    }
}

/// Hardware adapters from DXGI, skipping software renderers such as Microsoft Basic Render Driver.
fn known_adapters() -> Vec<KnownAdapter> {
    // SAFETY: plain COM factory creation and queries; no pointers are retained.
    let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else { return Vec::new() };
    let mut adapters = Vec::new();
    for index in 0.. {
        // SAFETY: EnumAdapters1 fails with DXGI_ERROR_NOT_FOUND past the last adapter, ending the loop.
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(index) }) else { break };
        // SAFETY: `adapter` is a valid interface returned above.
        let Ok(desc) = (unsafe { adapter.GetDesc1() }) else { continue };
        if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
            continue;
        }
        let name_len = desc.Description.iter().position(|&c| c == 0).unwrap_or(desc.Description.len());
        let luid = (u64::from(desc.AdapterLuid.HighPart as u32) << 32) | u64::from(desc.AdapterLuid.LowPart);
        // The same GPU can be listed once per output; keep the first.
        if adapters.iter().all(|a: &KnownAdapter| a.luid != luid) {
            adapters.push(KnownAdapter {
                luid,
                name: String::from_utf16_lossy(&desc.Description[..name_len]).trim().to_owned(),
                vram_total: desc.DedicatedVideoMemory as u64,
            });
        }
    }
    adapters
}

/// Parses the `luid_0xHIGH_0xLOW` part of an instance name, returning the LUID and the text after it.
fn parse_luid(name: &str) -> Option<(u64, &str)> {
    let rest = &name[name.find("luid_")? + "luid_".len()..];
    let mut parts = rest.splitn(3, '_');
    let high = u32::from_str_radix(parts.next()?.strip_prefix("0x")?, 16).ok()?;
    let low = u32::from_str_radix(parts.next()?.strip_prefix("0x")?, 16).ok()?;
    Some(((u64::from(high) << 32) | u64::from(low), parts.next().unwrap_or("")))
}

/// Identity of one engine: adapter LUID, physical GPU index and engine index.
type EngineKey = (u64, u32, u32);

/// `pid_.._luid_.._phys_0_eng_3_engtype_VideoDecode` → `((luid, 0, 3), "VideoDecode")`.
fn parse_engine(name: &str) -> Option<(EngineKey, &str)> {
    let (luid, rest) = parse_luid(name)?;
    let rest = rest.strip_prefix("phys_")?;
    let (phys, rest) = rest.split_once("_eng_")?;
    let (engine, kind) = rest.split_once("_engtype_")?;
    Some(((luid, phys.parse().ok()?, engine.parse().ok()?), kind))
}

fn aggregate(engines: &[(String, f64)], memory: &[(String, f64)], known: &[KnownAdapter]) -> Vec<GpuAdapter> {
    // Sum each engine's load over all processes using it.
    let mut engine_load: HashMap<EngineKey, (&str, f64)> = HashMap::new();
    for (name, value) in engines {
        if let Some((key, kind)) = parse_engine(name) {
            engine_load.entry(key).or_insert((kind, 0.0)).1 += value;
        }
    }

    let mut vram_used: HashMap<u64, u64> = HashMap::new();
    for (name, value) in memory {
        if let Some((luid, _)) = parse_luid(name) {
            *vram_used.entry(luid).or_default() += *value as u64;
        }
    }

    known
        .iter()
        .map(|adapter| {
            let mut by_kind: HashMap<&str, f64> = HashMap::new();
            for ((luid, ..), (kind, load)) in &engine_load {
                if *luid == adapter.luid {
                    let entry = by_kind.entry(kind).or_default();
                    *entry = entry.max(*load);
                }
            }
            let mut kinds: Vec<(String, f32)> =
                by_kind.into_iter().map(|(kind, load)| (kind.to_owned(), load.clamp(0.0, 100.0) as f32)).collect();
            kinds.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            GpuAdapter {
                name: adapter.name.clone(),
                usage: kinds.first().map_or(0.0, |(_, load)| *load),
                engines: kinds,
                vram_used: vram_used.get(&adapter.luid).copied().unwrap_or(0),
                vram_total: adapter.vram_total,
                temperature: None, // filled in by the sampler
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LUID: u64 = 0xD1E8;

    fn engine(pid: u32, eng: u32, kind: &str, value: f64) -> (String, f64) {
        (format!("pid_{pid}_luid_0x00000000_0x0000D1E8_phys_0_eng_{eng}_engtype_{kind}"), value)
    }

    #[test]
    fn parses_engine_instance_names() {
        let name = "pid_1124_luid_0x00000001_0x0000D1E8_phys_0_eng_3_engtype_VideoDecode";
        assert_eq!(parse_engine(name), Some(((0x1_0000_D1E8, 0, 3), "VideoDecode")));
        // Engine types can themselves contain underscores.
        let name = "pid_8_luid_0x00000000_0x0000D1E8_phys_1_eng_12_engtype_Compute_0";
        assert_eq!(parse_engine(name), Some(((LUID, 1, 12), "Compute_0")));
        assert_eq!(parse_engine("garbage"), None);
    }

    #[test]
    fn parses_memory_instance_names() {
        assert_eq!(parse_luid("luid_0x00000000_0x0000D1E8_phys_0").map(|(luid, _)| luid), Some(LUID));
    }

    #[test]
    fn sums_processes_per_engine_and_takes_busiest_engine() {
        let engines = [
            engine(1, 0, "3D", 20.0),
            engine(2, 0, "3D", 15.0), // same engine, another process: 35% total
            engine(1, 3, "VideoDecode", 50.0),
            engine(1, 1, "Copy", 5.0),
        ];
        let memory = [("luid_0x00000000_0x0000D1E8_phys_0".to_owned(), 2.0 * 1024.0 * 1024.0 * 1024.0)];
        let known = [KnownAdapter { luid: LUID, name: "Test GPU".into(), vram_total: 8 << 30 }];

        let adapters = aggregate(&engines, &memory, &known);
        assert_eq!(adapters.len(), 1);
        let gpu = &adapters[0];
        assert_eq!(gpu.usage, 50.0);
        assert_eq!(gpu.engines[..2], [("VideoDecode".to_owned(), 50.0), ("3D".to_owned(), 35.0)]);
        assert_eq!(gpu.vram_used, 2 << 30);
        assert_eq!(gpu.vram_percent(), Some(25.0));
    }

    #[test]
    fn ignores_counters_of_unknown_adapters() {
        // e.g. Microsoft Basic Render Driver, which DXGI reports as a software adapter.
        let other = ("pid_1_luid_0x00000000_0x00000042_phys_0_eng_0_engtype_3D".to_owned(), 90.0);
        let known = [KnownAdapter { luid: LUID, name: "Test GPU".into(), vram_total: 0 }];
        let adapters = aggregate(&[other], &[], &known);
        assert_eq!(adapters[0].usage, 0.0);
        assert_eq!(adapters[0].vram_percent(), None);
    }
}
