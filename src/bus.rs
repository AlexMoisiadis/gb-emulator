// src/bus.rs
use crate::gpu::{ GPU, GpuEvents, DebugOverlayConfig, DmaRead };
use crate::timer::Timer;
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
}

impl MemoryBus {
    pub fn new() -> Self {
        Self { mmu: MMU::new(), gpu: GPU::new(), timer: Timer::new() }
    }

    #[inline]
    pub fn service_gpu(&mut self, tcycles: u32) {
        // Localize the only unsafe we need during DMA, behind a tiny adapter:
        let bus_ptr: *const MemoryBus = self as *const MemoryBus;
        let mut dma = DmaProxy { bus: bus_ptr };
        let events: GpuEvents = self.gpu.tick(tcycles, &mut dma);
        self.apply_gpu_events(events);
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

    // #[inline]
    // pub fn service_timer(&mut self, tcycles: u32) {
    //     let mcycles = tcycles / 4;
    //     if mcycles > 0 && self.timer.tick(mcycles) {
    //         self.raise_interrupt(InterruptSource::Timer);
    //     }
    // }

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

    #[inline]
    pub fn read_byte(&self, address: u16) -> u8 {
        self.mmu.read8(address, &self.gpu, &self.timer)
    }

    #[inline]
    pub fn write_byte(&mut self, address: u16, value: u8) {
        self.mmu.write8(address, value, &mut self.gpu, &mut self.timer);
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
        // SAFETY: Read-only snapshot of sub-components while GPU holds &mut self.
        // MMU::read8 takes &self and &GPU/&Timer (both read-only here).
        unsafe {
            (*self.bus).mmu.read8_dma(addr, &(*self.bus).gpu, &(*self.bus).timer)
        }
    }
}
