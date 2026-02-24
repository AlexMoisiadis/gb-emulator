// src/gpu.rs
//
// DMG PPU timing fix:
// - Consume all tcycles in GPU::tick (no early returns on IF)
// - Constant 456 dots per line (Mode3=172 nominal on DMG)
// - Single VBlank IF raise on entering LY=144
// - STAT rising-edge aggregation without stopping time

use std::array::from_fn;

pub const VRAM_BEGIN: u16 = 0x8000;
pub const VRAM_END: u16 = 0x9fff;
pub const VRAM_SIZE: usize = (VRAM_END as usize) - (VRAM_BEGIN as usize) + 1;

pub const TILE_COUNT: usize = 384;
const TILE_DATA_LEN: usize = 0x1800; // 6 KiB

pub const LCD_WIDTH: usize = 160;
pub const LCD_HEIGHT: usize = 144;

type Tile = [[TilePixelValue; 8]; 8];

bitflags::bitflags! {
    pub struct IfBits: u8 {
        const VBLANK   = 0b0000_0001;
        const LCD_STAT = 0b0000_0010;
    }
}

pub struct GpuEvents {
    pub if_set: IfBits,
    pub frame_became_ready: bool,
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
    dot_in_mode: u16, // 0..=455 across a line (we clamp per-mode)
    ly: u8,
    frame_ready: bool,
    framebuf: [[u8; LCD_WIDTH]; LCD_HEIGHT],
    stat: u8,
    lyc: u8,
    lyc_irq_pending: bool,

    // OAM DMA (FF46)
    dma_active: bool,
    dma_src_high: u8,
    dma_byte_idx: u16,
    dma_mcycles_left: u16,

    // For Mode 3 length (DMG nominal, kept constant here)
    sprites_on_line_count: u8,
    stat_irq_line_prev: bool,

    // Window latches
    wy_latched_this_line: bool,
    wx_latched_for_line: u8,
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
            dot_in_mode: 0,
            ly: 0,
            frame_ready: false,
            framebuf: [[0; LCD_WIDTH]; LCD_HEIGHT],
            stat: 0,
            lyc: 0,
            lyc_irq_pending: false,
            dma_active: false,
            dma_src_high: 0,
            dma_byte_idx: 0,
            dma_mcycles_left: 0,
            sprites_on_line_count: 0,
            stat_irq_line_prev: false,
            wy_latched_this_line: false,
            wx_latched_for_line: 0,
        }
    }

    #[inline]
    fn stat_line_active_now(&self) -> bool {
        let lyc_src = (self.stat & (1 << 6)) != 0 && self.ly == self.lyc;
        let m0_src = (self.stat & (1 << 3)) != 0 && matches!(self.mode, PpuMode::HBlank0);
        let m1_src = (self.stat & (1 << 4)) != 0 && matches!(self.mode, PpuMode::VBlank1);
        let m2_src = (self.stat & (1 << 5)) != 0 && matches!(self.mode, PpuMode::Oam2);
        lyc_src || m0_src || m1_src || m2_src
    }

    #[inline]
    fn maybe_raise_stat_irq(&mut self, events: &mut GpuEvents) {
        let now = self.stat_line_active_now();
        if now && !self.stat_irq_line_prev {
            events.if_set |= IfBits::LCD_STAT; // rising edge
        }
        self.stat_irq_line_prev = now;
    }

    pub fn begin_frame(&mut self) {
        self.window_line_counter = 0;
    }

    // --------- register-ish API ----------
    pub fn write_lyc(&mut self, v: u8) {
        self.lyc = v;
        let equal = self.ly == self.lyc;
        if equal {
            self.stat |= 1 << 2;
            if (self.stat & (1 << 6)) != 0 {
                self.lyc_irq_pending = true;
            }
        } else {
            self.stat &= !(1 << 2);
        }
    }

    #[inline]
    pub fn take_lyc_irq_pending(&mut self) -> bool {
        let p = self.lyc_irq_pending;
        self.lyc_irq_pending = false;
        p
    }
    #[inline]
    pub fn ly(&self) -> u8 {
        self.ly
    }
    #[inline]
    pub fn write_stat(&mut self, v: u8) {
        self.stat = (self.stat & 0x07) | (v & 0x78);
    }
    #[inline]
    pub fn read_stat(&self) -> u8 {
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
        self.oam.get(i).copied().unwrap_or(0)
    }
    #[inline]
    pub fn write_oam(&mut self, i: usize, v: u8) {
        if i < self.oam.len() {
            self.oam[i] = v;
        }
    }

    // LCDC & scroll
    #[inline]
    pub fn set_lcdc(&mut self, v: u8) {
        self.lcdc = v;
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

    // --------- timing & DMA ----------
    #[inline]
    fn compute_mode3_length(&self) -> u16 {
        // Fixed nominal for DMG: keep line length = 456 dots
        // (Reintroduce variability behind a feature if you want)
        172
    }

    pub fn tick<D: DmaRead>(&mut self, mut tcycles: u32, dma: &mut D) -> GpuEvents {
        const LINE_DOTS: u16 = 456;
        const MODE2_OAM: u16 = 80;
        const MODE3_XFER: u16 = 172; // nominal DMG
        const MODE0_HBLK: u16 = LINE_DOTS - MODE2_OAM - MODE3_XFER;

        let mut events = GpuEvents { if_set: IfBits::empty(), frame_became_ready: false };

        while tcycles > 0 {
            tcycles -= 1;

            // 1 byte per M-cycle during OAM DMA
            self.step_oam_dma(dma);

            match self.mode {
                PpuMode::HBlank0 => {
                    self.dot_in_mode = self.dot_in_mode.saturating_add(1);
                    if self.dot_in_mode >= MODE0_HBLK {
                        self.dot_in_mode = 0;
                        // End of scanline: advance LY and decide next mode
                        self.ly = self.ly.wrapping_add(1);

                        if self.ly == 144 {
                            // Enter VBlank
                            self.mode = PpuMode::VBlank1;
                            self.set_stat_mode(PpuMode::VBlank1, &mut events);

                            // Raise VBlank IF exactly once (on entry to LY=144)
                            events.if_set |= IfBits::VBLANK;
                            if !self.frame_ready {
                                self.frame_ready = true;
                                events.frame_became_ready = true;
                            }
                            self.update_ly_and_lyc(&mut events);
                        } else if self.ly < 144 {
                            // Start of a visible scanline: Mode 2
                            self.mode = PpuMode::Oam2;
                            // WY latch for the new line: window enabled AND WY == current LY
                            self.wy_latched_this_line =
                                (self.lcdc & 0x20) != 0 && self.ly == self.wy;
                            self.set_stat_mode(PpuMode::Oam2, &mut events);
                            self.update_ly_and_lyc(&mut events);
                        } else if self.ly > 153 {
                            // Wrap to LY=0 and begin new frame
                            self.ly = 0;
                            self.window_line_counter = 0;
                            self.mode = PpuMode::Oam2;
                            self.wy_latched_this_line =
                                (self.lcdc & 0x20) != 0 && self.ly == self.wy;
                            self.set_stat_mode(PpuMode::Oam2, &mut events);
                            self.update_ly_and_lyc(&mut events);
                        }
                    }
                }

                PpuMode::VBlank1 => {
                    self.dot_in_mode = self.dot_in_mode.saturating_add(1);
                    if self.dot_in_mode >= LINE_DOTS {
                        self.dot_in_mode = 0;
                        self.ly = self.ly.wrapping_add(1);
                        if self.ly > 153 {
                            // Leave VBlank -> begin new frame at LY=0
                            self.ly = 0;
                            self.window_line_counter = 0;
                            self.mode = PpuMode::Oam2;
                            self.wy_latched_this_line =
                                (self.lcdc & 0x20) != 0 && self.ly == self.wy;
                            self.set_stat_mode(PpuMode::Oam2, &mut events);
                            self.update_ly_and_lyc(&mut events);
                        } else {
                            // Stay in VBlank
                            self.update_ly_and_lyc(&mut events);
                        }
                    }
                }

                PpuMode::Oam2 => {
                    self.dot_in_mode = self.dot_in_mode.saturating_add(1);
                    if self.dot_in_mode >= MODE2_OAM {
                        // Enter Mode 3 exactly once per visible line
                        self.dot_in_mode = 0;
                        self.mode = PpuMode::Xfer3;

                        // WX latch for this line at Mode 3 start
                        self.wx_latched_for_line = self.wx;
                        self.set_stat_mode(PpuMode::Xfer3, &mut events);

                        // Count sprites overlapping this line
                        self.sprites_on_line_count = self.count_sprites_on_line();

                        // Render the line
                        let ly = self.ly;
                        let mut line = [0u8; LCD_WIDTH];
                        self.render_scanline(ly, &mut line);
                        self.render_scanline_objs(ly, &mut line);
                        self.overlay_debug_scanline(ly, &mut line);
                        self.framebuf[ly as usize] = line;
                    }
                }

                PpuMode::Xfer3 => {
                    self.dot_in_mode = self.dot_in_mode.saturating_add(1);
                    // Fixed DMG length for determinism
                    if self.dot_in_mode >= self.compute_mode3_length() {
                        self.dot_in_mode = 0;
                        self.mode = PpuMode::HBlank0;
                        self.set_stat_mode(PpuMode::HBlank0, &mut events);
                    }
                }
            }

            // STAT rising-edge check once per dot
            // (This call considers LYC, Mode sources as of *current* state)
            self.maybe_raise_stat_irq(&mut events);
        }

        events
    }

    fn step_oam_dma<D: DmaRead>(&mut self, dma: &mut D) {
        if !self.dma_active {
            return;
        }
        if self.dma_mcycles_left == 0 {
            self.dma_active = false;
            return;
        }

        self.dma_mcycles_left -= 1;

        if self.dma_byte_idx < 160 {
            let src = ((self.dma_src_high as u16) << 8) | self.dma_byte_idx;
            let val = dma.read8(src);
            let dst_index = self.dma_byte_idx as usize;
            if dst_index < self.oam.len() {
                self.oam[dst_index] = val;
            }
            self.dma_byte_idx += 1;
        }

        if self.dma_byte_idx >= 160 {
            self.dma_active = false;
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
    fn set_stat_mode(&mut self, mode: PpuMode, _events: &mut GpuEvents) {
        self.stat = (self.stat & !0x03) | mode.stat_bits();
        // No immediate IF; we’ll check rising-edge in maybe_raise_stat_irq()
    }

    #[inline]
    fn update_ly_and_lyc(&mut self, _events: &mut GpuEvents) {
        if self.ly == self.lyc {
            self.stat |= 1 << 2;
        } else {
            self.stat &= !(1 << 2);
        }
        // Rising-edge from LYC enable is handled in maybe_raise_stat_irq(),
        // which runs once per dot.
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
        self.vram.get(index).copied().unwrap_or(0)
    }

    pub fn write_vram(&mut self, index: usize, value: u8) {
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
        let win_left_latched = self.wx_latched_for_line.wrapping_sub(7);
        let window_vert_active = win_on && self.wy_latched_this_line;

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
            eprintln!("[PPU] window drew on ly={}, start_x={}", ly, win_left_latched);
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
        let mut cand_idx: [u8; LCD_WIDTH] = [0; LCD_WIDTH];
        let mut cand_obp1: [bool; LCD_WIDTH] = [false; LCD_WIDTH];
        let mut cand_bgbit: [bool; LCD_WIDTH] = [false; LCD_WIDTH];

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
                if cand_idx[sx] != 0 {
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

                cand_idx[sx] = obj_idx;
                cand_obp1[sx] = use_obp1;
                cand_bgbit[sx] = behind_bg;
            }
        }

        for x in 0..LCD_WIDTH {
            let idx = cand_idx[x];
            if idx == 0 {
                continue;
            }
            if cand_bgbit[x] && self.bg_idx_line[x] != 0 {
                continue;
            }
            out[x] = self.map_obp(idx, cand_obp1[x]);
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
        self.dma_active = true;
        self.dma_src_high = v;
        self.dma_byte_idx = 0;
        self.dma_mcycles_left = 160; // 160 M-cycles (normal speed)
    }

    #[inline]
    pub fn dma_in_progress(&self) -> bool {
        self.dma_active
    }
}
