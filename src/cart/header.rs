use anyhow::{ bail, Result };

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapperKind {
    None, // 0x00
    Mbc1, // 0x01..=0x03
    Mbc2, // 0x05..=0x06
    Mbc3, // 0x0F..=0x13
    Mbc5, // 0x19..=0x1E
    // (others omitted for brevity)
}

#[derive(Debug, Clone)]
pub struct CartHeader {
    pub cart_type: u8, // 0x0147
    pub rom_size: u8, // 0x0148
    pub ram_size: u8, // 0x0149
}

pub fn parse_header(rom: &[u8]) -> Result<CartHeader> {
    if rom.len() < 0x0150 {
        bail!("ROM too small to contain header");
    }
    Ok(CartHeader {
        cart_type: rom[0x0147],
        rom_size: rom[0x0148],
        ram_size: rom[0x0149],
    })
}

pub fn detect_mapper(cart_type: u8) -> MapperKind {
    match cart_type {
        0x00 => MapperKind::None,
        0x01 | 0x02 | 0x03 => MapperKind::Mbc1,
        0x05 | 0x06 => MapperKind::Mbc2,
        0x0f..=0x13 => MapperKind::Mbc3,
        0x19..=0x1e => MapperKind::Mbc5,
        _ => MapperKind::None, // default; you may want a MapperKind::Unsupported(u8)
    }
}
