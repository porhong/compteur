//! Embeds the app icon in the `.exe` so Explorer, the taskbar, shortcuts and the tray show it.
//! Compiling the resource needs `windres` on PATH (llvm-mingw provides it).

use std::{env, fs, path::PathBuf};

const ICON: &str = "assets/compteur.ico";

fn main() {
    println!("cargo:rerun-if-changed={ICON}");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let rc = out_dir.join("compteur.rc");
    // .rc strings use C escapes, so backslashes in the path must be doubled. Ordinal 1 is `icon::RESOURCE_ID`.
    let ico_path = manifest_dir.join(ICON).display().to_string().replace('\\', "\\\\");
    fs::write(&rc, format!("1 ICON \"{ico_path}\"\n")).expect("write resource script");
    embed_resource::compile(&rc, embed_resource::NONE).manifest_required().expect("compile icon resource");
}
