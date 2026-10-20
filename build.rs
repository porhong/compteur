//! Embeds the app icon in the `.exe` so Explorer, the taskbar and shortcuts show it.
//! Compiling the resource needs `windres` on PATH (llvm-mingw provides it).

use std::{env, fs, path::PathBuf};

include!("src/icon.rs");

/// Sizes Windows picks from for Explorer views, Alt-Tab and high-DPI displays.
const ICON_SIZES: [u32; 5] = [16, 24, 32, 48, 256];

fn main() {
    println!("cargo:rerun-if-changed=src/icon.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let ico = out_dir.join("compteur.ico");
    fs::write(&ico, ico_file(&ICON_SIZES)).expect("write icon");
    let rc = out_dir.join("compteur.rc");
    // .rc strings use C escapes, so backslashes in the path must be doubled.
    let ico_path = ico.display().to_string().replace('\\', "\\\\");
    fs::write(&rc, format!("1 ICON \"{ico_path}\"\n")).expect("write resource script");
    embed_resource::compile(&rc, embed_resource::NONE).manifest_required().expect("compile icon resource");
}

/// An `.ico` file with one 32-bit BMP image per size.
fn ico_file(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes.iter().map(|&size| bmp_image(size)).collect();
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]); // reserved, type = icon
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (&size, image) in sizes.iter().zip(&images) {
        let dimension = if size >= 256 { 0 } else { size as u8 }; // 0 means 256
        out.extend_from_slice(&[dimension, dimension, 0, 0]); // width, height, palette, reserved
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(image.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += image.len() as u32;
    }
    for image in images {
        out.extend_from_slice(&image);
    }
    out
}

/// BITMAPINFOHEADER + bottom-up BGRA pixels + an empty AND mask (alpha already carries transparency).
fn bmp_image(size: u32) -> Vec<u8> {
    let rgba = icon_rgba(size);
    let mask_row = size.div_ceil(32) * 4;
    let mut out = Vec::new();
    out.extend_from_slice(&40u32.to_le_bytes()); // header size
    out.extend_from_slice(&(size as i32).to_le_bytes());
    out.extend_from_slice(&(2 * size as i32).to_le_bytes()); // height covers pixels + mask
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    out.extend_from_slice(&[0; 4]); // BI_RGB
    out.extend_from_slice(&(size * size * 4 + mask_row * size).to_le_bytes());
    out.extend_from_slice(&[0; 16]); // resolution and palette counts
    for y in (0..size).rev() {
        for x in 0..size {
            let at = ((y * size + x) * 4) as usize;
            out.extend_from_slice(&[rgba[at + 2], rgba[at + 1], rgba[at], rgba[at + 3]]);
        }
    }
    out.resize(out.len() + (mask_row * size) as usize, 0);
    out
}
