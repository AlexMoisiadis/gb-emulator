use anyhow::{ anyhow, Context, Result };
use std::fs::File;
use std::path::Path;

/// Expand a decoded PNG buffer to exactly one byte per sample.
///
/// Sub-byte depths (1/2/4) are packed several samples to a byte, with each row
/// padded to a byte boundary, so they cannot be compared bytewise against an
/// 8-bit image of the same dimensions. Values are rescaled to the full 0..=255
/// range so a 2-bit level 1 equals an 8-bit 85 — otherwise two pixel-identical
/// frames stored at different depths compare as completely different.
fn expand_samples(
    src: &[u8],
    width: u32,
    height: u32,
    bit_depth: png::BitDepth,
    channels: usize
) -> Result<Vec<u8>> {
    let samples_per_row = (width as usize) * channels;
    let bits = match bit_depth {
        png::BitDepth::One => 1usize,
        png::BitDepth::Two => 2,
        png::BitDepth::Four => 4,
        png::BitDepth::Eight => 8,
        png::BitDepth::Sixteen => 16,
    };

    if bits == 8 {
        return Ok(src.to_vec());
    }
    if bits == 16 {
        // Big-endian samples; the high byte is enough for a visual comparison.
        return Ok(
            src
                .chunks_exact(2)
                .map(|c| c[0])
                .collect()
        );
    }

    // Sub-byte: unpack respecting per-row byte padding.
    let stride = (samples_per_row * bits + 7) / 8;
    let max = ((1u16 << bits) - 1) as u32;
    let mut out = Vec::with_capacity(samples_per_row * (height as usize));
    for y in 0..height as usize {
        let row = src
            .get(y * stride..(y + 1) * stride)
            .ok_or_else(|| anyhow!("truncated PNG: row {} beyond {} bytes", y, src.len()))?;
        for i in 0..samples_per_row {
            let bit = i * bits;
            let shift = 8 - bits - (bit % 8);
            let raw = ((row[bit / 8] >> shift) as u32) & max;
            out.push(((raw * 255) / max) as u8);
        }
    }
    Ok(out)
}

fn load_gray<P: AsRef<Path>>(path: P) -> Result<(u32, u32, Vec<u8>)> {
    let file = File::open(path.as_ref()).with_context(||
        format!("open {}", path.as_ref().display())
    )?;
    let decoder = png::Decoder::new(file);
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf)?;
    let raw = &buf[..info.buffer_size()];

    let channels = match info.color_type {
        png::ColorType::Grayscale => 1usize,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => {
            return Err(anyhow!("indexed PNG not supported"));
        }
    };
    let src = &expand_samples(raw, info.width, info.height, info.bit_depth, channels)?;

    let expected_samples = (info.width as usize) * (info.height as usize) * channels;
    if src.len() != expected_samples {
        return Err(
            anyhow!(
                "{}: decoded {} samples, expected {} for {}x{}",
                path.as_ref().display(),
                src.len(),
                expected_samples,
                info.width,
                info.height
            )
        );
    }

    let gray = match info.color_type {
        png::ColorType::Grayscale => src.to_vec(),
        png::ColorType::Rgb =>
            src
                .chunks_exact(3)
                .map(|c| (((c[0] as u16) + (c[1] as u16) + (c[2] as u16)) / 3) as u8)
                .collect(),
        png::ColorType::Rgba =>
            src
                .chunks_exact(4)
                .map(|c| (((c[0] as u16) + (c[1] as u16) + (c[2] as u16)) / 3) as u8)
                .collect(),
        png::ColorType::GrayscaleAlpha =>
            src
                .chunks_exact(2)
                .map(|c| c[0])
                .collect(),
        png::ColorType::Indexed => unreachable!("rejected above"),
    };

    Ok((info.width, info.height, gray))
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let expected = args
        .next()
        .ok_or_else(||
            anyhow!("usage: image_diff <expected.png> <actual.png> [threshold_percent]")
        )?;
    let actual = args
        .next()
        .ok_or_else(||
            anyhow!("usage: image_diff <expected.png> <actual.png> [threshold_percent]")
        )?;
    let threshold = args
        .next()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(1.5);

    let (ew, eh, epx) = load_gray(&expected)?;
    let (aw, ah, apx) = load_gray(&actual)?;
    if ew != aw || eh != ah {
        return Err(anyhow!("dimension mismatch: expected {}x{}, actual {}x{}", ew, eh, aw, ah));
    }
    // zip() would silently truncate to the shorter side and report a bogus
    // percentage against the wrong total; fail loudly instead.
    if epx.len() != apx.len() {
        return Err(anyhow!("pixel count mismatch: expected {}, actual {}", epx.len(), apx.len()));
    }

    let diffs = epx
        .iter()
        .zip(apx.iter())
        .filter(|(a, b)| a != b)
        .count();
    let total = epx.len().max(1);
    let pct = ((diffs as f64) * 100.0) / (total as f64);

    println!("DIFF mismatched={} total={} pct={:.4}", diffs, total, pct);
    if pct > threshold {
        return Err(anyhow!("diff {:.4}% exceeds threshold {:.4}%", pct, threshold));
    }

    Ok(())
}
