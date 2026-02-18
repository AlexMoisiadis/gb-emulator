// src/bus.rs
use crate::gpu::GPU;

const VRAM_BEGIN: usize = 0x8000;
const VRAM_END: usize = 0x9fff;

pub struct MemoryBus {
    // 64 KiB address space (0x0000..=0xFFFF)
    memory: [u8; 0x10000],
    pub gpu: GPU,
}

impl MemoryBus {
    pub fn new() -> Self {
        Self { memory: [0; 0x10000], gpu: GPU::new() }
    }

    #[inline]
    pub fn read_byte(&self, address: u16) -> u8 {
        let address = address as usize;
        match address {
            VRAM_BEGIN..=VRAM_END => self.gpu.read_vram(address - VRAM_BEGIN),
            _ => self.memory[address], // temporary until you map other regions
        }
    }

    pub fn load_rom(&mut self, rom: &[u8]) {
        let max = (0x8000).min(rom.len()); // simple 32 KiB flat ROM
        self.memory[0x0000..0x0000 + max].copy_from_slice(&rom[..max]);
    }

    #[inline]
    pub fn write_byte(&mut self, address: u16, value: u8) {
        let address = address as usize;
        match address {
            0x0000..=0x7fff => {/* ignore: ROM */}
            0x8000..=0x9fff => self.gpu.write_vram(address - 0x8000, value),
            _ => {
                self.memory[address] = value;
            }
        }
    }
}
