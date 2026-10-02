// src/util.rs — shared utilities used by viewer and headless binaries.

#[cfg(feature = "png")]
use png::{ BitDepth, ColorType, Encoder };
#[cfg(feature = "png")]
use std::fs::File;
#[cfg(feature = "png")]
use std::path::Path;

/// Convert a 2-bit DMG shade (0–3) to an 8-bit grayscale value.
/// 0 = white (255), 3 = black (0).
#[inline]
pub fn dmg_shade_to_u8(v: u8) -> u8 {
    match v & 0b11 {
        0 => 255,
        1 => 170,
        2 => 85,
        _ => 0,
    }
}

/// Write a 160×144 DMG framebuffer as an 8-bit grayscale PNG.
#[cfg(feature = "png")]
pub fn save_png<P: AsRef<Path>>(fb: &[[u8; 160]; 144], path: P) -> anyhow::Result<()> {
    let mut gray = vec![0u8; 160 * 144];
    for y in 0..144 {
        for x in 0..160 {
            gray[y * 160 + x] = dmg_shade_to_u8(fb[y][x]);
        }
    }
    let file = File::create(path.as_ref())?;
    let mut enc = Encoder::new(file, 160, 144);
    enc.set_color(ColorType::Grayscale);
    enc.set_depth(BitDepth::Eight);
    let mut writer = enc.write_header()?;
    writer.write_image_data(&gray)?;
    Ok(())
}
