// src/mmu.rs
use crate::gpu::GPU;
use crate::timer::Timer;
use crate::cart::Cartridge;
use crate::input::joypad::Joypad;
use crate::apu::Apu;
#[cfg(feature = "trace_ppu")]
use crate::trace::{ self, Category as TraceCategory };

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

pub const IO_JOYPAD: u16 = 0xff00;

// STAT/LY/LYC
const IO_STAT: u16 = 0xff41;
const IO_LY: u16 = 0xff44;
const IO_LYC: u16 = 0xff45;

// OAM DMA
const IO_DMA: u16 = 0xff46;

// IF (Interrupt Flag) and FF50 (boot)
const IO_IF: u16 = 0xff0f;
const IO_IE: u16 = 0xffff;
const IO_BOOT: u16 = 0xff50;

pub struct MMU {
    memory: Box<[u8; 0x10000]>,
    boot_rom: Option<Vec<u8>>,
    boot_enabled: bool,
    pub(crate) cart: Cartridge,
    pub joypad: Joypad,
    /// Bytes sent over the serial port, in order. Blargg's multi-ROM suites
    /// report their result here rather than through the $A000 protocol, so the
    /// harness watches this for a terminator.
    pub serial_out: Vec<u8>,
}

impl MMU {
    #[inline]
    fn trace_io_write(_addr: u16, _value: u8, _gpu: &GPU) {
        #[cfg(feature = "trace_ppu")]
        {
            let critical = matches!(
                _addr,
                IO_LCDC | IO_STAT | IO_LY | IO_LYC | IO_JOYPAD | IO_IF | IO_IE
            );
            if critical && trace::enabled(TraceCategory::MmuIo) {
                eprintln!(
                    "event=mmu_io step={} addr={:04X} value={:02X} ly={} mode={}",
                    trace::step(),
                    _addr,
                    _value,
                    _gpu.ly(),
                    _gpu.mode_code()
                );
            }
        }
    }

    pub fn new() -> Self {
        Self {
            memory: Box::new([0u8; 0x10000]),
            boot_rom: None,
            boot_enabled: false,
            cart: Cartridge::empty(),
            joypad: Joypad::new(),
            serial_out: Vec::new(),
        }
    }

    #[inline]
    pub fn set_if_bits(&mut self, mask: u8) {
        let idx = IO_IF as usize;
        self.memory[idx] |= mask; // OR-in (do not overwrite)
    }

    #[inline]
    pub fn clear_if_bits(&mut self, mask: u8) {
        let idx = IO_IF as usize;
        self.memory[idx] &= !mask;
    }

    #[inline]
    pub fn if_reg(&self) -> u8 {
        self.memory[IO_IF as usize]
    }

    #[inline]
    pub fn poll_joypad_irq(&mut self) -> bool {
        self.joypad.update_interrupt().is_some()
    }

    pub fn load_rom(&mut self, rom: &[u8]) {
        self.try_load_rom(rom).expect("invalid/unsupported cartridge ROM");
    }

    /// Like `load_rom`, but returns an error for a bad or unsupported cart.
    pub fn try_load_rom(&mut self, rom: &[u8]) -> anyhow::Result<()> {
        self.cart = Cartridge::from_bytes(rom.to_vec())?;
        Ok(())
    }

    pub fn load_boot_rom(&mut self, bytes: Vec<u8>) {
        assert!(bytes.len() == 0x100, "DMG boot ROM must be 256 bytes");
        self.boot_rom = Some(bytes);
        self.boot_enabled = true;
    }

    #[inline]
    fn read8_impl(
        &self,
        addr: u16,
        gpu: &GPU,
        timer: &Timer,
        apu: &Apu,
        cpu_bus_rules: bool
    ) -> u8 {
        // DMG bus gating during OAM DMA: CPU can only access HRAM ($FF80..$FFFE)
        if cpu_bus_rules && gpu.dma_in_progress() && !(0xff80..=0xfffe).contains(&addr) {
            return 0xff;
        }

        // Boot ROM overlay 0000..00FF
        if self.boot_enabled && addr <= 0x00ff {
            if let Some(br) = &self.boot_rom {
                return br[addr as usize];
            }
        }

        match addr {
            // Only bits 0-4 of IF exist; the top three always read as 1.
            // IE ($FFFF) is a full 8-bit register on DMG and is not masked.
            IO_IF => self.memory[IO_IF as usize] | 0xe0,
            0x0000..=0x7fff => self.cart.read_rom(addr),
            0xa000..=0xbfff => self.cart.read_ram(addr),
            IO_JOYPAD => self.joypad.read(),

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

            0xff10..=0xff3f => apu.read(addr),

            _ => {
                // Unmapped I/O regions read as 0xFF on DMG
                match addr {
                    | 0xff03
                    | 0xff08..=0xff0e
                    | 0xff15
                    | 0xff1f
                    | 0xff27..=0xff2f
                    | 0xff4c..=0xff7f => 0xff,
                    _ => self.memory[addr as usize],
                }
            }
        }
    }

    /// CPU read; forwards to GPU/Timer or flat memory.
    pub fn read8(&self, addr: u16, gpu: &GPU, timer: &Timer, apu: &Apu) -> u8 {
        self.read8_impl(addr, gpu, timer, apu, true)
    }

    /// Internal DMA read path: bypasses CPU bus gating rules.
    pub fn read8_dma(&self, addr: u16, gpu: &GPU, timer: &Timer, apu: &Apu) -> u8 {
        self.read8_impl(addr, gpu, timer, apu, false)
    }

    /// CPU write; forwards to GPU/Timer or flat memory.
    pub fn write8(
        &mut self,
        addr: u16,
        value: u8,
        gpu: &mut GPU,
        timer: &mut Timer,
        apu: &mut Apu
    ) {
        // DMG bus gating during OAM DMA: CPU writes outside HRAM are ignored
        if gpu.dma_in_progress() && !(0xff80..=0xfffe).contains(&addr) {
            return;
        }

        match addr {
            0x0000..=0x7fff | 0xa000..=0xbfff => self.cart.write(addr, value),

            VRAM_BEGIN..=VRAM_END => gpu.write_vram_abs(addr, value),

            OAM_BEGIN..=OAM_END => gpu.write_oam((addr - OAM_BEGIN) as usize, value),

            IO_JOYPAD => {
                Self::trace_io_write(addr, value, gpu);
                self.joypad.write(value);
            }

            | IO_LCDC
            | IO_STAT
            | IO_LY
            | IO_LYC
            | IO_DMA
            | IO_SCY
            | IO_SCX
            | IO_BGP
            | IO_OBP0
            | IO_OBP1
            | IO_WY
            | IO_WX => {
                self.write_gpu_reg(addr, value, gpu);
            }

            IO_BOOT => {
                self.memory[addr as usize] = value;
                if (value & 0x01) != 0 {
                    self.boot_enabled = false;
                }
            }

            IO_IF => {
                Self::trace_io_write(addr, value, gpu);
                // Only the low 5 bits are storable; reads OR in 0xE0.
                self.memory[addr as usize] = value & 0x1f;
            }

            IO_IE => {
                Self::trace_io_write(addr, value, gpu);
                self.memory[addr as usize] = value;
            }

            // Serial transfer control
            0xff02 => {
                self.memory[addr as usize] = value & 0x7f;
                if (value & 0x81) == 0x81 {
                    let sb = self.memory[0xff01];
                    print!("{}", sb as char);
                    self.serial_out.push(sb);
                    self.set_if_bits(0x08);
                }
            }

            0xff04..=0xff07 => {
                if addr == 0xff04 {
                    apu.div_reset();
                }
                timer.write_io(addr, value);
            }

            0xff10..=0xff3f => apu.write(addr, value),

            _ => {
                self.memory[addr as usize] = value;
            }
        }
    }

    /// Handle writes to GPU I/O registers (FF40–FF4B).
    /// Sets STAT IRQ (IF bit 1) when certain registers trigger it.
    fn write_gpu_reg(&mut self, addr: u16, value: u8, gpu: &mut GPU) {
        Self::trace_io_write(addr, value, gpu);
        match addr {
            IO_LCDC => {
                if gpu.set_lcdc(value) {
                    self.set_if_bits(0x02);
                }
            }
            IO_STAT => {
                if gpu.write_stat(value) {
                    self.set_if_bits(0x02);
                }
                #[cfg(feature = "trace_ppu")]
                if !trace::structured_enabled() {
                    eprintln!(
                        "[CPU] STAT <= {:02X} at ly={}, mode={}",
                        value,
                        gpu.ly(),
                        gpu.mode_code()
                    );
                }
            }
            IO_LYC => {
                if gpu.write_lyc(value) {
                    self.set_if_bits(0x02);
                }
                #[cfg(feature = "trace_ppu")]
                if !trace::structured_enabled() {
                    eprintln!(
                        "[CPU] LYC <= {:>3} at ly={}, mode={}",
                        value,
                        gpu.ly(),
                        gpu.mode_code()
                    );
                }
            }
            IO_LY => {
                // DMG behaviour: writing to LY resets it to 0.
                if gpu.write_ly(value) {
                    self.set_if_bits(0x02);
                }
            }
            IO_SCY => {
                gpu.set_scy(value);
                #[cfg(feature = "trace_ppu")]
                if !trace::structured_enabled() {
                    eprintln!(
                        "[CPU] SCY <= {:>3} at ly={}, mode={}",
                        value,
                        gpu.ly(),
                        gpu.mode_code()
                    );
                }
            }
            IO_SCX => {
                gpu.set_scx(value);
                #[cfg(feature = "trace_ppu")]
                if !trace::structured_enabled() {
                    eprintln!(
                        "[CPU] SCX <= {:>3} at ly={}, mode={}",
                        value,
                        gpu.ly(),
                        gpu.mode_code()
                    );
                }
            }
            IO_WY => {
                gpu.set_wy(value);
                #[cfg(feature = "trace_ppu")]
                if !trace::structured_enabled() {
                    eprintln!(
                        "[CPU] WY  <= {:>3} at ly={}, mode={}",
                        value,
                        gpu.ly(),
                        gpu.mode_code()
                    );
                }
            }
            IO_WX => {
                gpu.set_wx(value);
                #[cfg(feature = "trace_ppu")]
                if !trace::structured_enabled() {
                    eprintln!(
                        "[CPU] WX  <= {:>3} at ly={}, mode={}",
                        value,
                        gpu.ly(),
                        gpu.mode_code()
                    );
                }
            }
            IO_BGP => gpu.set_bgp(value),
            IO_OBP0 => gpu.set_obp0(value),
            IO_OBP1 => gpu.set_obp1(value),
            IO_DMA => {
                gpu.write_ff46_start_dma(value);
                self.memory[addr as usize] = value; // mirror for realism
            }
            _ => {}
        }
    }
}
