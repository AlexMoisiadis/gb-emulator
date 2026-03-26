// src/bus.rs
use crate::gpu::{ GPU, GpuEvents, DebugOverlayConfig, DmaRead, OamCorruptionKind };
use crate::apu::output::AudioOutput;
use crate::timer::Timer;
use crate::apu::Apu;
use crate::mmu::MMU;
use ringbuf::traits::Producer;
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
    pub apu: Apu,
    audio: Option<AudioOutput>,
    pub audio_muted: bool,
    /// T-cycles elapsed within the current instruction (reset at each step()).
    /// Used to compute the correct OAM row for per-M-cycle-accurate corruption.
    pub instruction_tcycles: u32,
    /// T-cycles elapsed BEFORE the write M-cycle that enabled the LCD this step.
    /// Set to u32::MAX when no LCD enable occurred this step (sentinel = inactive).
    /// Used by service_gpu() to skip the pre-enable portion of the instruction.
    pub lcd_enable_offset: u32,
}

impl MemoryBus {
    pub fn new() -> Self {
        let audio = AudioOutput::new()
            .map_err(|e| eprintln!("[APU] audio init failed: {e}"))
            .ok();
        let sample_rate = audio
            .as_ref()
            .map(|a| a.sample_rate)
            .unwrap_or(44100.0);
        Self {
            mmu: MMU::new(),
            gpu: GPU::new(),
            timer: Timer::new(),
            apu: Apu::new(sample_rate),
            audio,
            audio_muted: false,
            instruction_tcycles: 0,
            lcd_enable_offset: u32::MAX,
        }
    }

    #[inline]
    pub fn service_gpu(&mut self, tcycles: u32) {
        // If the LCD was enabled mid-instruction this step, only advance the GPU
        // by the T-cycles during which the LCD was actually on (the write M-cycle
        // and any remaining M-cycles). The pre-enable portion is skipped.
        // Consume the offset immediately so subsequent service_gpu calls within
        // the same step (e.g. ISR dispatch) advance by their full cycle count.
        let effective = if self.lcd_enable_offset != u32::MAX {
            let offset = self.lcd_enable_offset;
            self.lcd_enable_offset = u32::MAX;
            tcycles.saturating_sub(offset)
        } else {
            tcycles
        };
        let bus_ptr: *const MemoryBus = self as *const MemoryBus;
        let mut dma = DmaProxy { bus: bus_ptr };
        let events: GpuEvents = self.gpu.tick(effective, &mut dma);
        self.apply_gpu_events(events);
    }

    pub fn toggle_mute(&mut self) -> bool {
        self.audio_muted = !self.audio_muted;
        self.audio_muted
    }

    #[inline]
    // In service loop (alongside service_gpu / service_timer):
    pub fn service_apu(&mut self, tcycles: u32) {
        self.apu.tick(tcycles);
        if let Some(audio) = &mut self.audio {
            if self.audio_muted {
                self.apu.sample_buffer.clear(); // discard, keep buffer from growing
            } else {
                for sample in self.apu.sample_buffer.drain(..) {
                    let _ = audio.producer.try_push(sample);
                }
            }
        }
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

    /// Compute the OAM row currently being scanned, accounting for T-cycles
    /// elapsed within the current instruction (per-M-cycle accuracy).
    #[inline]
    fn oam_current_row(&self) -> usize {
        self.gpu.oam_row_with_offset(self.instruction_tcycles)
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
        // Detect LCD 0→1 transition for timing correction in service_gpu().
        // Capture lcd_was_on only for LCDC writes to avoid the branch overhead
        // on every write; default true so the post-write check is a no-op otherwise.
        let lcd_was_on = address != 0xFF40 || self.gpu.lcd_enabled();
        self.mmu.write8(address, value, &mut self.gpu, &mut self.timer, &mut self.apu);
        if !lcd_was_on && self.gpu.lcd_enabled() {
            // instruction_tcycles was already incremented for the write M-cycle
            // before write_byte was called, so subtract 4 to get the T-cycles
            // elapsed BEFORE the write M-cycle (the portion when LCD was still off).
            self.lcd_enable_offset = self.instruction_tcycles.saturating_sub(4);
        }
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
