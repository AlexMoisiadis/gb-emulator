// src/bin/util/image.rs
//! Minimal image writers for the emulator frontend.
//!
//! - `write_pgm`: ASCII PGM (portable graymap), maps DMG shades 0..3 to 0..3.
//! - `write_ppm`: ASCII PPM (portable pixmap), maps DMG shades 0..3 to a 4-color RGB palette.
//!
//! These writers are intentionally simple, std-only, and suitable for quick
//! diagnostic dumps from the CLI frontend. Keep all file I/O here (frontend),
//! not in the emulation core.

use std::fs::File;
use std::io::{ Result as IoResult, Write };

/// DMG framebuffer dimensions (matches your GPU constants).
pub const WIDTH: usize = 160;
pub const HEIGHT: usize = 144;

/// Convenience alias matching your current framebuffer type.
pub type Framebuffer = [[u8; WIDTH]; HEIGHT];

/// Write the framebuffer as an ASCII **PGM** (portable graymap).
///
/// - Header: `P2`, then `WIDTH HEIGHT`, then `maxval = 3`.
/// - Each pixel is a number `0..3` (your DMG shade index).
/// - Most viewers/editors can open PGM directly or convert it to PNG.
///
/// # Errors
/// Returns any underlying I/O error from file creation or writes.
pub fn write_pgm(path: &str, fb: &Framebuffer) -> IoResult<()> {
    let mut file = File::create(path)?;

    // P2 header
    // Note: We keep maxval=3 since your buffer already uses 0..3.
    writeln!(file, "P2")?;
    writeln!(file, "{} {}", WIDTH, HEIGHT)?;
    writeln!(file, "3")?;

    for y in 0..HEIGHT {
        // Write a row of grayscale indices (0..3)
        for x in 0..WIDTH {
            // Clamp defensively in case any upstream code writes out of range
            let v = (fb[y][x] & 0b11) as u8;
            write!(file, "{v} ")?;
        }
        writeln!(file)?;
    }

    Ok(())
}

/// Write the framebuffer as an ASCII **PPM** (portable pixmap).
///
/// - Header: `P3`, then `WIDTH HEIGHT`, then `maxval = 255`.
/// - Each pixel is three numbers `R G B`.
/// - Uses a 4-entry palette (customizable) that maps DMG shade indices 0..3
///   to RGB888. By default we use a classic DMG-like grayscale.
///
/// # Parameters
/// * `palette` — If `None`, defaults to a grayscale palette:
///     - 0 → (255,255,255) white
///     - 1 → (170,170,170) light gray
///     - 2 → (85,85,85)    dark gray
///     - 3 → (0,0,0)       black
///
/// # Errors
/// Returns any underlying I/O error from file creation or writes.
pub fn write_ppm(path: &str, fb: &Framebuffer, palette: Option<[(u8, u8, u8); 4]>) -> IoResult<()> {
    let pal = palette.unwrap_or(DEFAULT_DMG_PAL);

    let mut file = File::create(path)?;

    // P3 header
    writeln!(file, "P3")?;
    writeln!(file, "{} {}", WIDTH, HEIGHT)?;
    writeln!(file, "255")?;

    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            // Clamp defensively to index 0..3
            let idx = (fb[y][x] & 0b11) as usize;
            let (r, g, b) = pal[idx];
            write!(file, "{r} {g} {b} ")?;
        }
        writeln!(file)?;
    }

    Ok(())
}

/// A simple DMG-like grayscale palette for PPM dumps.
///
/// Index meaning in your buffer is:
///     0 = white (typically background)
///     1 = light gray
///     2 = dark gray
///     3 = black
pub const DEFAULT_DMG_PAL: [(u8, u8, u8); 4] = [
    (255, 255, 255), // 0
    (170, 170, 170), // 1
    (85, 85, 85), // 2
    (0, 0, 0), // 3
];

/// An alternate high-contrast palette (optional).
/// Useful if you want overlays to “pop” more in PPM dumps.
pub const HIGH_CONTRAST_PAL: [(u8, u8, u8); 4] = [
    (250, 250, 250), // 0 - nearly white
    (120, 200, 255), // 1 - light blue
    (255, 180, 0), // 2 - orange
    (20, 20, 20), // 3 - near black
];
