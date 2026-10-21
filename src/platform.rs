//! Thin wrappers over the Win32 calls the strip needs beyond what egui/winit expose.

use std::{collections::HashSet, io};

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::{
    ApplicationModel::{StartupTask, StartupTaskState},
    Win32::{
        Foundation::{
            APPMODEL_ERROR_NO_PACKAGE, CloseHandle, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, GetLastError, HWND, LPARAM, LRESULT,
            WPARAM,
        },
        Graphics::Dwm::{
            DWMNCRP_DISABLED, DWMWA_CLOAKED, DWMWA_NCRENDERING_ENABLED, DWMWA_NCRENDERING_POLICY,
            DwmGetWindowAttribute, DwmSetWindowAttribute,
        },
        Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow},
        Storage::Packaging::Appx::GetCurrentPackageFullName,
        System::{
            Registry::{HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW},
            ProcessStatus::K32EmptyWorkingSet,
            SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
            Threading::{
                CreateMutexW, GetCurrentThreadId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA,
            },
        },
        UI::{
            Shell::{
                DefSubclassProc, SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI, SHERB_NOSOUND, SHEmptyRecycleBinW,
                SHQUERYRBINFO, SHQueryRecycleBinW, SetWindowSubclass, ShellExecuteW,
            },
            WindowsAndMessaging::{
                EnumThreadWindows, EnumWindows, GWL_EXSTYLE, GetForegroundWindow, GetWindowLongPtrW,
                GetWindowThreadProcessId, HWND_TOPMOST, IsIconic, IsWindowVisible, SW_HIDE, SW_SHOWNOACTIVATE,
                SW_SHOWNORMAL, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
                SetWindowLongPtrW, SetWindowPos, ShowWindow, WM_NCCALCSIZE, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
            },
        },
    },
    core::{BOOL, HSTRING, PCWSTR, w},
};

/// The strip's native window. Kept as an integer so event-handler closures (which must be `Send`)
/// can hold it; every call happens on the UI thread, which owns the window.
#[derive(Debug, Clone, Copy)]
pub struct Window(isize);

impl Window {
    pub fn from_handle(source: &impl HasWindowHandle) -> Option<Self> {
        match source.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(handle) => Some(Self(handle.hwnd.get())),
            _ => None,
        }
    }

    fn hwnd(self) -> HWND {
        HWND(self.0 as *mut _)
    }

    /// Makes it a tool window, which Alt-Tab leaves out. winit rewrites the extended style whenever it
    /// changes something (showing the window, click-through), so this is cheap to call every frame:
    /// it only writes when the style was reset.
    pub fn keep_out_of_alt_tab(self) {
        // SAFETY: plain style read/write on a window owned by this thread.
        unsafe {
            let style = GetWindowLongPtrW(self.hwnd(), GWL_EXSTYLE);
            let wanted = (style & !(WS_EX_APPWINDOW.0 as isize)) | WS_EX_TOOLWINDOW.0 as isize;
            if wanted != style {
                SetWindowLongPtrW(self.hwnd(), GWL_EXSTYLE, wanted);
            }
        }
    }

    pub fn is_visible(self) -> bool {
        // SAFETY: a read-only query on a valid window handle.
        unsafe { IsWindowVisible(self.hwnd()).as_bool() }
    }

    /// Shows without taking focus, or hides. Done natively because a hidden egui window gets no
    /// frames, so it could never process a "show" command itself.
    pub fn set_visible(self, visible: bool) {
        // SAFETY: valid window handle; the return value is the previous state, not an error.
        let _ = unsafe { ShowWindow(self.hwnd(), if visible { SW_SHOWNOACTIVATE } else { SW_HIDE }) };
    }

    /// The usable area (taskbar excluded) of the monitor the window is on, in physical pixels as
    /// `[left, top, right, bottom]`.
    pub fn work_area(self) -> Option<[i32; 4]> {
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        // SAFETY: `info` is a valid MONITORINFO with `cbSize` set, alive for the call.
        let found = unsafe { GetMonitorInfoW(MonitorFromWindow(self.hwnd(), MONITOR_DEFAULTTONEAREST), &mut info) };
        let work = info.rcWork;
        found.as_bool().then_some([work.left, work.top, work.right, work.bottom])
    }

    /// Moves the strip back to the top of the always-on-top band, where other topmost windows
    /// (the taskbar, overlays) can otherwise end up above it.
    pub fn raise_topmost(self) {
        // SAFETY: valid window handle; position and size are untouched.
        let _ = unsafe {
            SetWindowPos(self.hwnd(), Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE)
        };
    }
}

/// Removes the Windows 11 drop shadow and the 1 px top border from this thread's windows (the strip
/// and its popups).
///
/// egui-winit always requests winit's "undecorated shadow" for borderless windows, which keeps a
/// hidden caption so DWM draws its large default shadow. DWM offers no smaller variant, so this
/// turns DWM's non-client rendering (frame and shadow) off instead. Without the shadow, winit's
/// matching hack (a 1 px non-client strip along the top) only shows as a thin line, so a subclass
/// makes the whole window client area. Cheap enough to call every frame: it only acts on windows
/// it hasn't handled yet, such as a popup that just opened.
pub fn remove_window_frames() {
    unsafe extern "system" fn frameless(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        if msg == WM_NCCALCSIZE && wparam.0 != 0 {
            return LRESULT(0); // client area = the whole window
        }
        // SAFETY: forwards the message unchanged to the next handler (winit's window procedure).
        unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
    }

    unsafe extern "system" fn visit(hwnd: HWND, _: LPARAM) -> BOOL {
        let mut enabled = BOOL(0);
        // SAFETY: `enabled` is a valid BOOL-sized out-buffer for this attribute.
        let queried = unsafe {
            DwmGetWindowAttribute(hwnd, DWMWA_NCRENDERING_ENABLED, (&raw mut enabled).cast(), size_of::<BOOL>() as u32)
        };
        if queried.is_ok() && enabled.as_bool() {
            let policy = DWMNCRP_DISABLED;
            // SAFETY: `policy` is a valid DWMNCRENDERINGPOLICY-sized in-buffer for this attribute.
            let _ = unsafe {
                DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_NCRENDERING_POLICY,
                    (&raw const policy).cast(),
                    size_of_val(&policy) as u32,
                )
            };
            // SAFETY: subclasses a window owned by this thread with a stateless procedure; the
            // frame change makes Windows re-ask WM_NCCALCSIZE, now answered by `frameless`.
            unsafe {
                if SetWindowSubclass(hwnd, Some(frameless), FRAMELESS_SUBCLASS_ID, 0).as_bool() {
                    let flags = SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE;
                    let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, flags);
                }
            }
        }
        BOOL::from(true) // keep enumerating
    }
    // SAFETY: enumerates windows owned by the calling thread with a callback that holds no state.
    let _ = unsafe { EnumThreadWindows(GetCurrentThreadId(), Some(visit), LPARAM(0)) };
}

const FRAMELESS_SUBCLASS_ID: usize = 1;

/// Returns false if another copy of the app is already running.
/// The mutex handle is deliberately never closed: Windows releases it when this process exits.
pub fn claim_single_instance() -> bool {
    // SAFETY: creates or opens a named mutex; no pointers are retained.
    match unsafe { CreateMutexW(None, false, w!("Local\\compteur-single-instance")) } {
        Ok(_) => {
            // SAFETY: GetLastError is read right after the call that set it.
            let error = unsafe { GetLastError() };
            error != ERROR_ALREADY_EXISTS
        }
        Err(_) => true, // can't tell; running twice beats not running at all
    }
}

const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const RUN_VALUE: PCWSTR = w!("Compteur");
/// The startup task declared in `packaging/AppxManifest.xml`.
const STARTUP_TASK_ID: &str = "CompteurStartup";

/// True when running from the Store (MSIX) package. There, writes to the Run key go to the
/// package's private registry and are ignored at sign-in, so the manifest's startup task is used.
fn is_packaged() -> bool {
    let mut length = 0;
    // SAFETY: a length query with no buffer.
    let status = unsafe { GetCurrentPackageFullName(&mut length, None) };
    status != APPMODEL_ERROR_NO_PACKAGE
}

fn startup_task() -> windows::core::Result<StartupTask> {
    StartupTask::GetAsync(&HSTRING::from(STARTUP_TASK_ID))?.join()
}

/// Whether the app is registered to start at sign-in (per user, so no admin rights needed).
pub fn autostart_enabled() -> bool {
    if is_packaged() {
        let state = startup_task().and_then(|task| task.State());
        return state.is_ok_and(|s| s == StartupTaskState::Enabled || s == StartupTaskState::EnabledByPolicy);
    }
    // SAFETY: an existence check; no output buffers are passed.
    let status = unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE, RRF_RT_REG_SZ, None, None, None) };
    status == ERROR_SUCCESS
}

pub fn set_autostart(enabled: bool) -> io::Result<()> {
    if is_packaged() {
        let task = startup_task().map_err(io::Error::other)?;
        // Once the user turns the task off in Task Manager, only they can turn it back on; the
        // request then just reports that state, which `autostart_enabled` shows.
        let result = if enabled { task.RequestEnableAsync().and_then(|op| op.join()).map(drop) } else { task.Disable() };
        return result.map_err(io::Error::other);
    }
    let status = if enabled {
        let exe = std::env::current_exe()?;
        let command = HSTRING::from(format!("\"{}\"", exe.display()));
        let bytes = (command.len() + 1) * size_of::<u16>(); // including the terminating NUL
        // SAFETY: `command` is a NUL-terminated UTF-16 string of `bytes` bytes, alive for the call.
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                RUN_VALUE,
                REG_SZ.0,
                Some(command.as_ptr().cast()),
                bytes as u32,
            )
        }
    } else {
        // SAFETY: deletes one named value; no buffers involved.
        match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE) } {
            ERROR_FILE_NOT_FOUND => ERROR_SUCCESS, // already off
            status => status,
        }
    };
    if status == ERROR_SUCCESS { Ok(()) } else { Err(io::Error::from_raw_os_error(status.0 as i32)) }
}

/// Processes the user is looking at: the foreground app and every app with a visible window that
/// isn't minimized or cloaked (on another virtual desktop). The cleanup leaves these alone.
pub fn on_screen_process_ids() -> HashSet<u32> {
    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: `lparam` is the `HashSet` passed below, alive for the whole enumeration.
        let ids = unsafe { &mut *(lparam.0 as *mut HashSet<u32>) };
        let mut cloaked = 0u32;
        // SAFETY: read-only queries on a window handle supplied by EnumWindows; `cloaked` is a
        // valid u32 out-buffer for DWMWA_CLOAKED.
        unsafe {
            let _ = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, (&raw mut cloaked).cast(), size_of::<u32>() as u32);
            if IsWindowVisible(hwnd).as_bool() && !IsIconic(hwnd).as_bool() && cloaked == 0 {
                let mut pid = 0;
                GetWindowThreadProcessId(hwnd, Some(&raw mut pid));
                ids.insert(pid);
            }
        }
        BOOL::from(true) // keep enumerating
    }

    let mut ids = HashSet::new();
    // SAFETY: the callback only touches `ids` through `lparam` during this call.
    unsafe {
        let _ = EnumWindows(Some(visit), LPARAM(&raw mut ids as isize));
        let mut pid = 0;
        GetWindowThreadProcessId(GetForegroundWindow(), Some(&raw mut pid));
        ids.insert(pid);
    }
    ids.remove(&0);
    ids
}

/// Asks Windows to move a process's memory out of RAM (it comes back on demand). False when the
/// process can't be opened, such as system or elevated processes.
pub fn trim_working_set(pid: u32) -> bool {
    // SAFETY: the handle is checked, used for one call and closed.
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_SET_QUOTA | PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let trimmed = K32EmptyWorkingSet(process).as_bool();
        let _ = CloseHandle(process);
        trimmed
    }
}

/// Physical memory in use (total minus available), in bytes.
pub fn memory_in_use() -> Option<u64> {
    let mut status = MEMORYSTATUSEX { dwLength: size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    // SAFETY: `status` is a valid MEMORYSTATUSEX with `dwLength` set.
    unsafe { GlobalMemoryStatusEx(&mut status) }.ok()?;
    Some(status.ullTotalPhys.saturating_sub(status.ullAvailPhys))
}

/// Size in bytes and item count of the Recycle Bins on all drives.
pub fn recycle_bin_size() -> Option<(u64, u64)> {
    let mut info = SHQUERYRBINFO { cbSize: size_of::<SHQUERYRBINFO>() as u32, ..Default::default() };
    // SAFETY: a null root path means all drives; `info` is valid with `cbSize` set.
    unsafe { SHQueryRecycleBinW(PCWSTR::null(), &mut info) }.ok()?;
    Some((info.i64Size.max(0) as u64, info.i64NumItems.max(0) as u64))
}

/// Empties the Recycle Bins on all drives, without a confirmation, progress window or sound.
pub fn empty_recycle_bin() -> bool {
    // SAFETY: a null root path means all drives; no buffers are passed.
    unsafe {
        SHEmptyRecycleBinW(None, PCWSTR::null(), SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND)
    }
    .is_ok()
}

/// Opens Windows' own Disk Cleanup, which can elevate itself to clean system files.
pub fn open_disk_cleanup() {
    // SAFETY: constant, NUL-terminated strings; the result only reports whether it launched.
    let _ = unsafe { ShellExecuteW(None, w!("open"), w!("cleanmgr.exe"), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
}
