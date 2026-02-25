pub trait Mapper: Send {
    /// Cartridge ROM read (0x0000..=0x7FFF)
    fn read_rom(&self, addr: u16) -> u8;

    /// Cartridge external RAM read (0xA000..=0xBFFF)
    fn read_ram(&self, addr: u16) -> u8;

    /// Writes to MBC control regions or ext RAM
    fn write(&mut self, addr: u16, value: u8);
}
