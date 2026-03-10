// src/gpu.rs
//
// DMG PPU timing fix:
// - Consume all tcycles in GPU::tick (no early returns on IF)
// - Constant 456 dots per line (Mode3=172 nominal on DMG)
// - Single VBlank IF raise on entering LY=144
// - STAT rising-edge aggregation without stopping time

use std::array::from_fn;
use crate::trace::{ self, Category as TraceCategory };

pub const VRAM_BEGIN: u16 = 0x8000;
pub const VRAM_END: u16 = 0x9fff;
pub const VRAM_SIZE: usize = (VRAM_END as usize) - (VRAM_BEGIN as usize) + 1;

pub const TILE_COUNT: usize = 384;
const TILE_DATA_LEN: usize = 0x1800; // 6 KiB

pub const LCD_WIDTH: usize = 160;
pub const LCD_HEIGHT: usize = 144;
const LINE_DOTS: u16 = 456;
const MODE2_OAM_DOTS: u16 = 80;
const MODE3_BASE_DOTS: u16 = 172;

type Tile = [[TilePixelValue; 8]; 8];

bitflags::bitflags! {
    pub struct IfBits: u8 {
        const VBLANK   = 0b0000_0001;
        const LCD_STAT = 0b0000_0010;
    }
}

pub struct GpuEvents {
    pub if_set: IfBits,
    pub if_from_vblank: bool,
    pub if_from_stat: bool,
    pub frame_became_ready: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct FrameTelemetry {
    pub frame_index: u64,
    pub ly_wraps: u64,
    pub mode_switches: [u32; 4],
    pub stat_irq_raises: u32,
    pub stat_m0_edges: u32,
    pub stat_m1_edges: u32,
    pub stat_m2_edges: u32,
    pub stat_lyc_edges: u32,
    pub vblank_irq_raises: u32,
    pub frame_ready_ly: u8,
    pub frame_ready_mode: u8,
}

#[derive(Copy, Clone, Debug)]
enum PpuMode {
    HBlank0,
    VBlank1,
    Oam2,
    Xfer3,
}

impl PpuMode {
    #[inline]
    fn stat_bits(self) -> u8 {
        match self {
            PpuMode::HBlank0 => 0,
            PpuMode::VBlank1 => 1,
            PpuMode::Oam2 => 2,
            PpuMode::Xfer3 => 3,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TilePixelValue {
    Zero,
    One,
    Two,
    Three,
}

#[inline]
const fn empty_tile() -> Tile {
    [[TilePixelValue::Zero; 8]; 8]
}

pub trait DmaRead {
    fn read8(&mut self, addr: u16) -> u8;
}

/// All state for the OAM DMA transfer (FF46).
struct DmaState {
    active: bool,
    src_high: u8, // high byte of source address (source = src_high << 8)
    byte_idx: u16, // next byte to copy (0..160)
    mcycles_left: u16,
    start_delay_tcycles: u8, // T-cycle delay before first copy tick
    tcycle_phase: u8,
}

pub struct GPU {
    // VRAM + tile cache
    vram: [u8; VRAM_SIZE],
    tile_set: [Tile; TILE_COUNT],

    // Registers (DMG subset)
    lcdc: u8,
    scy: u8,
    scx: u8,
    wy: u8,
    wx: u8,
    bgp: u8,

    // OAM and OBJ palettes
    oam: [u8; 160],
    obp0: u8,
    obp1: u8,

    // Per-scanline BG raw indices
    bg_idx_line: [u8; LCD_WIDTH],

    // Window internal line counter
    window_line_counter: u8,

    // Debug overlay
    debug: DebugOverlayConfig,

    // PPU state
    mode: PpuMode,
    mode_dot: u16, // dot position within current mode (0-based)
    ly: u8,
    frame_ready: bool,
    framebuf: [[u8; LCD_WIDTH]; LCD_HEIGHT],
    stat: u8,
    lyc: u8,

    // OAM DMA (FF46)
    dma: DmaState,

    // For Mode 3 length (DMG nominal, kept constant here)
    sprites_on_line_count: u8,
    stat_irq_line_prev: bool,
    /// Raise STAT IRQ one dot before the actual HBlank transition (DMG quirk).
    hblank_irq_early: bool,

    // Window latches
    wy_triggered: bool, // true when window Y condition was met this frame
    wx_latched: u8, // latched WX value for the current scanline
    frame_index: u64,
    ly_wraps: u64,
    mode_switches: [u32; 4],
    stat_irq_raises: u32,
    stat_m0_edges: u32,
    stat_m1_edges: u32,
    stat_m2_edges: u32,
    stat_lyc_edges: u32,
    vblank_irq_raises: u32,
    last_frame_telemetry: Option<FrameTelemetry>,
    mode3_len_latched: u16,
    hblank_len_latched: u16,
}

#[derive(Copy, Clone, Debug)]
pub struct DebugOverlayConfig {
    pub show_bg_tile_grid: bool,
    pub show_window_bounds: bool,
    pub show_bg_axes: bool,
    pub show_sprite_boxes: bool,
    pub shade_grid: u8,
    pub shade_window: u8,
    pub shade_axes: u8,
    pub shade_sprite_box: u8,
}

impl Default for DebugOverlayConfig {
    fn default() -> Self {
        Self {
            show_bg_tile_grid: false,
            show_window_bounds: false,
            show_bg_axes: false,
            show_sprite_boxes: false,
            shade_grid: 3,
            shade_window: 2,
            shade_axes: 1,
            shade_sprite_box: 3,
        }
    }
}

impl DebugOverlayConfig {
    #[inline]
    pub fn any_enabled(&self) -> bool {
        self.show_bg_tile_grid ||
            self.show_window_bounds ||
            self.show_bg_axes ||
            self.show_sprite_boxes
    }
}

impl GPU {
    pub fn new() -> Self {
        Self {
            vram: [0; VRAM_SIZE],
            tile_set: from_fn(|_| empty_tile()),
            lcdc: 0x91,
            scy: 0,
            scx: 0,
            wy: 0,
            wx: 0,
            bgp: 0xfc,
            oam: [0; 160],
            obp0: 0xff,
            obp1: 0xff,
            bg_idx_line: [0; LCD_WIDTH],
            window_line_counter: 0,
            debug: DebugOverlayConfig::default(),
            mode: PpuMode::Oam2,
            mode_dot: 0,
            ly: 0,
            frame_ready: false,
            framebuf: [[0; LCD_WIDTH]; LCD_HEIGHT],
            stat: 0,
            lyc: 0,
            dma: DmaState {
                active: false,
                src_high: 0,
                byte_idx: 0,
                mcycles_left: 0,
                start_delay_tcycles: 0,
                tcycle_phase: 0,
            },
            sprites_on_line_count: 0,
            stat_irq_line_prev: false,
            hblank_irq_early: false,
            wy_triggered: false,
            wx_latched: 0,
            frame_index: 0,
            ly_wraps: 0,
            mode_switches: [0; 4],
            stat_irq_raises: 0,
            stat_m0_edges: 0,
            stat_m1_edges: 0,
            stat_m2_edges: 0,
            stat_lyc_edges: 0,
            vblank_irq_raises: 0,
            last_frame_telemetry: None,
            mode3_len_latched: MODE3_BASE_DOTS,
            hblank_len_latched: LINE_DOTS - MODE2_OAM_DOTS - MODE3_BASE_DOTS,
        }
    }

    #[inline]
    fn stat_source_flags_now(&self) -> (bool, bool, bool, bool) {
        if !self.lcd_enabled() {
            return (false, false, false, false);
        }
        let lyc_src = (self.stat & (1 << 6)) != 0 && self.ly == self.lyc;
        let m0_src =
            (self.stat & (1 << 3)) != 0 &&
            (matches!(self.mode, PpuMode::HBlank0) || self.hblank_irq_early);
        let m1_src = (self.stat & (1 << 4)) != 0 && matches!(self.mode, PpuMode::VBlank1);
        let m2_src = (self.stat & (1 << 5)) != 0 && matches!(self.mode, PpuMode::Oam2);
        (m0_src, m1_src, m2_src, lyc_src)
    }

    #[inline]
    fn stat_line_active_now(&self) -> bool {
        let (m0, m1, m2, lyc) = self.stat_source_flags_now();
        m0 || m1 || m2 || lyc
    }

    #[inline]
    fn drive_stat_line_edge(&mut self) -> bool {
        let (src_m0, src_m1, src_m2, src_lyc) = self.stat_source_flags_now();
        let now = self.stat_line_active_now();
        let prev = self.stat_irq_line_prev;
        let raised = now && !prev;
        let changed = now != prev;
        self.stat_irq_line_prev = now;
        if raised {
            self.stat_irq_raises = self.stat_irq_raises.saturating_add(1);
            if src_m0 {
                self.stat_m0_edges = self.stat_m0_edges.saturating_add(1);
            }
            if src_m1 {
                self.stat_m1_edges = self.stat_m1_edges.saturating_add(1);
            }
            if src_m2 {
                self.stat_m2_edges = self.stat_m2_edges.saturating_add(1);
            }
            if src_lyc {
                self.stat_lyc_edges = self.stat_lyc_edges.saturating_add(1);
            }
        }
        if trace::enabled(TraceCategory::GpuStat) && changed {
            eprintln!(
                "event=gpu_stat step={} frame={} ly={} mode={} line_now={} edge={} src_m0={} src_m1={} src_m2={} src_lyc={}",
                trace::step(),
                self.frame_index,
                self.ly,
                self.mode_code(),
                now as u8,
                raised as u8,
                src_m0 as u8,
                src_m1 as u8,
                src_m2 as u8,
                src_lyc as u8
            );
        }
        raised
    }

    #[inline]
    fn maybe_raise_stat_irq(&mut self, events: &mut GpuEvents) {
        if self.drive_stat_line_edge() {
            events.if_set |= IfBits::LCD_STAT;
            events.if_from_stat = true;
        }
    }

    #[inline]
    fn lcd_enabled(&self) -> bool {
        (self.lcdc & 0x80) != 0
    }

    pub fn begin_frame(&mut self) {
        self.window_line_counter = 0;
        self.wy_triggered = false;
    }

    // --------- register-ish API ----------
    pub fn write_lyc(&mut self, v: u8) -> bool {
        self.lyc = v;
        self.update_ly_and_lyc();
        self.hblank_irq_early = false;
        self.drive_stat_line_edge()
    }
    #[inline]
    pub fn ly(&self) -> u8 {
        self.ly
    }
    #[inline]
    pub fn write_stat(&mut self, v: u8) -> bool {
        self.stat = (self.stat & 0x07) | (v & 0x78);
        self.hblank_irq_early = false;
        self.drive_stat_line_edge()
    }
    #[inline]
    pub fn read_stat(&self) -> u8 {
        if !self.lcd_enabled() {
            // When LCD is disabled, mode/LYC state is forced idle.
            return 0x80 | (self.stat & 0x78);
        }
        let coinc = if self.ly == self.lyc { 1 << 2 } else { 0 };
        (self.stat & !(1 << 2)) | coinc | 0x80
    }
    #[inline]
    pub fn read_lyc(&self) -> u8 {
        self.lyc
    }
    #[inline]
    pub fn frame_is_ready(&self) -> bool {
        self.frame_ready
    }
    #[inline]
    pub fn take_frame_ready(&mut self) -> bool {
        let r = self.frame_ready;
        self.frame_ready = false;
        r
    }
    #[inline]
    pub fn copy_frame(&self, out: &mut [[u8; LCD_WIDTH]; LCD_HEIGHT]) {
        *out = self.framebuf;
    }

    #[inline]
    pub fn take_last_frame_telemetry(&mut self) -> Option<FrameTelemetry> {
        self.last_frame_telemetry.take()
    }

    // OBJ palettes
    #[inline]
    pub fn set_obp0(&mut self, v: u8) {
        self.obp0 = v;
    }
    #[inline]
    pub fn set_obp1(&mut self, v: u8) {
        self.obp1 = v;
    }
    #[inline]
    pub fn get_obp0(&self) -> u8 {
        self.obp0
    }
    #[inline]
    pub fn get_obp1(&self) -> u8 {
        self.obp1
    }

    // OAM access
    #[inline]
    pub fn read_oam(&self, i: usize) -> u8 {
        if self.lcd_enabled() && matches!(self.mode, PpuMode::Oam2 | PpuMode::Xfer3) {
            return 0xff;
        }
        self.oam.get(i).copied().unwrap_or(0)
    }
    #[inline]
    pub fn write_oam(&mut self, i: usize, v: u8) {
        if self.lcd_enabled() && matches!(self.mode, PpuMode::Oam2 | PpuMode::Xfer3) {
            return;
        }
        if i < self.oam.len() {
            self.oam[i] = v;
        }
    }

    // LCDC & scroll
    #[inline]
    pub fn set_lcdc(&mut self, v: u8) -> bool {
        let old_lcdc = self.lcdc;
        let was_on = self.lcd_enabled();
        let now_on = (v & 0x80) != 0;
        self.lcdc = v;
        let mut stat_edge = false;

        if trace::enabled(TraceCategory::GpuLcdc) && old_lcdc != v {
            eprintln!(
                "event=gpu_lcdc step={} ly={} from={:02X} to={:02X} lcd_on={}",
                trace::step(),
                self.ly,
                old_lcdc,
                v,
                now_on as u8
            );
        }

        match (was_on, now_on) {
            (true, false) => {
                // LCD OFF: LY resets and PPU idles in mode 0.
                self.mode = PpuMode::HBlank0;
                self.mode_dot = 0;
                self.mode3_len_latched = MODE3_BASE_DOTS;
                self.hblank_len_latched = LINE_DOTS - MODE2_OAM_DOTS - MODE3_BASE_DOTS;
                self.ly = 0;
                self.window_line_counter = 0;
                self.wy_triggered = false;
                self.wx_latched = self.wx;
                self.frame_ready = false;
                self.stat = (self.stat & !0x07) | PpuMode::HBlank0.stat_bits();
                self.stat_irq_line_prev = false;
                self.hblank_irq_early = false;
            }
            (false, true) => {
                // LCD ON: restart scanning from LY=0 in mode 2.
                self.mode = PpuMode::Oam2;
                self.mode_dot = 0;
                self.latch_visible_line_timing();
                self.ly = 0;
                self.window_line_counter = 0;
                self.wy_triggered = (self.lcdc & 0x20) != 0 && self.ly == self.wy;
                self.wx_latched = self.wx;
                self.stat = (self.stat & !0x03) | PpuMode::Oam2.stat_bits();
                if self.ly == self.lyc {
                    self.stat |= 1 << 2;
                } else {
                    self.stat &= !(1 << 2);
                }
                self.hblank_irq_early = false;
                stat_edge = self.drive_stat_line_edge();
            }
            _ => {}
        }
        stat_edge
    }
    #[inline]
    pub fn write_ly(&mut self, _v: u8) -> bool {
        self.ly = 0;
        self.update_ly_and_lyc();
        self.hblank_irq_early = false;
        self.drive_stat_line_edge()
    }
    #[inline]
    pub fn set_scy(&mut self, v: u8) {
        self.scy = v;
    }
    #[inline]
    pub fn set_scx(&mut self, v: u8) {
        self.scx = v;
    }
    #[inline]
    pub fn set_wy(&mut self, v: u8) {
        self.wy = v;
    }
    #[inline]
    pub fn set_wx(&mut self, v: u8) {
        self.wx = v;
    }
    #[inline]
    pub fn set_bgp(&mut self, v: u8) {
        self.bgp = v;
    }

    #[inline]
    pub fn get_lcdc(&self) -> u8 {
        self.lcdc
    }
    #[inline]
    pub fn get_scy(&self) -> u8 {
        self.scy
    }
    #[inline]
    pub fn get_scx(&self) -> u8 {
        self.scx
    }
    #[inline]
    pub fn get_wy(&self) -> u8 {
        self.wy
    }
    #[inline]
    pub fn get_wx(&self) -> u8 {
        self.wx
    }
    #[inline]
    pub fn get_bgp(&self) -> u8 {
        self.bgp
    }
    #[inline]
    pub fn mode_code(&self) -> u8 {
        self.mode.stat_bits()
    }
    #[inline]
    pub fn frame_index(&self) -> u64 {
        self.frame_index
    }

    // --------- timing & DMA ----------
    #[inline]
    fn compute_mode3_length(&self) -> u16 {
        // DMG fetch timing has small SCX-dependent variation.
        // Keep this approximation bounded and preserve 456 dots/line.
        match self.scx & 0x07 {
            5..=7 => MODE3_BASE_DOTS + 2,
            1..=4 => MODE3_BASE_DOTS + 1,
            _ => MODE3_BASE_DOTS,
        }
    }

    #[inline]
    fn latch_visible_line_timing(&mut self) {
        self.mode3_len_latched = self.compute_mode3_length();
        self.hblank_len_latched = LINE_DOTS.saturating_sub(MODE2_OAM_DOTS).saturating_sub(
            self.mode3_len_latched
        );
    }

    pub fn tick<D: DmaRead>(&mut self, mut tcycles: u32, dma: &mut D) -> GpuEvents {
        let mut events = GpuEvents {
            if_set: IfBits::empty(),
            if_from_vblank: false,
            if_from_stat: false,
            frame_became_ready: false,
        };

        if !self.lcd_enabled() {
            while tcycles > 0 {
                tcycles -= 1;
                self.step_oam_dma(dma);
            }
            return events;
        }

        while tcycles > 0 {
            tcycles -= 1;

            // 1 byte per M-cycle during OAM DMA
            self.step_oam_dma(dma);
            self.hblank_irq_early = false;

            match self.mode {
                PpuMode::HBlank0 => {
                    self.mode_dot = self.mode_dot.saturating_add(1);
                    if self.mode_dot >= self.hblank_len_latched {
                        self.mode_dot = 0;
                        // End of scanline: advance LY and decide next mode
                        self.ly = self.ly.wrapping_add(1);

                        if self.ly == 144 {
                            // Enter VBlank
                            self.mode = PpuMode::VBlank1;
                            self.set_stat_mode(PpuMode::VBlank1);

                            // Raise VBlank IF exactly once (on entry to LY=144)
                            events.if_set |= IfBits::VBLANK;
                            events.if_from_vblank = true;
                            self.vblank_irq_raises = self.vblank_irq_raises.saturating_add(1);
                            if !self.frame_ready {
                                self.frame_ready = true;
                                events.frame_became_ready = true;
                                self.frame_index = self.frame_index.saturating_add(1);
                                let mode_switches = self.mode_switches;
                                let stat_irq_raises = self.stat_irq_raises;
                                let stat_m0_edges = self.stat_m0_edges;
                                let stat_m1_edges = self.stat_m1_edges;
                                let stat_m2_edges = self.stat_m2_edges;
                                let stat_lyc_edges = self.stat_lyc_edges;
                                let vblank_irq_raises = self.vblank_irq_raises;
                                self.last_frame_telemetry = Some(FrameTelemetry {
                                    frame_index: self.frame_index,
                                    ly_wraps: self.ly_wraps,
                                    mode_switches,
                                    stat_irq_raises,
                                    stat_m0_edges,
                                    stat_m1_edges,
                                    stat_m2_edges,
                                    stat_lyc_edges,
                                    vblank_irq_raises,
                                    frame_ready_ly: self.ly,
                                    frame_ready_mode: self.mode_code(),
                                });
                                self.mode_switches = [0; 4];
                                self.stat_irq_raises = 0;
                                self.stat_m0_edges = 0;
                                self.stat_m1_edges = 0;
                                self.stat_m2_edges = 0;
                                self.stat_lyc_edges = 0;
                                self.vblank_irq_raises = 0;
                                if trace::enabled(TraceCategory::Frame) {
                                    eprintln!(
                                        "event=frame step={} frame={} ly_wraps={} m0={} m1={} m2={} m3={} stat_irq={} stat_m0={} stat_m1={} stat_m2={} stat_lyc={} vblank_irq={}",
                                        trace::step(),
                                        self.frame_index,
                                        self.ly_wraps,
                                        mode_switches[0],
                                        mode_switches[1],
                                        mode_switches[2],
                                        mode_switches[3],
                                        stat_irq_raises,
                                        stat_m0_edges,
                                        stat_m1_edges,
                                        stat_m2_edges,
                                        stat_lyc_edges,
                                        vblank_irq_raises
                                    );
                                }
                            }
                            self.update_ly_and_lyc();
                        } else if self.ly < 144 {
                            // Start of a visible scanline: Mode 2
                            self.mode = PpuMode::Oam2;
                            // WY latch for the new line: window enabled AND WY == current LY
                            if (self.lcdc & 0x20) != 0 && self.ly == self.wy {
                                self.wy_triggered = true;
                            }
                            self.set_stat_mode(PpuMode::Oam2);
                            self.update_ly_and_lyc();
                        } else if self.ly > 153 {
                            // Wrap to LY=0 and begin new frame
                            self.ly = 0;
                            self.ly_wraps = self.ly_wraps.saturating_add(1);
                            self.window_line_counter = 0;
                            self.mode = PpuMode::Oam2;
                            self.wy_triggered = (self.lcdc & 0x20) != 0 && self.ly == self.wy;
                            self.set_stat_mode(PpuMode::Oam2);
                            self.update_ly_and_lyc();
                        }
                    }
                }

                PpuMode::VBlank1 => {
                    self.mode_dot = self.mode_dot.saturating_add(1);
                    if self.mode_dot >= LINE_DOTS {
                        self.mode_dot = 0;
                        self.ly = self.ly.wrapping_add(1);
                        if self.ly > 153 {
                            // Leave VBlank -> begin new frame at LY=0
                            self.ly = 0;
                            self.ly_wraps = self.ly_wraps.saturating_add(1);
                            self.window_line_counter = 0;
                            self.mode = PpuMode::Oam2;
                            self.wy_triggered = (self.lcdc & 0x20) != 0 && self.ly == self.wy;
                            self.set_stat_mode(PpuMode::Oam2);
                            self.update_ly_and_lyc();
                        } else {
                            // Stay in VBlank
                            self.update_ly_and_lyc();
                        }
                    }
                }

                PpuMode::Oam2 => {
                    self.mode_dot = self.mode_dot.saturating_add(1);
                    if self.mode_dot >= MODE2_OAM_DOTS {
                        // Enter Mode 3 exactly once per visible line
                        self.mode_dot = 0;
                        self.mode = PpuMode::Xfer3;

                        // WX latch for this line at Mode 3 start
                        self.wx_latched = self.wx;
                        self.set_stat_mode(PpuMode::Xfer3);

                        // Count sprites overlapping this line
                        self.sprites_on_line_count = self.count_sprites_on_line();
                        self.latch_visible_line_timing();
                    }
                }

                PpuMode::Xfer3 => {
                    if self.mode_dot.saturating_add(1) >= self.mode3_len_latched {
                        // DMG nuance: mode-0 STAT source goes high one dot
                        // before the mode 3 -> mode 0 transition.
                        self.hblank_irq_early = true;
                        self.maybe_raise_stat_irq(&mut events);
                    }
                    self.mode_dot = self.mode_dot.saturating_add(1);
                    if self.mode_dot >= self.mode3_len_latched {
                        // Render at mode-3 completion (just before entering HBlank).
                        let ly = self.ly;
                        let mut line = [0u8; LCD_WIDTH];
                        self.render_scanline(ly, &mut line);
                        self.render_scanline_objs(ly, &mut line);
                        self.overlay_debug_scanline(ly, &mut line);
                        self.framebuf[ly as usize] = line;

                        self.mode_dot = 0;
                        self.mode = PpuMode::HBlank0;
                        self.set_stat_mode(PpuMode::HBlank0);
                    }
                }
            }

            // Checkpoint B: evaluate normal STAT sources after state updates.
            self.hblank_irq_early = false;
            self.maybe_raise_stat_irq(&mut events);
        }

        events
    }

    fn step_oam_dma<D: DmaRead>(&mut self, dma: &mut D) {
        if !self.dma.active {
            return;
        }
        if self.dma.start_delay_tcycles > 0 {
            self.dma.start_delay_tcycles -= 1;
            return;
        }
        self.dma.tcycle_phase = self.dma.tcycle_phase.wrapping_add(1);
        if self.dma.tcycle_phase < 4 {
            return;
        }
        self.dma.tcycle_phase = 0;
        if self.dma.mcycles_left == 0 || self.dma.byte_idx >= 160 {
            self.dma.active = false;
            return;
        }
        self.dma.mcycles_left -= 1;

        if self.dma.byte_idx < 160 {
            let src = ((self.dma.src_high as u16) << 8) | self.dma.byte_idx;
            let val = dma.read8(src);
            let dst_index = self.dma.byte_idx as usize;
            if dst_index < self.oam.len() {
                self.oam[dst_index] = val;
            }
            self.dma.byte_idx += 1;
        }

        if self.dma.byte_idx >= 160 {
            self.dma.active = false;
        }
    }

    // ------ helpers ------
    fn count_sprites_on_line(&self) -> u8 {
        let sprite_h = if (self.lcdc & 0x04) != 0 { 16 } else { 8 };
        let mut count = 0u8;
        let ly_i = self.ly as i16;
        for i in 0..40 {
            let base = i * 4;
            let y = (self.oam[base] as i16) - 16;
            if ly_i >= y && ly_i < y + sprite_h {
                count += 1;
                if count == 10 {
                    break;
                }
            }
        }
        count
    }

    #[inline]
    fn set_stat_mode(&mut self, mode: PpuMode) {
        let old_mode = self.stat & 0x03;
        let new_mode = mode.stat_bits();
        if old_mode != new_mode {
            self.mode_switches[new_mode as usize] =
                self.mode_switches[new_mode as usize].saturating_add(1);
        }
        self.stat = (self.stat & !0x03) | new_mode;
        // No immediate IF; rising-edge is checked at STAT checkpoints.
    }

    #[inline]
    fn update_ly_and_lyc(&mut self) {
        if self.ly == self.lyc {
            self.stat |= 1 << 2;
        } else {
            self.stat &= !(1 << 2);
        }
    }

    // -------- VRAM helpers --------
    #[inline]
    fn vram_index(abs: u16) -> Option<usize> {
        if abs < VRAM_BEGIN || abs > VRAM_END { None } else { Some((abs - VRAM_BEGIN) as usize) }
    }

    #[inline]
    pub fn read_vram_abs(&self, abs_addr: u16) -> u8 {
        Self::vram_index(abs_addr)
            .map(|ix| self.vram[ix])
            .unwrap_or(0)
    }

    pub fn write_vram_abs(&mut self, abs_addr: u16, value: u8) {
        if let Some(ix) = Self::vram_index(abs_addr) {
            self.write_vram(ix, value);
        }
    }

    #[inline]
    pub fn read_vram(&self, index: usize) -> u8 {
        if self.lcd_enabled() && matches!(self.mode, PpuMode::Xfer3) {
            return 0xff;
        }
        self.vram.get(index).copied().unwrap_or(0)
    }

    pub fn write_vram(&mut self, index: usize, value: u8) {
        if self.lcd_enabled() && matches!(self.mode, PpuMode::Xfer3) {
            return;
        }
        if index >= VRAM_SIZE {
            return;
        }
        self.vram[index] = value;

        if index >= TILE_DATA_LEN {
            return;
        }

        let norm = index & !1;
        let b0 = self.vram[norm];
        let b1 = self.vram[norm + 1];
        let tile_index = norm / 16;
        let row_index = (norm % 16) / 2;

        if tile_index < self.tile_set.len() {
            self.tile_set[tile_index][row_index] = Self::decode_tile_row(b0, b1);
        }
    }

    // -------- tile cache ----------
    #[inline]
    pub fn get_tile(&self, index: usize) -> Option<&Tile> {
        self.tile_set.get(index)
    }
    #[inline]
    pub fn get_tile_mut(&mut self, index: usize) -> Option<&mut Tile> {
        self.tile_set.get_mut(index)
    }

    // -------- BG/Window ----------
    pub fn render_scanline(&mut self, ly: u8, out: &mut [u8; LCD_WIDTH]) {
        let lcd_on = (self.lcdc & 0x80) != 0;
        let bg_on = (self.lcdc & 0x01) != 0;
        let win_on = (self.lcdc & 0x20) != 0;

        // Latches: WY in Mode 2; WX at Mode 3 start.
        let win_left_latched = self.wx_latched.wrapping_sub(7);
        let window_vert_active = win_on && self.wy_triggered;

        let use_8000 = (self.lcdc & 0x10) != 0;

        if !lcd_on || !bg_on {
            let c0 = self.map_bgp(TilePixelValue::Zero);
            out.fill(c0);
            self.bg_idx_line.fill(0);
            return;
        }

        // Tilemap bases (LCDC.3, LCDC.6)
        let bg_map_base_abs: u16 = if (self.lcdc & 0x08) != 0 { 0x9c00 } else { 0x9800 };
        let win_map_base_abs: u16 = if (self.lcdc & 0x40) != 0 { 0x9c00 } else { 0x9800 };
        let bg_map_base = Self::vram_index(bg_map_base_abs).unwrap_or(0);
        let win_map_base = Self::vram_index(win_map_base_abs).unwrap_or(0);

        // BG scrolled Y for this line; compute coarse/fine indices.
        let bg_y = ly.wrapping_add(self.scy);
        let bg_tile_row = ((bg_y as usize) / 8) % 32;
        let bg_fine_y = (bg_y as usize) & 7;

        let mut window_used_this_line = false;

        for x in 0..LCD_WIDTH {
            let window_active_here = window_vert_active && (x as u8) >= win_left_latched;
            if window_active_here {
                window_used_this_line = true;
            }

            let (map_base, tile_col, fine_x, tile_row, fine_y) = if window_active_here {
                let wy_internal = self.window_line_counter as usize;
                let wx0 = (x as u8).wrapping_sub(win_left_latched) as usize;
                let tile_col = (wx0 / 8) % 32;
                let tile_row = (wy_internal / 8) % 32;
                let fine_x = wx0 & 7;
                let fine_y = wy_internal & 7;
                (win_map_base, tile_col, fine_x, tile_row, fine_y)
            } else {
                let bg_x = (x as u8).wrapping_add(self.scx);
                let tile_col = ((bg_x as usize) / 8) % 32;
                let fine_x = (bg_x as usize) & 7;
                (bg_map_base, tile_col, fine_x, bg_tile_row, bg_fine_y)
            };

            let map_off = tile_row * 32 + tile_col;
            let tile_id = self.vram[map_base + map_off];

            let tile_index: usize = if use_8000 {
                tile_id as usize
            } else {
                let n = tile_id as i8 as i16; // [-128,127]
                (256i16 + n) as usize // -> 128..383
            };

            let px = if tile_index < TILE_COUNT {
                self.tile_set[tile_index][fine_y][fine_x]
            } else {
                TilePixelValue::Zero
            };

            self.bg_idx_line[x] = match px {
                TilePixelValue::Zero => 0,
                TilePixelValue::One => 1,
                TilePixelValue::Two => 2,
                TilePixelValue::Three => 3,
            };

            out[x] = self.map_bgp(px);
        }

        if window_used_this_line {
            self.window_line_counter = self.window_line_counter.wrapping_add(1);
            #[cfg(feature = "trace_ppu")]
            if !trace::structured_enabled() {
                eprintln!("[PPU] window drew on ly={}, start_x={}", ly, win_left_latched);
            }
        }
    }

    // -------- Sprites (OBJ) ----------
    pub fn render_scanline_objs(&self, ly: u8, out: &mut [u8; LCD_WIDTH]) {
        let lcd_on = (self.lcdc & 0x80) != 0;
        let obj_on = (self.lcdc & 0x02) != 0;
        if !lcd_on || !obj_on {
            return;
        }

        let obj_8x16 = (self.lcdc & 0x04) != 0;
        let obj_h = if obj_8x16 { 16 } else { 8 };

        // 1) Select up to 10 sprites by Y overlap, in OAM order
        let mut selected: [usize; 10] = [usize::MAX; 10];
        let mut count = 0usize;
        let ly_i16 = ly as i16;

        for i in 0..40 {
            let base = i * 4;
            let y = (self.oam[base] as i16) - 16;
            if ly_i16 >= y && ly_i16 < y + obj_h {
                selected[count] = i;
                count += 1;
                if count == 10 {
                    break;
                }
            }
        }
        if count == 0 {
            return;
        }

        // 2) OBJ→OBJ priority: smaller X first; ties -> lower OAM index
        let mut order: [(usize, i16); 10] = [(usize::MAX, 0); 10];
        for si in 0..count {
            let i = selected[si];
            let base = i * 4;
            let x = (self.oam[base + 1] as i16) - 8;
            order[si] = (i, x);
        }
        for a in 1..count {
            let key = order[a];
            let mut j = a;
            while j > 0 {
                let (ij, xj) = order[j - 1];
                let (ik, xk) = key;
                if xj < xk || (xj == xk && ij < ik) {
                    break;
                }
                order[j] = order[j - 1];
                j -= 1;
            }
            order[j] = key;
        }

        // 3) Pick first non-zero OBJ pixel per x, then apply BG-over-OBJ
        let mut sprite_pixel: [u8; LCD_WIDTH] = [0; LCD_WIDTH];
        let mut sprite_palette: [bool; LCD_WIDTH] = [false; LCD_WIDTH];
        let mut sprite_behind_bg: [bool; LCD_WIDTH] = [false; LCD_WIDTH];

        for si in 0..count {
            let (i, _) = order[si];
            let base = i * 4;
            let oam_y = self.oam[base] as i16;
            let oam_x = self.oam[base + 1] as i16;
            let tile = self.oam[base + 2];
            let attr = self.oam[base + 3];

            let y = oam_y - 16;
            let x = oam_x - 8;

            let behind_bg = (attr & 0x80) != 0;
            let yflip = (attr & 0x40) != 0;
            let xflip = (attr & 0x20) != 0;
            let use_obp1 = (attr & 0x10) != 0;

            let mut line = (ly as i16) - y;
            if yflip {
                line = obj_h - 1 - line;
            }

            let (tile_index, row_in_tile) = if obj_8x16 {
                let base_even = (tile & 0xfe) as usize;
                if line >= 8 {
                    (base_even + 1, (line - 8) as usize)
                } else {
                    (base_even, line as usize)
                }
            } else {
                (tile as usize, line as usize)
            };

            if tile_index >= TILE_COUNT || row_in_tile >= 8 {
                continue;
            }

            for px in 0..8 {
                let sx_i16 = x + ((if xflip { 7 - px } else { px }) as i16);
                if sx_i16 < 0 || sx_i16 >= (LCD_WIDTH as i16) {
                    continue;
                }
                let sx = sx_i16 as usize;
                if sprite_pixel[sx] != 0 {
                    continue;
                }

                let obj_px = self.tile_set[tile_index][row_in_tile][px as usize];
                let obj_idx = match obj_px {
                    TilePixelValue::Zero => 0,
                    TilePixelValue::One => 1,
                    TilePixelValue::Two => 2,
                    TilePixelValue::Three => 3,
                };
                if obj_idx == 0 {
                    continue;
                }

                sprite_pixel[sx] = obj_idx;
                sprite_palette[sx] = use_obp1;
                sprite_behind_bg[sx] = behind_bg;
            }
        }

        for x in 0..LCD_WIDTH {
            let idx = sprite_pixel[x];
            if idx == 0 {
                continue;
            }
            if sprite_behind_bg[x] && self.bg_idx_line[x] != 0 {
                continue;
            }
            out[x] = self.map_obp(idx, sprite_palette[x]);
        }
    }

    // -------- decode helpers --------
    #[inline]
    fn decode_tile_row(b0: u8, b1: u8) -> [TilePixelValue; 8] {
        let mut row = [TilePixelValue::Zero; 8];
        for k in 0..8 {
            let bit = 7 - k;
            let lsb = (b0 >> bit) & 1;
            let msb = (b1 >> bit) & 1;
            row[k] = match (msb, lsb) {
                (0, 0) => TilePixelValue::Zero,
                (0, 1) => TilePixelValue::One,
                (1, 0) => TilePixelValue::Two,
                (1, 1) => TilePixelValue::Three,
                _ => unreachable!(),
            };
        }
        row
    }

    #[inline]
    fn map_bgp(&self, px: TilePixelValue) -> u8 {
        let idx = match px {
            TilePixelValue::Zero => 0,
            TilePixelValue::One => 1,
            TilePixelValue::Two => 2,
            TilePixelValue::Three => 3,
        };
        (self.bgp >> (idx * 2)) & 0b11
    }

    #[inline]
    fn map_obp(&self, px_idx: u8, use_obp1: bool) -> u8 {
        let pal = if use_obp1 { self.obp1 } else { self.obp0 };
        (pal >> (px_idx * 2)) & 0b11
    }

    // -------- debug overlay --------
    pub fn set_debug_config(&mut self, cfg: DebugOverlayConfig) {
        fn clamp2(x: u8) -> u8 {
            x.min(3)
        }
        self.debug = DebugOverlayConfig {
            show_bg_tile_grid: cfg.show_bg_tile_grid,
            show_window_bounds: cfg.show_window_bounds,
            show_bg_axes: cfg.show_bg_axes,
            show_sprite_boxes: cfg.show_sprite_boxes,
            shade_grid: clamp2(cfg.shade_grid),
            shade_window: clamp2(cfg.shade_window),
            shade_axes: clamp2(cfg.shade_axes),
            shade_sprite_box: clamp2(cfg.shade_sprite_box),
        };
    }
    #[inline]
    pub fn debug_config_mut(&mut self) -> &mut DebugOverlayConfig {
        &mut self.debug
    }
    #[inline]
    pub fn debug_config(&self) -> &DebugOverlayConfig {
        &self.debug
    }

    #[inline]
    fn blend(over: u8, base: u8) -> u8 {
        if over > base { over } else { base }
    }

    #[inline]
    fn put_pixel_safe(out: &mut [u8; LCD_WIDTH], x: i32, shade: u8) {
        if (0..LCD_WIDTH as i32).contains(&x) {
            out[x as usize] = Self::blend(shade, out[x as usize]);
        }
    }

    pub fn overlay_debug_scanline(&self, ly: u8, out: &mut [u8; LCD_WIDTH]) {
        if !self.debug.any_enabled() {
            return;
        }

        if self.debug.show_bg_tile_grid {
            let shade = self.debug.shade_grid & 0b11;
            for x in 0..LCD_WIDTH as i32 {
                if (((self.scx as i32) + x) & 7) == 0 {
                    Self::put_pixel_safe(out, x, shade);
                }
            }
            if (((self.scy as i32) + (ly as i32)) & 7) == 0 {
                for x in 0..LCD_WIDTH as i32 {
                    Self::put_pixel_safe(out, x, shade);
                }
            }
        }

        if self.debug.show_window_bounds {
            let win_on = (self.lcdc & 0x20) != 0;
            if win_on {
                let win_left = self.wx.wrapping_sub(7) as i32;
                let wy = self.wy as i32;
                if (ly as i32) == wy {
                    for x in 0..LCD_WIDTH as i32 {
                        Self::put_pixel_safe(out, x, self.debug.shade_window & 0b11);
                    }
                }
                if (0..LCD_WIDTH as i32).contains(&win_left) {
                    Self::put_pixel_safe(out, win_left, self.debug.shade_window & 0b11);
                }
            }
        }

        if self.debug.show_bg_axes {
            let shade = self.debug.shade_axes & 0b11;
            if (0..LCD_WIDTH as i32).contains(&(0 - (self.scx as i32))) {
                Self::put_pixel_safe(out, 0 - (self.scx as i32), shade);
            }
            if (self.scy as i32) + (ly as i32) == 0 {
                for x in 0..LCD_WIDTH as i32 {
                    Self::put_pixel_safe(out, x, shade);
                }
            }
        }

        if self.debug.show_sprite_boxes {
            let shade = self.debug.shade_sprite_box & 0b11;
            for i in 0..40 {
                let base = i * 4;
                let sprite_y = (self.oam[base] as i32) - 16;
                let sprite_x = (self.oam[base + 1] as i32) - 8;
                let obj_8x16 = (self.lcdc & 0x04) != 0;
                let h = if obj_8x16 { 16 } else { 8 };
                let line = ly as i32;
                if line >= sprite_y && line < sprite_y + h {
                    for px in 0..8 {
                        Self::put_pixel_safe(out, sprite_x + px, shade);
                    }
                    Self::put_pixel_safe(out, sprite_x, shade);
                    Self::put_pixel_safe(out, sprite_x + 7, shade);
                }
            }
        }
    }

    // --------------- OAM DMA public API (FF46) ----------------
    pub fn write_ff46_start_dma(&mut self, v: u8) {
        self.dma.active = true;
        self.dma.src_high = v;
        self.dma.byte_idx = 0;
        self.dma.mcycles_left = 160; // 160 M-cycles (normal speed)
        self.dma.start_delay_tcycles = 4; // one M-cycle startup delay
        self.dma.tcycle_phase = 0;
    }

    #[inline]
    pub fn dma_in_progress(&self) -> bool {
        self.dma.active
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoDma;
    impl DmaRead for NoDma {
        fn read8(&mut self, _addr: u16) -> u8 {
            0
        }
    }

    #[test]
    fn stat_irq_is_rising_edge_only_across_source_overlap() {
        let mut gpu = GPU::new();
        gpu.mode = PpuMode::Oam2;
        gpu.set_stat_mode(PpuMode::Oam2);
        gpu.ly = 0;
        gpu.lyc = 0;
        gpu.update_ly_and_lyc();

        assert!(!gpu.write_stat(0x00));
        assert!(gpu.write_stat(1 << 5)); // M2 source goes low->high.
        assert!(!gpu.write_stat(1 << 6)); // Switch to LYC source while line stays high.
        assert!(!gpu.write_stat(0x00)); // Line drops low.
        assert!(gpu.write_stat(1 << 6)); // Low->high again.
    }

    #[test]
    fn mode0_pre_transition_raises_once() {
        let mut gpu = GPU::new();
        let mut dma = NoDma;

        gpu.mode = PpuMode::Xfer3;
        gpu.set_stat_mode(PpuMode::Xfer3);
        gpu.mode_dot = gpu.mode3_len_latched.saturating_sub(1);
        gpu.write_stat(1 << 3); // Enable mode-0 STAT source only.
        gpu.stat_irq_line_prev = false;

        let events = gpu.tick(1, &mut dma);
        assert!(events.if_from_stat);
        assert!(events.if_set.contains(IfBits::LCD_STAT));
        assert_eq!(gpu.mode_code(), PpuMode::HBlank0.stat_bits());
        assert_eq!(gpu.stat_irq_raises, 1);
    }

    #[test]
    fn lyc_write_can_raise_immediate_stat_edge() {
        let mut gpu = GPU::new();
        gpu.mode = PpuMode::Xfer3;
        gpu.set_stat_mode(PpuMode::Xfer3);
        gpu.ly = 23;
        gpu.lyc = 0;
        gpu.update_ly_and_lyc();
        assert!(!gpu.write_stat(1 << 6)); // Enable LYC source while mismatch.
        assert!(gpu.write_lyc(23)); // Match LY immediately -> edge.
        assert!(!gpu.write_lyc(23)); // Line is still high, no retrigger.
    }

    #[test]
    fn stat_write_enabling_true_source_raises_once() {
        let mut gpu = GPU::new();
        gpu.mode = PpuMode::Oam2;
        gpu.set_stat_mode(PpuMode::Oam2);
        gpu.ly = 10;
        gpu.lyc = 42;
        gpu.update_ly_and_lyc();

        assert!(!gpu.write_stat(0x00));
        assert!(gpu.write_stat(1 << 5)); // Enable M2 while in mode 2.
        assert!(!gpu.write_stat(1 << 5)); // Still high, blocked.
    }
}
