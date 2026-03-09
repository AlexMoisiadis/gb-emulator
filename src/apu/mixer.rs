// src/apu/mixer.rs

/// Mix four channel samples (each 0.0–15.0) with a master volume (0–7).
/// Returns a normalised float in –1.0 … +1.0.
///
/// The DMG master volume adds 1 to the raw 0–7 value (so range is 1–8),
/// matching hardware behaviour documented in the Pan Docs.
#[inline]
pub fn mix(s1: f32, s2: f32, s3: f32, s4: f32, master_vol: u8) -> f32 {
    let sum = s1 + s2 + s3 + s4;
    // 4 channels × max 15 = 60; master volume scales 1–8 on top.
    // Normalise to –1 … +1 by dividing by the theoretical maximum.
    let vol = ((master_vol as f32) + 1.0) / 8.0; // 0.125 … 1.0
    let normalised = sum / (4.0 * 15.0); // 0.0 … 1.0
    // Centre around 0 and apply volume
    (normalised * 2.0 - 1.0) * vol
}
