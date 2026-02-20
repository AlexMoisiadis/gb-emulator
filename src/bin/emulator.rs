// /bin/emulator.rs

#[derive(Default, Clone, Copy)]
struct OverlayArgs {
    grid: bool,
    window: bool,
    axes: bool,
    sprites: bool,
}

use gb_emulator::{ CPU, MemoryBus };
use gb_emulator::gpu::DebugOverlayConfig;
use std::{ env, fs, process };

mod util {
    pub mod image;
}
use util::image::{ write_pgm, write_ppm, Framebuffer };

fn main() {
    // ---- Parse CLI ----
    // Examples:
    //   emulator --boot dmg_boot.bin --rom dmg-acid2.gb
    //   emulator --rom tetris.gb
    let mut args = env::args().skip(1);
    let mut boot_path: Option<String> = None;
    let mut rom_path: Option<String> = None;
    let mut overlay: OverlayArgs = OverlayArgs::default();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--boot" => {
                boot_path = args.next();
            }
            "--rom" => {
                rom_path = args.next();
            }
            "--overlay" => {
                if let Some(list) = args.next() {
                    overlay = parse_overlay_list(&list)
                        .map_err(|e| {
                            eprintln!("Invalid --overlay: {e}");
                            e
                        })
                        .unwrap_or_else(|_| {
                            // fall back to no overlays on error
                            OverlayArgs::default()
                        });
                } else {
                    eprintln!("--overlay requires a value, e.g. --overlay grid,window,sprites");
                }
            }
            _ => eprintln!("Unknown arg: {arg}"),
        }
    }

    let rom_path = rom_path.unwrap_or_else(|| {
        eprintln!(
            "Usage: emulator --rom <path.gb> [--boot <dmg_boot.bin>] [--overlay grid,window,axes,sprites]"
        );
        process::exit(1);
    });

    let rom = fs::read(&rom_path).unwrap_or_else(|e| {
        eprintln!("Failed to read ROM '{}': {}", rom_path, e);
        process::exit(1);
    });

    // ---- Wire up CPU / Bus ----
    let mut cpu = CPU::new();
    let mut bus = MemoryBus::new();

    // NEW: enable trace via environment variable
    let trace_enabled = std::env::var("GB_TRACE").ok().as_deref() == Some("1");
    cpu.set_trace(trace_enabled);

    // Optional: user-supplied DMG boot ROM (256 bytes)
    if let Some(bp) = boot_path {
        match fs::read(&bp) {
            Ok(bytes) if bytes.len() == 0x100 => {
                bus.load_boot_rom(bytes);
                println!("Loaded DMG boot ROM from '{}'", bp);
                cpu.pc = 0x0000; // start at boot ROM
            }
            Ok(bytes) => {
                eprintln!(
                    "Boot ROM '{}' is {} bytes; expected 256 (DMG). Skipping boot ROM.",
                    bp,
                    bytes.len()
                );
                post_boot_init(&mut cpu, &mut bus);
            }
            Err(e) => {
                eprintln!("Failed to read boot ROM '{}': {}. Skipping boot ROM.", bp, e);
                post_boot_init(&mut cpu, &mut bus);
            }
        }
    } else {
        post_boot_init(&mut cpu, &mut bus);
    }

    apply_overlay(&mut bus, overlay);

    // Load cartridge (no MBC => first 32 KiB)
    bus.load_rom(&rom);

    // ---- Run a little, then render a frame ----
    // (Soon you’ll interleave CPU cycles with PPU tick)
    for _ in 0..20_000 {
        cpu.step(&mut bus);
    }

    let mut fb: Framebuffer = [[0; 160]; 144];
    bus.render_frame(&mut fb);

    // Write PGM (grayscale, tiny)
    if let Err(e) = write_pgm("frame_overlay.pgm", &fb) {
        eprintln!("PGM write failed: {e}");
    } else {
        println!("Wrote frame_overlay.pgm");
    }

    // Or write PPM with default grayscale palette
    if let Err(e) = write_ppm("frame_overlay.ppm", &fb, None) {
        eprintln!("PPM write failed: {e}");
    } else {
        println!("Wrote frame_overlay.ppm");
    }

    // Or PPM with the alternate high-contrast palette (makes overlays pop)
    if let Err(e) = write_ppm("frame_overlay_hicon.ppm", &fb, Some(util::image::HIGH_CONTRAST_PAL)) {
        eprintln!("PPM write failed: {e}");
    }
    // Small diagnostics
    let ly = bus.read_byte(0xff44);
    let stat = bus.read_byte(0xff41);
    let iflg = bus.read_byte(0xff0f);
    println!("Rendered 1 frame. LY={ly}, STAT=0b{stat:08b}, IF=0b{iflg:08b}");
}

/// Fallback to “post‑boot” state if no boot ROM is provided.
/// Typical DMG values; exact list comes from boot behavior and legacy docs. [4](https://gbdev.gg8.se/wiki/articles/Power_Up_Sequence)
fn apply_overlay(bus: &mut MemoryBus, oa: OverlayArgs) {
    let mut cfg = DebugOverlayConfig::default();
    cfg.show_bg_tile_grid = oa.grid;
    cfg.show_window_bounds = oa.window;
    cfg.show_bg_axes = oa.axes;
    cfg.show_sprite_boxes = oa.sprites;

    // High-contrast DMG shades (0..3)
    cfg.shade_grid = 3; // black
    cfg.shade_window = 2; // dark gray
    cfg.shade_axes = 1; // light gray
    cfg.shade_sprite_box = 3; // black

    bus.set_gpu_debug_config(cfg);
}
fn post_boot_init(cpu: &mut CPU, bus: &mut MemoryBus) {
    // CPU registers
    cpu.regs.set_af(0x01b0); // A=0x01, F=0xB0 (uses your existing code)
    cpu.regs.set_bc(0x0013);
    cpu.regs.set_de(0x00d8);
    cpu.regs.set_hl(0x014d);
    cpu.sp = 0xfffe;
    cpu.pc = 0x0100;

    // PPU defaults commonly set by boot ROM
    bus.write_byte(0xff40, 0x91); // LCDC
    bus.write_byte(0xff42, 0x00); // SCY
    bus.write_byte(0xff43, 0x00); // SCX
    bus.write_byte(0xff47, 0xfc); // BGP
    bus.write_byte(0xff4a, 0x00); // WY
    bus.write_byte(0xff4b, 0x00); // WX
}

fn parse_overlay_list(s: &str) -> Result<OverlayArgs, String> {
    let mut oa = OverlayArgs::default();
    for tok in s.split(',').map(|t| t.trim().to_ascii_lowercase()) {
        if tok.is_empty() {
            continue;
        }
        match tok.as_str() {
            "grid" => {
                oa.grid = true;
            }
            "window" => {
                oa.window = true;
            }
            "axes" => {
                oa.axes = true;
            }
            "sprites" => {
                oa.sprites = true;
            }
            other => {
                return Err(format!("unknown overlay token: '{other}'"));
            }
        }
    }
    Ok(oa)
}
