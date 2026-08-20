use gb_emulator::{ CPU, MemoryBus };
use gb_emulator::trace::{ self, Category as TraceCategory };
use gb_emulator::util::save_png;
use std::io::Write as IoWrite;

const W: usize = 160;
const H: usize = 144;

/// Exit code for a run that ended without reaching a pass/fail verdict.
/// Distinct from 1 (a real blargg failure) so a hang cannot be mistaken for
/// either a pass or a failure.
const EXIT_NO_VERDICT: i32 = 2;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum TimeoutMode {
    Strict,
    LcdAware,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum TimeoutReason {
    None,
    Budget,
    LcdOffWindow,
}

struct BlarggMonitor {
    signature_valid: bool,
    text_pos: u16,
}

impl BlarggMonitor {
    fn new() -> Self {
        Self { signature_valid: false, text_pos: 0 }
    }

    fn is_active(&self) -> bool { self.signature_valid }

    /// Drain new text chars from $A004+, then check for completion.
    /// Returns Some(status_code) when the test finishes, None while still running.
    fn poll(&mut self, bus: &mut MemoryBus) -> Option<u8> {
        if !self.signature_valid {
            if bus.read_byte(0xa001) == 0xde
                && bus.read_byte(0xa002) == 0xb0
                && bus.read_byte(0xa003) == 0x61
            {
                self.signature_valid = true;
            } else {
                return None;
            }
        }
        let status = bus.read_byte(0xa000);
        // 0x00 / 0xFF = RAM not yet initialized by the ROM; wait.
        // 0x80 = test running (properly initialized).
        // anything else = final result code.
        if status == 0xff {
            return None; // RAM not yet initialized by the ROM; wait
        }
        // Drain any newly-appended text. Stop on null terminator or 0xFF
        // (uninitialized RAM sentinel) to avoid printing garbage.
        loop {
            let b = bus.read_byte(0xa004 + self.text_pos);
            if b == 0 || b == 0xff { break; }
            print!("{}", b as char);
            self.text_pos += 1;
        }
        let _ = std::io::stdout().flush();
        if status != 0x80 { Some(status) } else { None }
    }
}

/// Blargg's multi-ROM suites (cpu_instrs, instr_timing, mem_timing) report
/// over the serial port instead of the $A000 protocol, ending with "Passed"
/// or "Failed". Watching for those lets a run stop at the verdict rather than
/// burning the whole frame budget.
struct SerialMonitor {
    scanned: usize,
}

impl SerialMonitor {
    fn new() -> Self {
        Self { scanned: 0 }
    }

    /// Some(true) = passed, Some(false) = failed, None = still running.
    /// Only rescans when new bytes have arrived.
    fn poll(&mut self, bus: &MemoryBus) -> Option<bool> {
        let out = &bus.mmu.serial_out;
        if out.len() == self.scanned {
            return None;
        }
        self.scanned = out.len();
        let text = String::from_utf8_lossy(out);
        if text.contains("Passed") {
            Some(true)
        } else if text.contains("Failed") {
            Some(false)
        } else {
            None
        }
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
    let blargg_max_frames = std::env
        ::var("GB_HEADLESS_BLARGG_MAX_FRAMES")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(5000);

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
    let mut blargg = BlarggMonitor::new();
    let mut serial = SerialMonitor::new();

    let mut f = 0u32;
    loop {
        let limit = if blargg.is_active() { blargg_max_frames } else { frames_to_dump };
        if f >= limit { break; }
        let mut safety_dots = 0u32;
        let mut lcd_on_seen = (bus.read_byte(0xff40) & 0x80) != 0;
        let mut timeout_reason = TimeoutReason::None;
        while !bus.gpu.frame_is_ready() {
            if safety_dots >= timeout_dots {
                timeout_reason = match timeout_mode {
                    TimeoutMode::Strict => TimeoutReason::Budget,
                    TimeoutMode::LcdAware if lcd_on_seen => TimeoutReason::Budget,
                    TimeoutMode::LcdAware => TimeoutReason::LcdOffWindow,
                };
                break;
            }

            // Blargg test output: print text from $A004+ and detect completion
            if let Some(status) = blargg.poll(&mut bus) {
                println!();
                if status == 0x00 {
                    eprintln!("blargg: PASSED");
                    eprintln!("completion=blargg_pass frames={} hard_timeouts={}", f, hard_timeouts);
                } else {
                    eprintln!("blargg: FAILED (code={:#04x})", status);
                    eprintln!(
                        "completion=blargg_fail code={:#04x} frames={} hard_timeouts={}",
                        status,
                        f,
                        hard_timeouts
                    );
                }
                if save_final {
                    bus.gpu.copy_frame(&mut fb);
                    save_png(&fb, &final_path)?;
                }
                std::process::exit(if status == 0x00 { 0 } else { 1 });
            }

            // Second verdict channel: serial. Kept distinct from the $A000
            // reasons above so which channel reported stays legible.
            if let Some(passed) = serial.poll(&bus) {
                println!();
                if passed {
                    eprintln!("serial: PASSED");
                    eprintln!("completion=serial_pass frames={} hard_timeouts={}", f, hard_timeouts);
                } else {
                    eprintln!("serial: FAILED");
                    eprintln!("completion=serial_fail frames={} hard_timeouts={}", f, hard_timeouts);
                }
                if save_final {
                    bus.gpu.copy_frame(&mut fb);
                    save_png(&fb, &final_path)?;
                }
                std::process::exit(if passed { 0 } else { 1 });
            }

            let cy = cpu.step(&mut bus);
            safety_dots = safety_dots.saturating_add(cy);
            if (bus.read_byte(0xff40) & 0x80) != 0 {
                lcd_on_seen = true;
            }
        }
        if bus.gpu.take_frame_ready() && timeout_reason == TimeoutReason::None {
            // Keep the latest rendered frame in fb; we only save at the end.
            bus.gpu.copy_frame(&mut fb);
            if print_hashes {
                let hash = frame_hash_hex(&fb);
                println!("FRAME {} HASH {}", f, hash);
            }
        } else {
            let in_warmup = f < warmup_frames;
            match timeout_reason {
                TimeoutReason::LcdOffWindow => {
                    lcd_off_skips = lcd_off_skips.saturating_add(1);
                }
                TimeoutReason::Budget if in_warmup => {
                    warmup_timeouts = warmup_timeouts.saturating_add(1);
                }
                TimeoutReason::Budget => {
                    hard_timeouts = hard_timeouts.saturating_add(1);
                    eprintln!("Frame {} timed out waiting for frame_ready", f);
                }
                TimeoutReason::None => {}
            }
            if trace::enabled(TraceCategory::Harness) {
                let timed_out = (timeout_reason == TimeoutReason::Budget) as u8;
                eprintln!(
                    "event=harness frame_req={} timeout={} reason={:?} lcd_on_seen={} dots={}",
                    f + 1,
                    timed_out,
                    timeout_reason,
                    lcd_on_seen as u8,
                    safety_dots
                );
            }
        }
        f += 1;
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

    // Every exit path reports why it ended. Without this a run that never
    // reached a verdict — a hang, or a ROM that does not use the $A000
    // protocol — was indistinguishable from a pass by exit code alone.
    let (reason, code) = if blargg.is_active() {
        // Signature validated, so the ROM speaks the protocol, but it never
        // reported a status within the frame budget: it is stuck.
        ("blargg_no_verdict", EXIT_NO_VERDICT)
    } else if hard_timeouts > 0 {
        ("frame_budget_exhausted", EXIT_NO_VERDICT)
    } else {
        // Ran to completion but reports results some other way (e.g. an
        // on-screen CRC), or is not a test ROM at all.
        ("no_blargg_signature", 0)
    };
    eprintln!("completion={} frames={} hard_timeouts={}", reason, f, hard_timeouts);
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}
