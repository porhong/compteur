//! Minimal safe wrapper over Windows Performance Data Helper (PDH) counters.
//!
//! Counters are added by English name, so they work on any Windows display language.

use windows::{
    Win32::System::Performance::{
        PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_VALID_DATA, PDH_FMT, PDH_FMT_COUNTERVALUE, PDH_FMT_COUNTERVALUE_ITEM_W,
        PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA, PdhAddEnglishCounterW, PdhCloseQuery,
        PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhGetFormattedCounterValue, PdhOpenQueryW,
    },
    core::{HSTRING, PCWSTR},
};

const ERROR_SUCCESS: u32 = 0;
/// `PDH_FMT_NOCAP100`, missing from the `windows` crate: don't clamp per-instance values to 100.
const FMT_DOUBLE_NOCAP: PDH_FMT = PDH_FMT(PDH_FMT_DOUBLE.0 | 0x8000);

/// A set of counters sampled together. PDH handles are thread-bound, so this is deliberately `!Send`.
pub struct Query {
    handle: PDH_HQUERY,
}

/// Handle to one counter; only valid while its `Query` is alive.
#[derive(Clone, Copy)]
pub struct Counter(PDH_HCOUNTER);

impl Query {
    pub fn new() -> Option<Self> {
        let mut handle = PDH_HQUERY::default();
        // SAFETY: `handle` is a valid out-pointer; a null data source means live data.
        let status = unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut handle) };
        (status == ERROR_SUCCESS).then_some(Self { handle })
    }

    /// Adds a counter such as `\PhysicalDisk(_Total)\Disk Read Bytes/sec`. `None` if it doesn't exist here.
    pub fn add(&self, path: &str) -> Option<Counter> {
        let mut counter = PDH_HCOUNTER::default();
        // SAFETY: the query handle is open and `counter` is a valid out-pointer.
        let status = unsafe { PdhAddEnglishCounterW(self.handle, &HSTRING::from(path), 0, &mut counter) };
        (status == ERROR_SUCCESS).then_some(Counter(counter))
    }

    /// Takes a sample of every counter. Rate counters need two collections before they have a value.
    pub fn collect(&self) -> bool {
        // SAFETY: the query handle is open.
        unsafe { PdhCollectQueryData(self.handle) == ERROR_SUCCESS }
    }

    /// The counter's value at the last `collect`, or `None` if it has no valid data yet.
    pub fn value(&self, counter: Counter) -> Option<f64> {
        let mut value = PDH_FMT_COUNTERVALUE::default();
        // SAFETY: `counter` belongs to this open query and `value` is a valid out-pointer.
        let status = unsafe { PdhGetFormattedCounterValue(counter.0, PDH_FMT_DOUBLE, None, &mut value) };
        let valid = status == ERROR_SUCCESS && matches!(value.CStatus, PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA);
        // SAFETY: PDH_FMT_DOUBLE was requested, so the union holds `doubleValue`.
        valid.then_some(unsafe { value.Anonymous.doubleValue })
    }

    /// Every instance of a wildcard counter (e.g. `\GPU Engine(*)\...`) as `(instance name, value)`.
    /// Wildcards are re-expanded on each `collect`, so instances that come and go are picked up.
    pub fn values(&self, counter: Counter) -> Vec<(String, f64)> {
        // The buffer holds the item array followed by the name strings it points into.
        let item_size = size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>();
        let mut buffer = Vec::new();
        let (mut size, mut count) = (0u32, 0u32);
        // Instances can appear between the size query and the read, so retry a few times.
        for _ in 0..3 {
            // SAFETY: `buffer` holds at least `size` bytes, aligned for the items; an empty buffer is a size query.
            let status = unsafe {
                let items = (!buffer.is_empty()).then_some(buffer.as_mut_ptr());
                PdhGetFormattedCounterArrayW(counter.0, FMT_DOUBLE_NOCAP, &mut size, &mut count, items)
            };
            match status {
                ERROR_SUCCESS if !buffer.is_empty() => break,
                PDH_MORE_DATA => {
                    buffer = vec![PDH_FMT_COUNTERVALUE_ITEM_W::default(); (size as usize).div_ceil(item_size)];
                    count = 0;
                }
                _ => return Vec::new(),
            }
        }

        buffer[..(count as usize).min(buffer.len())]
            .iter()
            .filter(|item| matches!(item.FmtValue.CStatus, PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA))
            // SAFETY: `szName` points into `buffer`, which is still alive; the union holds a double.
            .filter_map(|item| unsafe {
                Some((item.szName.to_string().ok()?, item.FmtValue.Anonymous.doubleValue))
            })
            .collect()
    }
}

impl Drop for Query {
    fn drop(&mut self) {
        // SAFETY: the handle is open and closed exactly once; this also frees its counters.
        unsafe { PdhCloseQuery(self.handle) };
    }
}
