use anyhow::{ anyhow, Context, Result };
use std::fs::File;
use std::path::Path;

fn load_gray<P: AsRef<Path>>(path: P) -> Result<(u32, u32, Vec<u8>)> {
    let file = File::open(path.as_ref()).with_context(||
        format!("open {}", path.as_ref().display())
    )?;
    let decoder = png::Decoder::new(file);
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf)?;
    let src = &buf[..info.buffer_size()];

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
        png::ColorType::Indexed => {
            return Err(anyhow!("indexed PNG not supported"));
        }
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
