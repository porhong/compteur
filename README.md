# Compteur

A small always-on-top strip that shows how your Windows PC is doing:

```
CPU 23% │ RAM 61% │ GPU 10% 50° │ VRAM 13% │ NET ↓8.3K ↑6.1K │ DSK R 0B W4.0K │ TOP chrome 12%
```

Values turn amber at 70 % (75 °C) and red at 90 % (90 °C). Hover a segment for details and a 60-second graph.

## Using it

| Action | How |
|---|---|
| Move the strip | Drag it with the left mouse button |
| Settings (metrics, refresh rate, opacity, lock, click-through, start with Windows) | Right-click the strip, or tray icon → Settings… |
| Show / hide the strip | `Ctrl+Alt+H`, or tray icon |
| Click-through (clicks go to the window below) | `Ctrl+Alt+M`, or tray icon, or settings |
| Exit | Tray icon → Exit, or settings → Exit Compteur |

Settings are saved to `%APPDATA%\compteur\config\config.toml`.

## What each metric means

- **CPU**: total load across all cores, as in Task Manager; hover for per-core load and clock speed.
- **RAM**: physical memory in use; hover for available memory and swap.
- **GPU**: load of the busiest engine on the busiest GPU (Task Manager's method), with temperature when the driver reports it. Works with NVIDIA, AMD and Intel.
- **VRAM**: dedicated memory used on the GPU with the most video memory.
- **NET**: download/upload speed over physical adapters (virtual switches are skipped so traffic isn't counted twice).
- **DSK**: read/write speed of all physical disks; colored by how busy the disks are. Hover for free space per drive.
- **TOP**: the app using the most CPU, grouped by name (all `chrome.exe` processes count as one). Refreshes every 3 s.

## Known limits

- **CPU temperature** only appears on PCs whose firmware exposes an ACPI thermal zone (many laptops do, many desktops don't). Reading the CPU die directly needs a kernel driver, which this app avoids.
- **Exclusive-fullscreen games** draw over every window, including this one. Borderless/windowed games are fine.
- Hidden metrics are not sampled at all, so turning off GPU (the most expensive counters) lowers CPU use further.

## Building

No Visual Studio or admin rights are needed; the project uses the `x86_64-pc-windows-gnullvm` toolchain (pinned in `rust-toolchain.toml`) with llvm-mingw.

1. Install Rust: `winget install Rustlang.Rustup`
2. Install llvm-mingw (per user): `winget install MartinStorsjo.LLVM-MinGW.UCRT --scope user`
3. Open a new terminal, then:

```
cargo build --release      # target\release\compteur.exe — a single file, no DLLs to ship
cargo test
cargo clippy --all-targets -- -D warnings
```

`.cargo/config.toml` links the C runtime statically, so the exe runs on PCs without llvm-mingw.

## Code map

| File | Role |
|---|---|
| `src/main.rs` | Single-instance check, window setup |
| `src/app.rs` | The strip, hover details, settings menu |
| `src/sampler.rs` | Background thread that samples enabled metrics and wakes the UI |
| `src/metrics/` | One collector per area: CPU/RAM, disk/network, GPU, temperature, processes |
| `src/pdh.rs` | Safe wrapper over Windows performance counters |
| `src/platform.rs` | Win32 bits egui doesn't expose: Alt-Tab hiding, topmost, shadows, autostart |
| `src/controls.rs` | Tray icon and global hotkeys |
| `src/icon.rs`, `build.rs` | App icon drawn in code and embedded in the exe |
