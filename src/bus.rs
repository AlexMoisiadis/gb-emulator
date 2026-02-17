// src/bus.rs
#[derive(Debug, Clone)]
pub struct MemoryBus {
    // 64 KiB address space (0x0000..=0xFFFF), hence 0x10000 bytes
    pub memory: [u8; 0x10000],
}

impl Default for MemoryBus {
    fn default() -> Self {
        Self { memory: [0u8; 0x10000] }
    }
}

impl MemoryBus {
    #[inline]
    pub fn read_byte(&self, address: u16) -> u8 {
        self.memory[address as usize]
    }

    #[inline]
    pub fn write_byte(&mut self, address: u16, value: u8) {
        self.memory[address as usize] = value;
    }
}