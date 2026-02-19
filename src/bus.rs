// src/bus.rs
use crate::gpu::GPU;
use crate::timer::Timer;

const VRAM_BEGIN: usize = 0x8000;
const VRAM_END: usize = 0x9fff;

pub struct MemoryBus {
    // 64 KiB address space (0x0000..=0xFFFF)
    memory: [u8; 0x10000],
    pub gpu: GPU,
    timer: Timer,
}

impl MemoryBus {
    pub fn new() -> Self {
        Self { memory: [0; 0x10000], gpu: GPU::new(), timer: Timer::new() }
    }

    #[inline]
    pub fn read_byte(&self, address: u16) -> u8 {
        let addr = address as usize;
        match addr {
            VRAM_BEGIN..=VRAM_END => self.gpu.read_vram(addr - VRAM_BEGIN),
            0xff04..=0xff07 => self.timer.read_io(address), // DIV/TIMA/TMA/TAC
            _ => self.memory[addr],
        }
    }

    /// Load a ROM image into 0x0000.. (flat/no MBC for now).
    pub fn load_rom(&mut self, rom: &[u8]) {
        let max = (0x8000).min(rom.len());
        self.memory[0x0000..0x0000 + max].copy_from_slice(&rom[..max]);
    }

    #[inline]
    pub fn write_byte(&mut self, address: u16, value: u8) {
        let addr = address as usize;
        match addr {
            0x0000..=0x7fff => {/* ROM area: ignore for now */}
            VRAM_BEGIN..=VRAM_END => self.gpu.write_vram(addr - VRAM_BEGIN, value),
            0xff04..=0xff07 => self.timer.write_io(address, value), // DIV/TIMA/TMA/TAC
            _ => {
                self.memory[addr] = value;
            }
        }
    }

    /// Advance on-chip components by `cycles` CPU cycles.
    pub fn tick(&mut self, cycles: u32) {
        let overflow = self.timer.tick(cycles);
        if overflow {
            // IF register (0xFF0F): set bit-2 (Timer)
            let if_addr = 0xff0fusize;
            self.memory[if_addr] |= 0b0000_0100;
        }
    }
}
