use super::mapper::Mapper;

pub struct NoMbc {
    rom: Vec<u8>,
}

impl NoMbc {
    pub fn new(rom: Vec<u8>) -> Self {
        Self { rom }
    }
}

impl Mapper for NoMbc {
    fn read_rom(&self, addr: u16) -> u8 {
        self.rom
            .get(addr as usize)
            .copied()
            .unwrap_or(0xff)
    }
    fn read_ram(&self, _addr: u16) -> u8 {
        0xff
    }
    fn write(&mut self, _addr: u16, _value: u8) {/* no-op */}
}
