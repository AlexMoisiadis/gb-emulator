use anyhow::Result;
use super::header::CartHeader;
use super::mapper::Mapper;

pub struct Mbc1 {
    rom: Vec<u8>,
    // Optional RAM not implemented in this minimal version
    rom_bank: u8, // 7-bit field: low5 + high2
    bank_hi2: u8, // upper 2 bits from 0x4000..=0x5FFF
    mode_rom: bool, // false=ROM banking mode (we’ll keep false here)
    rom_mask: usize, // (rom.len()/0x4000)-1 for wrapping
}

impl Mbc1 {
    pub fn new(rom: Vec<u8>, _hdr: CartHeader) -> Result<Self> {
        // rom length should be multiple of 16KB; mask helps wrap safely
        let banks = (rom.len() / 0x4000).max(1);
        let rom_mask = banks - 1;
        Ok(Self {
            rom,
            rom_bank: 1, // MBC1 coerces 0 -> 1
            bank_hi2: 0,
            mode_rom: true, // stick to ROM banking mode in this minimal version
            rom_mask,
        })
    }

    fn effective_bank(&self) -> usize {
        // Combine hi2:low5 (but coerce 0->1 at low5 stage)
        let mut bank = (self.bank_hi2 << 5) | (self.rom_bank & 0x1f);
        if (bank & 0x1f) == 0 {
            bank |= 0x01;
        }
        (bank as usize) & self.rom_mask
    }

    #[inline]
    fn rom_byte(&self, bank: usize, ofs: usize) -> u8 {
        let addr = (bank * 0x4000 + ofs) & (self.rom.len() - 1);
        self.rom[addr]
    }
}

impl Mapper for Mbc1 {
    fn read_rom(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x3fff => self.rom_byte(0, addr as usize),
            0x4000..=0x7fff => {
                let bank = self.effective_bank();
                self.rom_byte(bank, (addr as usize) - 0x4000)
            }
            _ => 0xff,
        }
    }

    fn read_ram(&self, _addr: u16) -> u8 {
        // Not implemented in this minimal version
        0xff
    }

    fn write(&mut self, addr: u16, value: u8) {
        match addr {
            // 0000–1FFF: RAM enable (ignored in ROM-only minimal)
            0x0000..=0x1fff => {/* ignore */}

            // 2000–3FFF: ROM bank low5
            0x2000..=0x3fff => {
                let low5 = value & 0x1f;
                self.rom_bank = (self.rom_bank & !0x1f) | low5;
                #[cfg(feature = "debug_timing")]
                eprintln!("[MBC1] set low5={:02X} -> bank={}", low5, self.effective_bank());
            }

            // 4000–5FFF: bank hi2 (for large ROMs)
            0x4000..=0x5fff => {
                self.bank_hi2 = value & 0x03;
                #[cfg(feature = "debug_timing")]
                eprintln!("[MBC1] set hi2={:02X} -> bank={}", self.bank_hi2, self.effective_bank());
            }

            // 6000–7FFF: ROM/RAM mode (we hold ROM banking mode)
            0x6000..=0x7fff => {
                self.mode_rom = (value & 0x01) == 0;
                #[cfg(feature = "debug_timing")]
                eprintln!("[MBC1] mode set: ROM={}", self.mode_rom);
            }

            // A000–BFFF: ext RAM writes (ignored here)
            0xa000..=0xbfff => {/* no RAM in minimal version */}

            _ => {}
        }
    }
}
