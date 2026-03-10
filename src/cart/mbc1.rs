use anyhow::Result;

use super::header::CartHeader;
use super::mapper::Mapper;

const RAM_BANK_SIZE: usize = 0x2000; // 8 KiB external RAM bank
const LOW5_MASK: u8 = 0x1f; // lower 5 ROM bank bits (0x2000–0x3FFF writes)
const HI2_MASK: u8 = 0x03; // upper 2 bank bits (0x4000–0x5FFF writes)
const MODE_MASK: u8 = 0x01; // mode select bit (0x6000–0x7FFF writes)
const RAM_ENABLE_MAGIC: u8 = 0x0a; // lower nibble value that enables external RAM

pub struct Mbc1 {
    rom: Vec<u8>,
    rom_bank: u8, // low 5 bits from 0x2000..=0x3FFF
    bank_hi2: u8, // upper 2 bits from 0x4000..=0x5FFF
    mode_rom: bool, // true=mode 0 (ROM banking), false=mode 1 (RAM banking)
    rom_banks: usize, // total count of 16KiB ROM banks
    ram: Vec<u8>,
    ram_enable: bool,
}

impl Mbc1 {
    pub fn new(rom: Vec<u8>, hdr: &CartHeader) -> Result<Self> {
        // Ceil division keeps odd ROM lengths addressable without panicking.
        let rom_banks = ((rom.len() + 0x3fff) / 0x4000).max(1);
        let ram_size = Self::ram_size_bytes(hdr.ram_size);
        Ok(Self {
            rom,
            rom_bank: 1, // MBC1 coerces 0 -> 1 in lower 5 bits
            bank_hi2: 0,
            mode_rom: true,
            rom_banks,
            ram: vec![0xff; ram_size],
            ram_enable: false,
        })
    }

    #[inline]
    fn ram_size_bytes(ram_size_code: u8) -> usize {
        match ram_size_code {
            0x00 => 0,
            0x01 => 2 * 1024,
            0x02 => 8 * 1024,
            0x03 => 32 * 1024,
            0x04 => 128 * 1024,
            0x05 => 64 * 1024,
            _ => 0,
        }
    }

    #[inline]
    fn wrap_bank(&self, bank: usize) -> usize {
        bank % self.rom_banks
    }

    #[inline]
    fn low5_coerced(&self) -> usize {
        let low5 = (self.rom_bank & 0x1f) as usize;
        if low5 == 0 {
            1
        } else {
            low5
        }
    }

    #[inline]
    fn lower_window_bank(&self) -> usize {
        // Mode 0: fixed bank 0. Mode 1: bank 00/20/40/60 via hi2.
        if self.mode_rom {
            0
        } else {
            self.wrap_bank((self.bank_hi2 as usize) << 5)
        }
    }

    #[inline]
    fn upper_window_bank(&self) -> usize {
        let bank = ((self.bank_hi2 as usize) << 5) | self.low5_coerced();
        self.wrap_bank(bank)
    }

    #[inline]
    fn ram_bank(&self) -> usize {
        if self.mode_rom { 0 } else { (self.bank_hi2 & 0x03) as usize }
    }

    #[inline]
    fn rom_byte(&self, bank: usize, ofs: usize) -> u8 {
        let addr = self.wrap_bank(bank) * 0x4000 + (ofs & 0x3fff);
        self.rom.get(addr).copied().unwrap_or(0xff)
    }
}

impl Mapper for Mbc1 {
    fn read_rom(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x3fff => self.rom_byte(self.lower_window_bank(), addr as usize),
            0x4000..=0x7fff => {
                let bank = self.upper_window_bank();
                self.rom_byte(bank, (addr as usize) - 0x4000)
            }
            _ => 0xff,
        }
    }

    fn read_ram(&self, _addr: u16) -> u8 {
        if !self.ram_enable || self.ram.is_empty() {
            return 0xff;
        }
        let addr = _addr as usize;
        if !(0xa000..=0xbfff).contains(&addr) {
            return 0xff;
        }
        let bank = self.ram_bank();
        let offset = addr - 0xa000;
        let idx = bank * RAM_BANK_SIZE + offset;
        self.ram.get(idx).copied().unwrap_or(0xff)
    }

    fn write(&mut self, addr: u16, value: u8) {
        match addr {
            // 0000-1FFF: RAM enable latch.
            0x0000..=0x1fff => {
                self.ram_enable = (value & 0x0f) == RAM_ENABLE_MAGIC;
            }

            // 2000-3FFF: ROM bank low5
            0x2000..=0x3fff => {
                self.rom_bank = value & LOW5_MASK;
                #[cfg(feature = "debug_timing")]
                eprintln!(
                    "[MBC1] set low5={:02X} -> high_bank={}",
                    self.rom_bank,
                    self.upper_window_bank()
                );
            }

            // 4000-5FFF: bank hi2 (ROM upper bits in mode 0, low-window bank in mode 1)
            0x4000..=0x5fff => {
                self.bank_hi2 = value & HI2_MASK;
                #[cfg(feature = "debug_timing")]
                eprintln!(
                    "[MBC1] set hi2={:02X} -> low_bank={} high_bank={}",
                    self.bank_hi2,
                    self.lower_window_bank(),
                    self.upper_window_bank()
                );
            }

            // 6000-7FFF: mode select (0=ROM banking, 1=RAM banking)
            0x6000..=0x7fff => {
                self.mode_rom = (value & MODE_MASK) == 0;
                #[cfg(feature = "debug_timing")]
                eprintln!("[MBC1] mode set: ROM={}.", self.mode_rom);
            }

            // A000-BFFF: ext RAM writes (ignored here)
            0xa000..=0xbfff => {
                if !self.ram_enable || self.ram.is_empty() {
                    return;
                }
                let bank = self.ram_bank();
                let idx = bank * RAM_BANK_SIZE + ((addr as usize) - 0xa000);
                if let Some(slot) = self.ram.get_mut(idx) {
                    *slot = value;
                }
            }

            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hdr(ram_size: u8) -> CartHeader {
        CartHeader {
            cart_type: 0x03,
            rom_size: 0x00,
            ram_size,
        }
    }

    fn build_rom(banks: usize) -> Vec<u8> {
        let mut rom = vec![0u8; banks * 0x4000];
        for b in 0..banks {
            let base = b * 0x4000;
            rom[base] = b as u8;
            rom[base + 0x0100] = b as u8;
        }
        rom
    }

    #[test]
    fn low5_zero_is_coerced_to_one() {
        let rom = build_rom(8);
        let mut m = Mbc1::new(rom, &hdr(0x00)).expect("mbc1");
        m.write(0x2000, 0x00);
        assert_eq!(m.read_rom(0x4000), 1);
    }

    #[test]
    fn mode1_switches_lower_window_bank_group() {
        let rom = build_rom(128);
        let mut m = Mbc1::new(rom, &hdr(0x00)).expect("mbc1");
        m.write(0x6000, 0x01); // mode 1
        m.write(0x4000, 0x02); // hi2=2 => lower bank starts at 64
        assert_eq!(m.read_rom(0x0000), 64);
    }

    #[test]
    fn non_power_of_two_bank_count_wraps_safely() {
        let rom = build_rom(72);
        let mut m = Mbc1::new(rom, &hdr(0x00)).expect("mbc1");
        m.write(0x4000, 0x03);
        m.write(0x2000, 0x1f);
        let v = m.read_rom(0x4000);
        assert!(v < 72);
    }

    #[test]
    fn ram_banking_works_in_mode1() {
        let rom = build_rom(8);
        let mut m = Mbc1::new(rom, &hdr(0x03)).expect("mbc1");
        m.write(0x0000, 0x0a); // enable RAM
        m.write(0x6000, 0x01); // mode 1

        m.write(0x4000, 0x00);
        m.write(0xa000, 0x11);
        m.write(0x4000, 0x01);
        m.write(0xa000, 0x22);

        m.write(0x4000, 0x00);
        assert_eq!(m.read_ram(0xa000), 0x11);
        m.write(0x4000, 0x01);
        assert_eq!(m.read_ram(0xa000), 0x22);
    }
}
