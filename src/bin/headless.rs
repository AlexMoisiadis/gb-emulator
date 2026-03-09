use gb_emulator::{ CPU, MemoryBus };
use gb_emulator::trace::{ self, Category as TraceCategory };
use std::{ fs::File, path::Path };
use png::{ Encoder, ColorType, BitDepth };

const W: usize = 160;
const H: usize = 144;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum TimeoutMode {
    Strict,
    LcdAware,
}

fn dmg_shade_to_u8(v: u8) -> u8 {
    match v & 0b11 {
        0 => 255,
        1 => 170,
        2 => 85,
        _ => 0,
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn frame_hash_hex(fb: &[[u8; W]; H]) -> String {
    let mut flat = vec![0u8; W * H];
    for y in 0..H {
        for x in 0..W {
            flat[y * W + x] = fb[y][x] & 0b11;
        }
    }
    format!("{:016x}", fnv1a64(&flat))
}

fn save_png<P: AsRef<Path>>(fb: &[[u8; W]; H], path: P) -> anyhow::Result<()> {
    let mut gray = vec![0u8; W * H];
    for y in 0..H {
        for x in 0..W {
            gray[y * W + x] = dmg_shade_to_u8(fb[y][x]);
        }
    }
    let file = File::create(path.as_ref())?;
    let mut enc = Encoder::new(file, W as u32, H as u32);
    enc.set_color(ColorType::Grayscale);
    enc.set_depth(BitDepth::Eight);
    let mut writer = enc.write_header()?;
    writer.write_image_data(&gray)?;
    Ok(())
}
fn main() -> anyhow::Result<()> {
    let rom_path = std::env::args().nth(1).expect("Usage: headless <path.gb>");
    let rom = std::fs::read(rom_path)?;
    let mut cpu = CPU::new();
    let mut bus = MemoryBus::new();
    let frames_to_dump = std::env
        ::var("GB_HEADLESS_FRAMES")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(3);
    let save_final = std::env::var("GB_HEADLESS_SAVE_FINAL").ok().as_deref() == Some("1");
    let final_path = std::env
        ::var("GB_HEADLESS_FINAL_PATH")
        .unwrap_or_else(|_| "headless-final.png".to_string());
    let print_hashes = std::env::var("GB_HEADLESS_HASH").ok().as_deref() == Some("1");
    let timeout_mode = match
        std::env
            ::var("GB_HEADLESS_TIMEOUT_MODE")
            .ok()
            .unwrap_or_else(|| "lcd_aware".to_string())
            .to_ascii_lowercase()
            .as_str()
    {
        "strict" => TimeoutMode::Strict,
        _ => TimeoutMode::LcdAware,
    };
    let timeout_dots = std::env
        ::var("GB_HEADLESS_TIMEOUT_DOTS")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(70_224 * 3);
    let warmup_frames = std::env
        ::var("GB_HEADLESS_WARMUP_FRAMES")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(2);

    // post_boot_init (same as viewer)
    cpu.regs.set_af(0x01b0);
    cpu.regs.set_bc(0x0013);
    cpu.regs.set_de(0x00d8);
    cpu.regs.set_hl(0x014d);
    cpu.sp = 0xfffe;
    cpu.pc = 0x0100;
    bus.write_byte(0xff40, 0x91);
    bus.write_byte(0xff42, 0x00);
    bus.write_byte(0xff43, 0x00);
    bus.write_byte(0xff47, 0xfc);
    bus.write_byte(0xff4a, 0x00);
    bus.write_byte(0xff4b, 0x00);
    bus.load_rom(&rom);

    let mut fb = [[0u8; W]; H];
    let mut hard_timeouts: u32 = 0;
    let mut warmup_timeouts: u32 = 0;
    let mut lcd_off_skips: u32 = 0;

    for f in 0..frames_to_dump {
        let mut safety_dots = 0u32;
        let mut lcd_on_seen = (bus.read_byte(0xff40) & 0x80) != 0;
        let mut timeout_reason = "none";
        while !bus.gpu.frame_is_ready() {
            if safety_dots >= timeout_dots {
                timeout_reason = match timeout_mode {
                    TimeoutMode::Strict => "budget",
                    TimeoutMode::LcdAware if lcd_on_seen => "budget",
                    TimeoutMode::LcdAware => "lcd_off_window",
                };
                break;
            }

            // Blargg status register: 0x80 = running, 0x01 = passed, other = failure code
            let blargg_status = bus.read_byte(0xa000);
            if blargg_status != 0x00 && blargg_status != 0x80 && blargg_status != 0xff {
                if blargg_status == 0x01 {
                    eprintln!("blargg: PASSED");
                } else {
                    eprintln!("blargg: FAILED (code={:#04x})", blargg_status);
                }
                if save_final {
                    bus.gpu.copy_frame(&mut fb);
                    save_png(&fb, &final_path)?;
                }
                std::process::exit(if blargg_status == 0x01 { 0 } else { 1 });
            }

            let cy = cpu.step(&mut bus);
            safety_dots = safety_dots.saturating_add(cy);
            if (bus.read_byte(0xff40) & 0x80) != 0 {
                lcd_on_seen = true;
            }
        }
        if bus.gpu.take_frame_ready() && timeout_reason == "none" {
            // Keep the latest rendered frame in fb; we only save at the end.
            bus.gpu.copy_frame(&mut fb);
            if print_hashes {
                let hash = frame_hash_hex(&fb);
                println!("FRAME {} HASH {}", f, hash);
            }
        } else {
            let in_warmup = f < warmup_frames;
            match timeout_reason {
                "lcd_off_window" => {
                    lcd_off_skips = lcd_off_skips.saturating_add(1);
                }
                "budget" if in_warmup => {
                    warmup_timeouts = warmup_timeouts.saturating_add(1);
                }
                "budget" => {
                    hard_timeouts = hard_timeouts.saturating_add(1);
                    eprintln!("Frame {} timed out waiting for frame_ready", f);
                }
                _ => {}
            }
            if trace::enabled(TraceCategory::Harness) {
                eprintln!(
                    "event=harness frame_req={} timeout={} reason={} lcd_on_seen={} dots={}",
                    f + 1,
                    (timeout_reason == "budget") as u8,
                    timeout_reason,
                    lcd_on_seen as u8,
                    safety_dots
                );
            }
        }
    }

    // Save the final frame if requested
    if save_final {
        save_png(&fb, &final_path)?;
        eprintln!("Saved final frame to {}", final_path);
    }

    eprintln!(
        "headless_summary requested={} hard_timeouts={} warmup_timeouts={} lcd_off_skips={} mode={:?} timeout_dots={} warmup_frames={}",
        frames_to_dump,
        hard_timeouts,
        warmup_timeouts,
        lcd_off_skips,
        timeout_mode,
        timeout_dots,
        warmup_frames
    );
    Ok(())
}
