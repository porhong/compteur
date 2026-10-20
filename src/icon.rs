// The app icon, drawn in code so there is no binary asset to keep in sync.
// Self-contained (no crate imports, no inner doc comments): `build.rs` includes this file
// to make the `.exe` icon.

/// Three bars of different heights on a dark rounded square, as straight (non-premultiplied) RGBA.
pub fn icon_rgba(size: u32) -> Vec<u8> {
    const BACKGROUND: [u8; 4] = [30, 30, 30, 255];
    const BARS: [(f32, [u8; 4]); 3] = [
        (0.45, [90, 200, 160, 255]), // teal
        (0.75, [255, 190, 70, 255]), // amber
        (0.60, [230, 230, 230, 255]), // light gray
    ];
    let s = size as f32;
    let radius = s * 0.18;
    let margin = s * 0.18;
    let bar_width = (s - 2.0 * margin) / 5.0; // three bars with a bar-wide gap between each
    let bottom = s - margin;

    let mut pixels = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            // Distance outside the rounded square's corner circles; > 0 means transparent.
            let dx = (radius - fx).max(fx - (s - radius)).max(0.0);
            let dy = (radius - fy).max(fy - (s - radius)).max(0.0);
            if (dx * dx + dy * dy).sqrt() > radius {
                continue;
            }
            let mut color = BACKGROUND;
            for (i, (height, bar_color)) in BARS.iter().enumerate() {
                let left = margin + i as f32 * 2.0 * bar_width;
                let top = bottom - height * (s - 2.0 * margin);
                if fx >= left && fx < left + bar_width && fy >= top && fy < bottom {
                    color = *bar_color;
                }
            }
            let at = ((y * size + x) * 4) as usize;
            pixels[at..at + 4].copy_from_slice(&color);
        }
    }
    pixels
}
