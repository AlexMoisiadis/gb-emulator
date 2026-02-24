// src/mmu.rs
use crate::gpu::GPU;
use crate::timer::Timer;

// VRAM and OAM windows (DMG)
const VRAM_BEGIN: u16 = 0x8000;
const VRAM_END: u16 = 0x9fff;
const OAM_BEGIN: u16 = 0xfe00;
const OAM_END: u16 = 0xfe9f;

// GPU I/O
const IO_LCDC: u16 = 0xff40;
const IO_SCY: u16 = 0xff42;
const IO_SCX: u16 = 0xff43;
const IO_BGP: u16 = 0xff47;
const IO_OBP0: u16 = 0xff48;
const IO_OBP1: u16 = 0xff49;
const IO_WY: u16 = 0xff4a;
const IO_WX: u16 = 0xff4b;

// STAT/LY/LYC
const IO_STAT: u16 = 0xff41;
const IO_LY: u16 = 0xff44;
const IO_LYC: u16 = 0xff45;

// OAM DMA
const IO_DMA: u16 = 0xff46;

// IF (Interrupt Flag) and FF50 (boot)
const IO_IF: u16 = 0xff0f;
const IO_BOOT: u16 = 0xff50;

pub struct MMU {
    memory: Box<[u8; 0x10000]>,
    boot_rom: Option<Vec<u8>>,
    boot_enabled: bool,
}

impl MMU {
    pub fn new() -> Self {
        Self {
            memory: Box::new([0u8; 0x10000]),
            boot_rom: None,
            boot_enabled: false,
        }
    }

    #[inline]
    pub fn set_if_bits(&mut self, mask: u8) {
        let idx = IO_IF as usize;
        self.memory[idx] |= mask; // OR-in (do not overwrite)
    }

    pub fn load_rom(&mut self, rom: &[u8]) {
        let max = (0x8000).min(rom.len());
        self.memory[0x0000..0x0000 + max].copy_from_slice(&rom[..max]);
    }

    pub fn load_boot_rom(&mut self, bytes: Vec<u8>) {
        assert!(bytes.len() == 0x100, "DMG boot ROM must be 256 bytes");
        self.boot_rom = Some(bytes);
        self.boot_enabled = true;
    }

    /// CPU read; forwards to GPU/Timer or flat memory.
    pub fn read8(&self, addr: u16, gpu: &GPU, timer: &Timer) -> u8 {
        // DMG bus gating during OAM DMA: CPU can only access HRAM ($FF80..$FFFE)
        if gpu.dma_in_progress() && !(0xff80..=0xfffe).contains(&addr) {
            return 0xff;
        }

        // Boot ROM overlay 0000..00FF
        if self.boot_enabled && addr <= 0x00ff {
            if let Some(br) = &self.boot_rom {
                return br[addr as usize];
            }
        }

        match addr {
            VRAM_BEGIN..=VRAM_END => {
                let ix = (addr - VRAM_BEGIN) as usize;
                gpu.read_vram(ix)
            }
            OAM_BEGIN..=OAM_END => {
                let off = (addr - OAM_BEGIN) as usize;
                gpu.read_oam(off)
            }

            IO_LCDC => gpu.get_lcdc(),
            IO_SCY => gpu.get_scy(),
            IO_SCX => gpu.get_scx(),
            IO_BGP => gpu.get_bgp(),
            IO_OBP0 => gpu.get_obp0(),
            IO_OBP1 => gpu.get_obp1(),
            IO_WY => gpu.get_wy(),
            IO_WX => gpu.get_wx(),

            IO_STAT => gpu.read_stat(),
            IO_LY => gpu.ly(),
            IO_LYC => gpu.read_lyc(),

            0xff04..=0xff07 => timer.read_io(addr),

            _ => self.memory[addr as usize],
        }
    }

    /// CPU write; forwards to GPU/Timer or flat memory.
    pub fn write8(&mut self, addr: u16, value: u8, gpu: &mut GPU, timer: &mut Timer) {
        // DMG bus gating during OAM DMA: CPU writes outside HRAM are ignored
        if gpu.dma_in_progress() && !(0xff80..=0xfffe).contains(&addr) {
            return;
        }

        match addr {
            0x0000..=0x7fff => {/* no-MBC: ignore mapper writes for now */}

            VRAM_BEGIN..=VRAM_END => {
                gpu.write_vram_abs(addr, value);
            }

            OAM_BEGIN..=OAM_END => {
                let off = (addr - OAM_BEGIN) as usize;
                gpu.write_oam(off, value);
            }

            IO_LCDC => gpu.set_lcdc(value),

            IO_SCY => {
                let _old = gpu.get_scy();
                gpu.set_scy(value);
                #[cfg(feature = "trace_ppu")]
                eprintln!("[CPU] SCY <= {:>3} at ly={}, mode={}", value, gpu.ly(), gpu.mode_code());
            }

            IO_SCX => {
                let _old = gpu.get_scx();
                gpu.set_scx(value);
                #[cfg(feature = "trace_ppu")]
                eprintln!("[CPU] SCX <= {:>3} at ly={}, mode={}", value, gpu.ly(), gpu.mode_code());
            }

            IO_BGP => gpu.set_bgp(value),
            IO_OBP0 => gpu.set_obp0(value),
            IO_OBP1 => gpu.set_obp1(value),

            IO_WY => {
                let _old = gpu.get_wy();
                gpu.set_wy(value);
                #[cfg(feature = "trace_ppu")]
                eprintln!("[CPU] WY  <= {:>3} at ly={}, mode={}", value, gpu.ly(), gpu.mode_code());
            }

            IO_WX => {
                let _old = gpu.get_wx();
                gpu.set_wx(value);
                #[cfg(feature = "trace_ppu")]
                eprintln!("[CPU] WX  <= {:>3} at ly={}, mode={}", value, gpu.ly(), gpu.mode_code());
            }

            IO_STAT => {
                gpu.write_stat(value);
                #[cfg(feature = "trace_ppu")]
                eprintln!(
                    "[CPU] STAT <= {:02X} at ly={}, mode={}",
                    value,
                    gpu.ly(),
                    gpu.mode_code()
                );
            }

            IO_LYC => {
                gpu.write_lyc(value);
                #[cfg(feature = "trace_ppu")]
                eprintln!("[CPU] LYC <= {:>3} at ly={}, mode={}", value, gpu.ly(), gpu.mode_code());
            }

            // FF46 — start timed OAM DMA handled by GPU
            IO_DMA => {
                gpu.write_ff46_start_dma(value);
                // Optional: mirror write for realism
                self.memory[addr as usize] = value;
            }

            IO_BOOT => {
                self.memory[addr as usize] = value;
                if (value & 0x01) != 0 {
                    self.boot_enabled = false;
                }
            }

            0xff04..=0xff07 => timer.write_io(addr, value),

            _ => {
                self.memory[addr as usize] = value;
            }
        }
    }
}
