// src/mmu.rs
use crate::gpu::GPU;
use crate::timer::Timer;

/// VRAM and OAM windows for address decoding (DMG).
const VRAM_BEGIN: u16 = 0x8000;
const VRAM_END: u16 = 0x9fff;

const OAM_BEGIN: u16 = 0xfe00;
const OAM_END: u16 = 0xfe9f;

/// I/O registers we forward to the GPU
const IO_LCDC: u16 = 0xff40;
const IO_SCY: u16 = 0xff42;
const IO_SCX: u16 = 0xff43;
const IO_BGP: u16 = 0xff47;
const IO_OBP0: u16 = 0xff48;
const IO_OBP1: u16 = 0xff49;
const IO_WY: u16 = 0xff4a;
const IO_WX: u16 = 0xff4b;

/// Interrupt Flag register (IF)
const IO_IF: u16 = 0xff0f;

/// OAM DMA register
const IO_DMA: u16 = 0xff46;

/// Boot ROM disable register
const IO_BOOT: u16 = 0xff50;

/// Minimal MMU that holds a flat 64 KiB memory and performs address decode.
/// GPU & Timer are passed in for device-backed regions and I/O.
pub struct MMU {
    memory: [u8; 0x10000],

    // --- Boot ROM support (DMG) ---
    boot_rom: Option<Vec<u8>>,
    boot_enabled: bool,
}

impl MMU {
    pub fn new() -> Self {
        Self {
            memory: [0; 0x10000],
            boot_rom: None,
            boot_enabled: false, // enable only when a boot ROM is loaded
        }
    }

    /// OR the Interrupt Flag (IF/FF0F) with `mask`.
    #[inline]
    pub fn set_if_bits(&mut self, mask: u8) {
        let idx = IO_IF as usize;
        self.memory[idx] |= mask; // <-- fix: OR in the bits rather than overwrite
    }

    /// Load a flat ROM into 0000..7FFF (no MBC yet).
    pub fn load_rom(&mut self, rom: &[u8]) {
        let max = (0x8000).min(rom.len());
        self.memory[0x0000..0x0000 + max].copy_from_slice(&rom[..max]);
    }

    /// Install a DMG boot ROM (256 bytes). Enables boot ROM mapping.
    pub fn load_boot_rom(&mut self, bytes: Vec<u8>) {
        assert!(bytes.len() == 0x100, "DMG boot ROM must be 256 bytes");
        self.boot_rom = Some(bytes);
        self.boot_enabled = true; // start with boot ROM mapped
    }

    /// Read a byte with address decoding and device forwarding.
    pub fn read8(&self, addr: u16, gpu: &GPU, timer: &Timer) -> u8 {
        // BOOT ROM overlay: 0000–00FF while enabled
        if self.boot_enabled && addr <= 0x00ff {
            if let Some(br) = &self.boot_rom {
                return br[addr as usize];
            }
        }

        match addr {
            // --- GPU-backed regions ---
            VRAM_BEGIN..=VRAM_END => {
                let ix = (addr - VRAM_BEGIN) as usize;
                gpu.read_vram(ix)
            }
            OAM_BEGIN..=OAM_END => {
                let off = (addr - OAM_BEGIN) as usize;
                gpu.read_oam(off)
            }

            // --- GPU I/O registers ---
            IO_LCDC => gpu.get_lcdc(),
            IO_SCY => gpu.get_scy(),
            IO_SCX => gpu.get_scx(),
            IO_BGP => gpu.get_bgp(),
            IO_OBP0 => gpu.get_obp0(),
            IO_OBP1 => gpu.get_obp1(),
            IO_WY => gpu.get_wy(),
            IO_WX => gpu.get_wx(),

            // --- Timer I/O (DIV/TIMA/TMA/TAC) ---
            0xff04..=0xff07 => timer.read_io(addr),

            // --- Everything else -> flat memory ---
            _ => self.memory[addr as usize],
        }
    }

    /// Write a byte with address decoding and device forwarding.
    pub fn write8(&mut self, addr: u16, value: u8, gpu: &mut GPU, timer: &mut Timer) {
        match addr {
            // ROM: ignore (no MBC yet)
            0x0000..=0x7fff => {}

            // --- GPU-backed regions ---
            VRAM_BEGIN..=VRAM_END => {
                let ix = (addr - VRAM_BEGIN) as usize;
                gpu.write_vram(ix, value);
            }
            OAM_BEGIN..=OAM_END => {
                let off = (addr - OAM_BEGIN) as usize;
                gpu.write_oam(off, value);
            }

            // --- GPU I/O registers ---
            IO_LCDC => gpu.set_lcdc(value),
            IO_SCY => gpu.set_scy(value),
            IO_SCX => gpu.set_scx(value),
            IO_BGP => gpu.set_bgp(value),
            IO_OBP0 => gpu.set_obp0(value),
            IO_OBP1 => gpu.set_obp1(value),
            IO_WY => gpu.set_wy(value),
            IO_WX => gpu.set_wx(value),

            // --- OAM DMA (FF46): instant copy XX00..XX9F -> FE00..FE9F ---
            IO_DMA => self.do_oam_dma_instant(value, gpu, timer),

            // --- FF50 — disable Boot ROM (bit0 set => unmap) ---
            IO_BOOT => {
                self.memory[addr as usize] = value; // optional 1-byte mirror

                // Log the transition BEFORE clearing boot_enabled
                println!(
                    "Boot ROM disabled via write to FF50: value=0x{:02X} (bit0={})",
                    value,
                    value & 1
                );

                if (value & 0x01) != 0 {
                    self.boot_enabled = false;
                }
            }

            // --- Timer I/O (DIV/TIMA/TMA/TAC) ---
            0xff04..=0xff07 => timer.write_io(addr, value),

            // --- Everything else -> flat memory ---
            _ => {
                self.memory[addr as usize] = value;
            }
        }
    }

    /// Simplified OAM DMA: copy 160 bytes from XX00..XX9F into FE00..FE9F now.
    fn do_oam_dma_instant(&mut self, src_high: u8, gpu: &mut GPU, timer: &Timer) {
        let base: u16 = (src_high as u16) << 8; // XX00
        for i in 0..160u16 {
            let b = self.read8(base + i, gpu, timer);
            let off = i as usize;
            gpu.write_oam(off, b);
        }
    }
}
