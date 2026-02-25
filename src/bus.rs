// src/bus.rs
use crate::gpu::{ GPU, GpuEvents, DebugOverlayConfig, DmaRead };
use crate::timer::Timer;
use crate::mmu::MMU;
use crate::cart::Cartridge;

/// System memory bus tying MMU, GPU and Timer.
pub struct MemoryBus {
    pub mmu: MMU,
    pub gpu: GPU,
    timer: Timer,
    cart: Cartridge,
}

impl MemoryBus {
    pub fn new() -> Self {
        Self { mmu: MMU::new(), gpu: GPU::new(), timer: Timer::new(), cart: Cartridge }
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
        if !events.if_set.is_empty() {
            let old_if = self.read_byte(0xff0f);
            let new_if = old_if | events.if_set.bits();
            if new_if != old_if {
                self.write_byte(0xff0f, new_if);
                #[cfg(feature = "trace_ppu")]
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
            (*self.bus).mmu.read8(addr, &(*self.bus).gpu, &(*self.bus).timer)
        }
    }
}
