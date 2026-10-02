// src/gameboy.rs — one struct for frontends that just want to run frames.

use crate::gpu::{ LCD_HEIGHT, LCD_WIDTH };
use crate::input::joypad::Button;
use crate::{ CPU, MemoryBus };

const DOTS_PER_FRAME: u32 = 70_224;

/// Register and LCD state the DMG boot ROM leaves behind, so carts start at
/// $0100 without a boot ROM. Shared by every frontend.
pub fn post_boot_init(cpu: &mut CPU, bus: &mut MemoryBus) {
    cpu.regs.set_af(0x01b0);
    cpu.regs.set_bc(0x0013);
    cpu.regs.set_de(0x00d8);
    cpu.regs.set_hl(0x014d);
    cpu.sp = 0xfffe;
    cpu.pc = 0x0100;

    bus.write_byte(0xff40, 0x91); // LCDC
    bus.write_byte(0xff42, 0x00); // SCY
    bus.write_byte(0xff43, 0x00); // SCX
    bus.write_byte(0xff47, 0xfc); // BGP
    bus.write_byte(0xff4a, 0x00); // WY
    bus.write_byte(0xff4b, 0x00); // WX
}

pub struct GameBoy {
    cpu: CPU,
    bus: MemoryBus,
    /// T-cycles the last instruction ran past the frame boundary.
    overshoot: u32,
}

impl GameBoy {
    /// Errors (rather than panicking) on a malformed or unsupported cart.
    pub fn new(rom: &[u8]) -> anyhow::Result<Self> {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();
        post_boot_init(&mut cpu, &mut bus);
        bus.mmu.try_load_rom(rom)?;
        Ok(Self { cpu, bus, overshoot: 0 })
    }

    /// Run one frame's worth of T-cycles. A fixed budget rather than "until
    /// VBlank", so a ROM with the LCD off can't hang the caller.
    pub fn run_frame(&mut self) {
        let mut t = self.overshoot;
        while t < DOTS_PER_FRAME {
            t += self.cpu.step(&mut self.bus);
        }
        self.overshoot = t - DOTS_PER_FRAME;
        self.bus.gpu.take_frame_ready();
    }

    /// Shade indices 0–3; the frontend applies the palette.
    pub fn framebuffer(&self) -> &[[u8; LCD_WIDTH]; LCD_HEIGHT] {
        self.bus.gpu.frame()
    }

    /// Bit 0..7 = Right, Left, Up, Down, A, B, Select, Start (1 = pressed).
    pub fn set_buttons(&mut self, pressed: u8) {
        use Button::*;
        for (bit, button) in [Right, Left, Up, Down, A, B, Select, Start].into_iter().enumerate() {
            if pressed & (1 << bit) != 0 {
                self.bus.press_button(button);
            } else {
                self.bus.release_button(button);
            }
        }
    }

    pub fn set_sample_rate(&mut self, hz: f32) {
        self.bus.apu.set_sample_rate(hz);
    }

    /// Interleaved stereo f32. Call every frame, or the buffer keeps growing.
    pub fn drain_audio(&mut self, out: &mut Vec<f32>) {
        self.bus.apu.drain_samples(out);
    }

    pub fn cart_ram(&self) -> Option<&[u8]> {
        self.bus.mmu.cart.ram()
    }

    /// Copies as much of `data` as fits; a short or long save is tolerated.
    pub fn load_cart_ram(&mut self, data: &[u8]) {
        if let Some(ram) = self.bus.mmu.cart.ram_mut() {
            let n = ram.len().min(data.len());
            ram[..n].copy_from_slice(&data[..n]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rom(cart_type: u8, ram_size: u8) -> Vec<u8> {
        let mut rom = vec![0u8; 0x8000]; // all NOPs
        rom[0x147] = cart_type;
        rom[0x149] = ram_size;
        rom
    }

    #[test]
    fn bad_carts_are_errors_not_panics() {
        assert!(GameBoy::new(&[0u8; 0x10]).is_err()); // too small for a header
        assert!(GameBoy::new(&rom(0x19, 0)).is_err()); // MBC5, unsupported
    }

    #[test]
    fn run_frame_spends_one_frame_budget() {
        let mut gb = GameBoy::new(&rom(0x00, 0)).unwrap();
        for _ in 0..3 {
            gb.run_frame();
            assert!(gb.overshoot < 24, "overshoot {} is more than one instruction", gb.overshoot);
        }
    }

    #[test]
    fn cart_ram_round_trips() {
        assert!(GameBoy::new(&rom(0x00, 0)).unwrap().cart_ram().is_none());
        let mut gb = GameBoy::new(&rom(0x03, 0x02)).unwrap(); // MBC1+RAM+BATTERY, 8 KiB
        gb.load_cart_ram(&[1, 2, 3]);
        assert_eq!(&gb.cart_ram().unwrap()[..4], &[1, 2, 3, 0xff]);
    }
}
