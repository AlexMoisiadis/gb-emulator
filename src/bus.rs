// src/bus.rs
use crate::gpu::GPU;
use crate::timer::Timer;
use crate::mmu::MMU;
use crate::gpu::{ DebugOverlayConfig, LCD_HEIGHT, LCD_WIDTH };

// I/O registers
const IO_STAT: u16 = 0xff41; // LCD STAT
const IO_LY: u16 = 0xff44; // current scanline
const IO_LYC: u16 = 0xff45; // LY compare

// IF bits
const IF_VBLANK: u8 = 0b0000_0001; // bit 0
const IF_LCD_STAT: u8 = 0b0000_0010; // bit 1

// STAT bits (FF41)
const STAT_INT_MODE0: u8 = 1 << 3; // HBlank interrupt enable
const STAT_INT_MODE1: u8 = 1 << 4; // VBlank interrupt enable
const STAT_INT_MODE2: u8 = 1 << 5; // OAM interrupt enable
const STAT_INT_LYC: u8 = 1 << 6; // LYC=LY interrupt enable
const STAT_COINC: u8 = 1 << 2; // coincidence flag (read)

pub struct MemoryBus {
    pub mmu: MMU,
    pub gpu: GPU,
    timer: Timer,
}

impl MemoryBus {
    pub fn new() -> Self {
        Self {
            mmu: MMU::new(),
            gpu: GPU::new(),
            timer: Timer::new(),
        }
    }

    pub fn set_gpu_debug_config(&mut self, cfg: DebugOverlayConfig) {
        self.gpu.set_debug_config(cfg);
    }

    pub fn load_rom(&mut self, rom: &[u8]) {
        self.mmu.load_rom(rom);
    }

    // src/bus.rs (inside impl MemoryBus)
    pub fn load_boot_rom(&mut self, bytes: Vec<u8>) {
        self.mmu.load_boot_rom(bytes);
    }

    /// Scanline renderer: BG/Window first, then sprites; raises VBlank IF at LY=144.
    /// Render one full frame into `framebuffer[144][160]` (DMG shades 0..3 per pixel).
    /// Adds coarse STAT mode timing:
    ///   - Visible lines (0..143): Mode 2 -> Mode 3 -> Mode 0
    ///   - VBlank lines  (144..153): Mode 1
    /// Also performs LYC compare and raises STAT interrupts on mode entry/LYC=LY events.
    pub fn render_frame(&mut self, framebuffer: &mut [[u8; 160]; 144]) {
        // Window internal counter reset at start of frame (per Pan Docs).
        self.gpu.begin_frame(); // [4](https://aquova.net/emudev/gb/21-control-register.html)

        // Small helpers for STAT updates & interrupts ------------------------------

        // Write STAT mode bits (bits 0..1), preserving other bits.
        let set_stat_mode = |bus: &mut MemoryBus, mode: u8| {
            let stat = bus.mmu.read8(IO_STAT, &bus.gpu, &bus.timer);
            let new_stat = (stat & !0x03) | (mode & 0x03);
            bus.mmu.write8(IO_STAT, new_stat, &mut bus.gpu, &mut bus.timer);

            // If entering Mode 2/1/0, check enable bits and raise LCD STAT IF if enabled.
            let enable_bit = match mode {
                2 => STAT_INT_MODE2, // OAM
                1 => STAT_INT_MODE1, // VBlank
                0 => STAT_INT_MODE0, // HBlank
                _ => 0,
            };
            if enable_bit != 0 && (stat & enable_bit) != 0 {
                bus.mmu.set_if_bits(IF_LCD_STAT); // request LCD STAT interrupt [3](https://gbdev.io/pandocs/Scrolling.html)
            }
        };

        // Update LY and perform LYC compare; raise STAT if enabled and equal.
        let write_ly_and_lyc_check = |bus: &mut MemoryBus, ly: u8| {
            // Write LY
            bus.mmu.write8(IO_LY, ly, &mut bus.gpu, &mut bus.timer);

            // LYC compare
            let lyc = bus.mmu.read8(IO_LYC, &bus.gpu, &bus.timer);
            let stat = bus.mmu.read8(IO_STAT, &bus.gpu, &bus.timer);
            let equal = ly == lyc;

            // Set/clear coincidence flag (bit 2)
            let new_stat = if equal { stat | STAT_COINC } else { stat & !STAT_COINC };
            bus.mmu.write8(IO_STAT, new_stat, &mut bus.gpu, &mut bus.timer);

            // If enabled (STAT bit 6) and equal, raise LCD STAT IF.
            if equal && (new_stat & STAT_INT_LYC) != 0 {
                bus.mmu.set_if_bits(IF_LCD_STAT); // [2](https://viboycolor.fabini.one/en/bitacora/entries/2026-01-03__0464__fix-bg-tilemap-base-scroll.html)
            }
        };

        // ---------------- Visible scanlines: 0..=143 ------------------------------
        for ly in 0u8..LCD_HEIGHT as u8 {
            // LY update + LYC compare
            write_ly_and_lyc_check(self, ly);

            // Mode 2 (OAM scan) — entering OAM: raise STAT if enabled.
            set_stat_mode(self, 2); // [1](https://gekkio.fi/files/gb-docs/gbctr.pdf)

            // "Switch" to Mode 3 (VRAM drawing); no STAT source on entry to Mode 3.
            set_stat_mode(self, 3); // [1](https://gekkio.fi/files/gb-docs/gbctr.pdf)

            // Render BG/Window and OBJ overlay
            let mut line = [0u8; LCD_WIDTH];
            self.gpu.render_scanline(ly, &mut line);
            self.gpu.render_scanline_objs(ly, &mut line);

            self.gpu.overlay_debug_scanline(ly, &mut line);

            framebuffer[ly as usize].copy_from_slice(&line);

            // End of line -> Mode 0 (HBlank): raise STAT if enabled.
            set_stat_mode(self, 0); // [1](https://gekkio.fi/files/gb-docs/gbctr.pdf)
        }

        // ---------------- VBlank scanlines: 144..=153 -----------------------------
        // Raise VBlank IF (bit 0) once when entering VBlank (LY=144).
        self.mmu.set_if_bits(IF_VBLANK); // [3](https://gbdev.io/pandocs/Scrolling.html)

        for ly in 144u8..=153 {
            write_ly_and_lyc_check(self, ly);
            // Mode 1 for all VBlank lines; entering VBlank triggers STAT if enabled.
            set_stat_mode(self, 1); // [1](https://gekkio.fi/files/gb-docs/gbctr.pdf)
        }

        // Next frame will start at LY=0 again (outer CPU/SoC loop controls that).
    }

    #[inline]
    pub fn read_byte(&mut self, address: u16) -> u8 {
        self.mmu.read8(address, &self.gpu, &self.timer)
    }

    #[inline]
    pub fn write_byte(&mut self, address: u16, value: u8) {
        self.mmu.write8(address, value, &mut self.gpu, &mut self.timer);
        if self.gpu.take_lyc_irq_pending() {
            self.mmu.set_if_bits(0x02);
        }
    }

    pub fn tick(&mut self, cpu_cycles: u32) {
        let _overflow = self.timer.tick(cpu_cycles);
        let ev = self.gpu.tick(cpu_cycles); // T-cycles == dots on DMG, no multiply
        if !ev.if_set.is_empty() {
            self.mmu.set_if_bits(ev.if_set.bits());
        }
    }
}
