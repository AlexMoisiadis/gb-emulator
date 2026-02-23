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
    memory: Box<[u8; 0x10000]>, // heap, not stack

    // --- Boot ROM support (DMG) ---
    boot_rom: Option<Vec<u8>>,
    boot_enabled: bool,
}

impl MMU {
    pub fn new() -> Self {
        Self {
            memory: Box::new([0u8; 0x10000]),
            boot_rom: None,
            boot_enabled: false, // enable only when a boot ROM is loaded
        }
    }

    /// OR the Interrupt Flag (IF/FF0F) with `mask`.

    #[inline]
    pub fn set_if_bits(&mut self, mask: u8) {
        let idx = IO_IF as usize;
        self.memory[idx] |= mask; // <-- OR-in, do not overwrite
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

            // --- GPU I/O registers (read paths) ---
            IO_LCDC => gpu.get_lcdc(),
            IO_SCY => gpu.get_scy(),
            IO_SCX => gpu.get_scx(),
            IO_BGP => gpu.get_bgp(),
            IO_OBP0 => gpu.get_obp0(),
            IO_OBP1 => gpu.get_obp1(),
            IO_WY => gpu.get_wy(),
            IO_WX => gpu.get_wx(),

            // STAT/LY/LYC from the PPU mirrors
            0xff41 => gpu.read_stat(), // STAT
            0xff44 => gpu.ly(), // LY (read-only)
            0xff45 => gpu.read_lyc(), // LYC

            // --- Timer I/O (DIV/TIMA/TMA/TAC) ---
            0xff04..=0xff07 => timer.read_io(addr),

            // --- Everything else -> flat memory ---
            _ => self.memory[addr as usize],
        }
    }

    /// Write a byte with address decoding and device forwarding.

    /// Write a byte with address decoding and device forwarding.
    pub fn write8(&mut self, addr: u16, value: u8, gpu: &mut GPU, timer: &mut Timer) {
        match addr {
            // --- Cartridge ROM: OK to ignore for "no-MBC" builds ---
            0x0000..=0x7fff => {/* no-op for now */}

            // --- VRAM: tile patterns (0x8000..=0x97FF) & tile maps (0x9800..=0x9FFF) ---
            VRAM_BEGIN..=VRAM_END => {
                // You already have write_vram_abs; either is fine. Using abs keeps the cache logic centralized.
                // gpu.write_vram(ix, value);
                gpu.write_vram_abs(addr, value);
            }

            // --- OAM: sprite attribute table ---
            OAM_BEGIN..=OAM_END => {
                let off = (addr - OAM_BEGIN) as usize;
                gpu.write_oam(off, value);
            }

            // --- GPU I/O registers (write paths) ---
            IO_LCDC => gpu.set_lcdc(value), // LCDC (enable, tile region selects)
            IO_SCY => gpu.set_scy(value), // SCY
            IO_SCX => gpu.set_scx(value), // SCX
            IO_BGP => gpu.set_bgp(value), // DMG BG palette
            IO_OBP0 => gpu.set_obp0(value), // DMG OBJ palette 0
            IO_OBP1 => gpu.set_obp1(value), // DMG OBJ palette 1
            IO_WY => gpu.set_wy(value), // WY
            IO_WX => gpu.set_wx(value), // WX (renderer applies WX-7 internally)

            // --- STAT/LYC mirrors in the PPU state machine ---
            0xff41 => gpu.write_stat(value), // STAT (mode-int enables; coincidence flag set by PPU)
            0xff45 => gpu.write_lyc(value), // LYC (compare value for LY)

            // --- OAM DMA (FF46): instant copy XX00..XX9F -> FE00..FE9F ---
            IO_DMA => self.do_oam_dma_instant(value, gpu, timer),

            // --- FF50 — disable Boot ROM (bit0=1 -> unmap) ---
            IO_BOOT => {
                // Optional mirror of last write; not required by HW, but harmless.
                self.memory[addr as usize] = value;

                // Log before clearing the flag
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
