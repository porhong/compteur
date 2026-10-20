//! Minimal safe wrapper over Windows Performance Data Helper (PDH) counters.
//!
//! Counters are added by English name, so they work on any Windows display language.

use windows::{
    Win32::System::Performance::{
        PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY,
        PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterValue, PdhOpenQueryW,
    },
    core::{HSTRING, PCWSTR},
};

const ERROR_SUCCESS: u32 = 0;

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
}

impl Drop for Query {
    fn drop(&mut self) {
        // SAFETY: the handle is open and closed exactly once; this also frees its counters.
        unsafe { PdhCloseQuery(self.handle) };
    }
}
