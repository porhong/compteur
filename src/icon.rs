//! The app icon. `assets/` is generated from `logo.png` by `tools/make_icon.py`.

/// Resource ordinal of the icon `build.rs` embeds in the exe.
pub const RESOURCE_ID: u16 = 1;

pub const WINDOW_ICON_SIZE: u32 = 64;

/// The window icon as straight RGBA, `WINDOW_ICON_SIZE` square.
pub fn window_icon_rgba() -> Vec<u8> {
    include_bytes!("../assets/icon-64.rgba").to_vec()
}
