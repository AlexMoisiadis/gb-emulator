pub trait Mapper: Send {
    /// Cartridge ROM read (0x0000..=0x7FFF)
    fn read_rom(&self, addr: u16) -> u8;

    /// Cartridge external RAM read (0xA000..=0xBFFF)
    fn read_ram(&self, addr: u16) -> u8;

    /// Writes to MBC control regions or ext RAM
    fn write(&mut self, addr: u16, value: u8);

    /// Whole external RAM, for saving and restoring battery-backed carts.
    /// `None` when the cart has no RAM.
    fn ram(&self) -> Option<&[u8]> {
        None
    }

    fn ram_mut(&mut self) -> Option<&mut [u8]> {
        None
    }
}
