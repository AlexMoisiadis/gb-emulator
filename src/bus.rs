// src/bus.rs
use crate::gpu::{ GPU, GpuEvents, DebugOverlayConfig, DmaRead, OamCorruptionKind };
use crate::timer::Timer;
use crate::apu::Apu;
use crate::mmu::MMU;
#[cfg(feature = "trace_ppu")]
use crate::trace::{ self, Category as TraceCategory };

#[derive(Debug, Clone, Copy)]
pub enum InterruptSource {
    VBlank,
    LcdStat,
    Timer,
    Serial,
    Joypad,
}

impl InterruptSource {
    #[inline]
    pub fn if_mask(self) -> u8 {
        match self {
            InterruptSource::VBlank => 0x01,
            InterruptSource::LcdStat => 0x02,
            InterruptSource::Timer => 0x04,
            InterruptSource::Serial => 0x08,
            InterruptSource::Joypad => 0x10,
        }
    }
}

/// System memory bus tying MMU, GPU and Timer.
pub struct MemoryBus {
    pub mmu: MMU,
    pub gpu: GPU,
    timer: Timer,
    /// Generated samples collect in `apu.sample_buffer`; the frontend must
    /// drain or clear them every frame (`Apu::drain_samples`).
    pub apu: Apu,
    /// T-cycles elapsed within the current instruction (reset at each step()).
    /// Used to compute the correct OAM row for per-M-cycle-accurate corruption.
    pub instruction_tcycles: u32,
    /// How much of the current instruction has already been ticked into the APU
    /// by a lazy catch-up. Reconciled against the authoritative total in
    /// `service_apu()` at instruction end.
    apu_consumed: u32,
}

impl MemoryBus {
    pub fn new() -> Self {
        Self {
            mmu: MMU::new(),
            gpu: GPU::new(),
            timer: Timer::new(),
            // Frontends with a real device call apu.set_sample_rate().
            apu: Apu::new(44100.0),
            instruction_tcycles: 0,
            apu_consumed: 0,
        }
    }

    #[inline]
    pub fn service_gpu(&mut self, tcycles: u32) {
        let bus_ptr: *const MemoryBus = self as *const MemoryBus;
        let mut dma = DmaProxy { bus: bus_ptr };
        let events: GpuEvents = self.gpu.tick(tcycles, &mut dma);
        self.apply_gpu_events(events);
    }

    /// Commit a deferred LCD enable: enter mode 2 at the scanline boundary and
    /// raise a STAT interrupt if the mode-2 STAT source is enabled.
    /// Called from cpu::step() after all service_gpu calls for the enabling instruction.
    #[inline]
    pub fn commit_lcd_enable(&mut self) {
        if self.gpu.commit_lcd_enable() {
            self.raise_interrupt(InterruptSource::LcdStat);
        }
    }

    /// Advance the APU to the current M-cycle boundary within the instruction.
    /// Called before any observation that can see sub-instruction APU state:
    /// APU register / wave RAM access, and the DIV write that resets the FS.
    #[inline]
    pub fn sync_apu(&mut self) {
        let owed = self.instruction_tcycles.saturating_sub(self.apu_consumed);
        if owed > 0 {
            self.apu.tick(owed);
            self.apu_consumed = self.instruction_tcycles;
        }
    }

    #[inline]
    // In service loop (alongside service_gpu / service_timer):
    pub fn service_apu(&mut self, tcycles: u32) {
        // Only the cycles not already applied by a mid-instruction catch-up.
        self.apu.tick(tcycles.saturating_sub(self.apu_consumed));
        self.apu_consumed = 0;
    }

    pub fn apply_gpu_events(&mut self, events: GpuEvents) {
        let _old_if = self.read_if();
        if events.if_from_vblank || events.if_set.contains(crate::gpu::IfBits::VBLANK) {
            self.raise_interrupt(InterruptSource::VBlank);
        }
        if events.if_from_stat || events.if_set.contains(crate::gpu::IfBits::LCD_STAT) {
            self.raise_interrupt(InterruptSource::LcdStat);
        }
        #[cfg(feature = "trace_ppu")]
        {
            let new_if = self.read_if();
            if new_if != _old_if && !trace::structured_enabled() {
                eprintln!(
                    "[BUS] IF now={:02X} IE={:02X} (ly={}, mode={})",
                    new_if,
                    self.read_byte(0xffff),
                    self.gpu.ly(),
                    self.gpu.mode_code()
                );
            }
        }
        if events.frame_became_ready {
            // Front-end can observe via gpu.take_frame_ready()
        }
    }

    pub fn press_button(&mut self, button: crate::input::joypad::Button) {
        self.mmu.joypad.press(button);
    }
    pub fn release_button(&mut self, button: crate::input::joypad::Button) {
        self.mmu.joypad.release(button);
    }

    #[inline]
    pub fn service_timer(&mut self, tcycles: u32) {
        if tcycles > 0 && self.timer.tick(tcycles) {
            self.raise_interrupt(InterruptSource::Timer);
        }
    }

    #[inline]
    pub fn service_input(&mut self) {
        if self.mmu.poll_joypad_irq() {
            self.raise_interrupt(InterruptSource::Joypad);
        }
    }

    #[inline]
    pub fn raise_interrupt(&mut self, source: InterruptSource) {
        self.mmu.set_if_bits(source.if_mask());
        #[cfg(feature = "trace_ppu")]
        if trace::enabled(TraceCategory::CpuIrq) {
            let src = match source {
                InterruptSource::VBlank => "vblank",
                InterruptSource::LcdStat => "lcd_stat",
                InterruptSource::Timer => "timer",
                InterruptSource::Serial => "serial",
                InterruptSource::Joypad => "joypad",
            };
            eprintln!(
                "event=irq_raise step={} src={} if={:02X} frame={} ly={} mode={}",
                trace::step(),
                src,
                self.read_if(),
                self.gpu.frame_index(),
                self.gpu.ly(),
                self.gpu.mode_code()
            );
        }
    }

    #[inline]
    pub fn clear_interrupt(&mut self, source: InterruptSource) {
        self.mmu.clear_if_bits(source.if_mask());
    }

    #[inline]
    pub fn read_if(&self) -> u8 {
        self.mmu.if_reg()
    }

    pub fn set_gpu_debug_config(&mut self, cfg: DebugOverlayConfig) {
        self.gpu.set_debug_config(cfg);
    }

    pub fn load_rom(&mut self, rom: &[u8]) {
        self.mmu.load_rom(rom);
    }
    pub fn load_boot_rom(&mut self, bytes: Vec<u8>) {
        self.mmu.load_boot_rom(bytes);
    }

    /// Compute the OAM row currently being scanned.
    /// With per-M-cycle GPU stepping, mode_dot already reflects the current
    /// M-cycle boundary so no instruction_tcycles offset is needed.
    #[inline]
    fn oam_current_row(&self) -> usize {
        self.gpu.oam2_row()
    }

    #[inline]
    pub fn read_byte(&mut self, address: u16) -> u8 {
        // OAM corruption bug: any read from $FE00–$FEFF while PPU is in mode 2
        // applies Read Corruption to the currently-scanned OAM row.
        // Mode 3 still silently returns 0xFF (no corruption).
        if matches!(address, 0xFE00..=0xFEFF) {
            if self.gpu.is_oam2_active() {
                let row = self.oam_current_row();
                self.gpu.corrupt_oam_row(row, OamCorruptionKind::Read);
                return 0xFF;
            } else if self.gpu.is_xfer3_active() {
                return 0xFF;
            }
        }
        if matches!(address, 0xFF10..=0xFF3F) {
            self.sync_apu();
        }
        self.mmu.read8(address, &self.gpu, &self.timer, &self.apu)
    }

    #[inline]
    pub fn write_byte(&mut self, address: u16, value: u8) {
        // OAM corruption bug: any write to $FE00–$FEFF while PPU is in mode 2
        // applies Write Corruption to the currently-scanned OAM row (no actual write).
        // Mode 3 silently discards the write (no corruption).
        if matches!(address, 0xFE00..=0xFEFF) {
            if self.gpu.is_oam2_active() {
                let row = self.oam_current_row();
                self.gpu.corrupt_oam_row(row, OamCorruptionKind::Write);
                return;
            } else if self.gpu.is_xfer3_active() {
                return;
            }
        }
        // 0xFF04: the DIV write resets the frame-sequencer timer, so pending
        // cycles must land at the old FS position first.
        if matches!(address, 0xFF10..=0xFF3F | 0xFF04) {
            self.sync_apu();
        }
        self.mmu.write8(address, value, &mut self.gpu, &mut self.timer, &mut self.apu);
    }

    /// For LD A,[HL+], LD A,[HL-], and instruction fetch from OAM range:
    /// The memory read and the PC/HL IDU increment happen in the same M-cycle,
    /// producing the combined Read-During-Inc/Dec corruption pattern.
    #[inline]
    pub fn oam_read_during_inc(&mut self, address: u16) -> u8 {
        if matches!(address, 0xFE00..=0xFEFF) && self.gpu.is_oam2_active() {
            let row = self.oam_current_row();
            self.gpu.corrupt_oam_row(row, OamCorruptionKind::ReadDuringIncDec);
            return 0xFF;
        }
        // Not in OAM range or not in mode 2 — normal read.
        // Re-use the normal read_byte path (mode 3 blocking included).
        self.read_byte(address)
    }

    /// For LD [HL+],A and LD [HL-],A when HL is in OAM range:
    /// Write and IDU increment are in the same M-cycle → single Write Corruption.
    #[inline]
    pub fn oam_write_during_inc(&mut self, address: u16, value: u8) {
        if matches!(address, 0xFE00..=0xFEFF) && self.gpu.is_oam2_active() {
            let row = self.oam_current_row();
            self.gpu.corrupt_oam_row(row, OamCorruptionKind::Write);
            return;
        }
        self.write_byte(address, value);
    }

    /// IDU-only corruption: 16-bit INC/DEC where the register value is in
    /// $FE00–$FEFF during mode 2. No actual memory access; the IDU drives
    /// the address bus and causes a Write Corruption.
    #[inline]
    pub fn idu_oam_corrupt(&mut self, reg_val: u16) {
        if matches!(reg_val, 0xFE00..=0xFEFF) && self.gpu.is_oam2_active() {
            let row = self.oam_current_row();
            self.gpu.corrupt_oam_row(row, OamCorruptionKind::Write);
        }
    }
}

/// Small adapter implementing `DmaRead` for the GPU.
/// This allows the GPU to read the bus during OAM DMA while `&mut GPU` is held,
/// without taking additional Rust borrows that would conflict with that mutable borrow.
/// Safety: `read8` only performs read-only operations; we dereference `bus` immutably.
struct DmaProxy {
    bus: *const MemoryBus,
}

impl DmaRead for DmaProxy {
    #[inline]
    fn read8(&mut self, addr: u16) -> u8 {
        unsafe {
            (*self.bus).mmu.read8_dma(addr, &(*self.bus).gpu, &(*self.bus).timer, &(*self.bus).apu)
        }
    }
}
