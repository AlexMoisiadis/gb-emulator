use gb_emulator::{ CPU, MemoryBus };
use std::{ fs::File, path::Path };
use png::{ Encoder, ColorType, BitDepth };

const W: usize = 160;
const H: usize = 144;

fn dmg_shade_to_u8(v: u8) -> u8 {
    match v & 0b11 {
        0 => 255,
        1 => 170,
        2 => 85,
        _ => 0,
    }
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
    for f in 0..3 {
        let mut safety_dots = 0u32;
        while !bus.gpu.frame_is_ready() && safety_dots < 70_224 {
            let cy = cpu.step(&mut bus);
            safety_dots = safety_dots.saturating_add(cy);
        }
        if bus.gpu.take_frame_ready() {
            bus.gpu.copy_frame(&mut fb);
            save_png(&fb, format!("frame-{f:03}.png"))?;
        }
    }
    Ok(())
}
