mod header;
mod mapper;
mod none;
mod mbc1;

use anyhow::{ bail, Result };
use header::{ parse_header, detect_mapper, MapperKind, CartHeader };
use mapper::Mapper;
use none::NoMbc;
use mbc1::Mbc1;

use std::path::Path;

pub struct Cartridge {
    pub header: CartHeader,
    pub mapper_kind: MapperKind,
    inner: Box<dyn Mapper>,
}

impl Cartridge {
    /// Safe default cartridge used before a ROM is loaded.
    pub fn empty() -> Self {
        let header = CartHeader {
            cart_type: 0x00,
            rom_size: 0x00,
            ram_size: 0x00,
        };
        let inner: Box<dyn Mapper> = Box::new(NoMbc::new(vec![0xff; 0x8000]));
        Self {
            header,
            mapper_kind: MapperKind::None,
            inner,
        }
    }

    /// Build from a file path at the application boundary
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let bytes = std::fs::read(path)?;
        Self::from_bytes(bytes)
    }

    pub fn from_bytes(rom: Vec<u8>) -> Result<Self> {
        let header = parse_header(&rom)?;
        let mapper_kind = detect_mapper(header.cart_type);

        #[cfg(feature = "debug_timing")]
        eprintln!(
            "[CART] type={:#04x} mapper={:?} rom_size={:#04x} ram_size={:#04x}",
            header.cart_type,
            mapper_kind,
            header.rom_size,
            header.ram_size
        );

        let inner: Box<dyn Mapper> = match mapper_kind {
            MapperKind::None => Box::new(NoMbc::new(rom)),
            MapperKind::Mbc1 => Box::new(Mbc1::new(rom, &header)?),
            // MapperKind::Mbc2 | MapperKind::Mbc3 | MapperKind::Mbc5 => bail until implemented:
            _ =>
                bail!(
                    "Unsupported cartridge type {:?} (0x{:02X}). Implement this mapper.",
                    mapper_kind,
                    header.cart_type
                ),
        };

        Ok(Cartridge { header, mapper_kind, inner })
    }

    #[inline]
    pub fn read_rom(&self, addr: u16) -> u8 {
        self.inner.read_rom(addr)
    }
    #[inline]
    pub fn read_ram(&self, addr: u16) -> u8 {
        self.inner.read_ram(addr)
    }
    #[inline]
    pub fn write(&mut self, addr: u16, value: u8) {
        self.inner.write(addr, value)
    }

    pub fn ram(&self) -> Option<&[u8]> {
        self.inner.ram()
    }
    pub fn ram_mut(&mut self) -> Option<&mut [u8]> {
        self.inner.ram_mut()
    }
}
