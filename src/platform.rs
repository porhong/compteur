//! Thin wrappers over the Win32 calls the strip needs beyond what egui/winit expose.

use std::io;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::{
    Win32::{
        Foundation::{
            ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, GetLastError, HWND, LPARAM, LRESULT, WPARAM,
        },
        Graphics::Dwm::{
            DWMNCRP_DISABLED, DWMWA_NCRENDERING_ENABLED, DWMWA_NCRENDERING_POLICY, DwmGetWindowAttribute,
            DwmSetWindowAttribute,
        },
        Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow},
        System::{
            Registry::{HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW},
            Threading::{CreateMutexW, GetCurrentThreadId},
        },
        UI::{
            Shell::{DefSubclassProc, SetWindowSubclass},
            WindowsAndMessaging::{
                EnumThreadWindows, GWL_EXSTYLE, GetWindowLongPtrW, HWND_TOPMOST, IsWindowVisible, SW_HIDE,
                SW_SHOWNOACTIVATE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
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

/// Whether the app is registered to start at sign-in (per user, so no admin rights needed).
pub fn autostart_enabled() -> bool {
    // SAFETY: an existence check; no output buffers are passed.
    let status = unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE, RRF_RT_REG_SZ, None, None, None) };
    status == ERROR_SUCCESS
}

pub fn set_autostart(enabled: bool) -> io::Result<()> {
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
