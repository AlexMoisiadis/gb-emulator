// src/gpu.rs
use std::array::from_fn;

/// Game Boy VRAM window for tile/tilemap data.
/// (DMG/CGB share this base window; banking is not modeled here.)
pub const VRAM_BEGIN: u16 = 0x8000;
pub const VRAM_END: u16 = 0x9fff;
pub const VRAM_SIZE: usize = (VRAM_END as usize) - (VRAM_BEGIN as usize) + 1;

/// Number of tiles in the 0x8000 tile data block on DMG (256+128)
/// - 0x8000-0x8FFF: 256 tiles (unsigned index mode)
/// - 0x9000-0x97FF: 128 tiles (signed index mode base)
/// We cache 384 tiles total (indices 0..383).
pub const TILE_COUNT: usize = 384;

/// The first 0x1800 bytes (0x8000..=0x97FF) are tile pattern data.
/// The rest contains tile maps (0x9800..=0x9FFF).
const TILE_DATA_LEN: usize = 0x1800; // 6 KiB

/// LCD dimensions (visible screen)
pub const LCD_WIDTH: usize = 160;
pub const LCD_HEIGHT: usize = 144;

type Tile = [[TilePixelValue; 8]; 8];

#[derive(Copy, Clone, Debug)]
pub struct DebugOverlayConfig {
    /// Draw BG tile grid every 8 pixels in screen space, taking SCX/SCY into account.
    pub show_bg_tile_grid: bool,
    /// Outline the Window: top border at WY, left border at WX-7 when the window is enabled.
    pub show_window_bounds: bool,
    /// Draw axes for the BG origin (SCX=0 vertical line, SCY=0 horizontal line) if visible.
    pub show_bg_axes: bool,
    /// Draw 8×8 / 8×16 OBJ bounding boxes using OAM entries (all 40 are considered).
    pub show_sprite_boxes: bool,
    /// Shade (0..3) used for grid lines.
    pub shade_grid: u8,
    /// Shade (0..3) used for window bounds.
    pub shade_window: u8,
    /// Shade (0..3) used for BG axes.
    pub shade_axes: u8,
    /// Shade (0..3) used for sprite boxes.
    pub shade_sprite_box: u8,
}

impl Default for DebugOverlayConfig {
    fn default() -> Self {
        Self {
            show_bg_tile_grid: false,
            show_window_bounds: false,
            show_bg_axes: false,
            show_sprite_boxes: false,
            // choose dark shades for overlays so they are visible on all palettes
            shade_grid: 3,
            shade_window: 2,
            shade_axes: 1,
            shade_sprite_box: 3,
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

pub struct GPU {
    // VRAM contents
    vram: [u8; VRAM_SIZE],
    // Decoded tiles cache (2bpp -> 4 color indices)
    tile_set: [Tile; TILE_COUNT],

    // --- PPU-visible registers (DMG subset) ---
    lcdc: u8, // LCD Control (FF40)
    scy: u8, // Scroll Y (FF42)
    scx: u8, // Scroll X (FF43)
    wy: u8, // Window Y (FF4A)
    wx: u8, // Window X (FF4B) (note: internal uses WX-7)
    bgp: u8, // BG Palette (FF47) (DMG only)

    // --- OAM (Object Attribute Memory) ---
    oam: [u8; 160], // 40 sprites * 4 bytes

    // --- OBJ palettes (DMG) ---
    obp0: u8, // FF48
    obp1: u8, // FF49

    // --- Scratch: BG raw color indices for current scanline (0..3) ---
    bg_idx_line: [u8; LCD_WIDTH],

    // --- Window internal line counter (DMG-accurate) ---
    window_line_counter: u8,
    // --- Debug overlay configuration (all off by default) ---
    debug: DebugOverlayConfig,
}

impl GPU {
    pub fn new() -> Self {
        Self {
            vram: [0; VRAM_SIZE],
            tile_set: from_fn(|_| empty_tile()),
            // Common defaults: LCD on + BG/window on + unsigned tiles + BG map 0
            lcdc: 0x91,
            scy: 0,
            scx: 0,
            wy: 0,
            wx: 0,
            // Typical DMG defaults (boot ROM often sets BGP=0xFC)
            bgp: 0xfc,
            oam: [0; 160],
            obp0: 0xff,
            obp1: 0xff,
            bg_idx_line: [0; LCD_WIDTH],
            window_line_counter: 0,
            debug: DebugOverlayConfig::default(),
        }
    }

    /// Call this at LY=0 (start of a new frame / after VBlank)
    /// Resets the window internal line counter per Pan Docs.
    pub fn begin_frame(&mut self) {
        self.window_line_counter = 0;
    }

    // ---------------------------
    // OBJ palette accessors
    // ---------------------------
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

    // ---------------------------
    // OAM read/write (index 0..159)
    // ---------------------------
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

    // ---------------------------
    // Register accessors
    // ---------------------------
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

    // ---------------------------
    // Addressing helpers
    // ---------------------------

    /// Convert absolute address (0x8000..=0x9FFF) to VRAM index.
    #[inline]
    fn vram_index(abs: u16) -> Option<usize> {
        if abs < VRAM_BEGIN || abs > VRAM_END { None } else { Some((abs - VRAM_BEGIN) as usize) }
    }

    /// Safe read from VRAM by absolute address. Returns 0 if out of range.
    #[inline]
    pub fn read_vram_abs(&self, abs_addr: u16) -> u8 {
        Self::vram_index(abs_addr)
            .map(|ix| self.vram[ix])
            .unwrap_or(0)
    }

    /// Safe write to VRAM by absolute address. Ignores out-of-range writes.
    /// Also updates the decoded tile cache when writing within tile pattern region.
    pub fn write_vram_abs(&mut self, abs_addr: u16, value: u8) {
        if let Some(ix) = Self::vram_index(abs_addr) {
            self.write_vram(ix, value);
        }
    }

    // ---------------------------
    // Index-based access (internal/core)
    // ---------------------------

    /// Read VRAM by (already-offset) index [0..VRAM_SIZE). Returns 0 if OOB.
    #[inline]
    pub fn read_vram(&self, index: usize) -> u8 {
        self.vram.get(index).copied().unwrap_or(0)
    }

    /// Write VRAM by index [0..VRAM_SIZE). No-ops if OOB.
    /// If within tile pattern data region (first 0x1800 bytes),
    /// decodes that tile row into the tile cache.
    pub fn write_vram(&mut self, index: usize, value: u8) {
        if index >= VRAM_SIZE {
            return; // ignore OOB writes
        }

        self.vram[index] = value;

        // Only the tile pattern area updates the tile cache.
        if index >= TILE_DATA_LEN {
            return;
        }

        // Each tile is 16 bytes: 8 rows, 2 bytes per row (low and high planes).
        // Normalize to an even index so we always read the low/high pair.
        let norm = index & !1; // clear bit 0 => even
        let b0 = self.vram[norm];
        let b1 = self.vram[norm + 1];

        let tile_index = norm / 16;
        let row_index = (norm % 16) / 2;

        if tile_index >= self.tile_set.len() {
            // Defensive: should not happen if TILE_DATA_LEN and TILE_COUNT are consistent.
            return;
        }

        // Decode the 2-bit plane row into 8 pixels.
        let row = Self::decode_tile_row(b0, b1);
        self.tile_set[tile_index][row_index] = row;
    }

    // ---------------------------
    // Tile cache API
    // ---------------------------

    #[inline]
    pub fn get_tile(&self, index: usize) -> Option<&Tile> {
        self.tile_set.get(index)
    }

    #[inline]
    pub fn get_tile_mut(&mut self, index: usize) -> Option<&mut Tile> {
        self.tile_set.get_mut(index)
    }

    // ---------------------------
    // BG/Window scanline rendering (DMG)
    // ---------------------------

    /// Render the background/window for scanline `ly` into `out[0..160]`.
    /// Output values are DMG shade indices 0..3 after BGP mapping
    /// (0 = white, 3 = black in typical palette).
    ///
    /// This does NOT draw sprites (OBJ).
    pub fn render_scanline(&mut self, ly: u8, out: &mut [u8; LCD_WIDTH]) {
        // LCDC gates
        let lcd_on = (self.lcdc & 0x80) != 0;
        let bg_on = (self.lcdc & 0x01) != 0;
        let win_on = (self.lcdc & 0x20) != 0; // Window enable (bit5)
        let use_8000 = (self.lcdc & 0x10) != 0; // Tile data select: 1=0x8000 unsigned; 0=0x8800 signed

        if !lcd_on || !bg_on {
            // If LCD or BG is off, fill with color 0 (as per DMG behavior)
            let c0 = self.map_bgp(TilePixelValue::Zero);
            out.fill(c0);
            return;
        }

        // Select tilemap base addresses (absolute)
        let bg_map_base_abs: u16 = if (self.lcdc & 0x08) != 0 { 0x9c00 } else { 0x9800 };
        let win_map_base_abs: u16 = if (self.lcdc & 0x40) != 0 { 0x9c00 } else { 0x9800 };

        // Precompute VRAM indices for tile maps
        let bg_map_base = Self::vram_index(bg_map_base_abs).unwrap_or(0);
        let win_map_base = Self::vram_index(win_map_base_abs).unwrap_or(0);

        // Fine Y within the 256-line BG space and per-window
        let bg_y = ly.wrapping_add(self.scy); // wrap at 256
        let bg_tile_row = ((bg_y as usize) / 8) % 32;
        let bg_fine_y = (bg_y as usize) & 7;

        // Window left x with WX-7 quirk
        let win_left = self.wx.wrapping_sub(7);

        // Determine if window is vertically active THIS scanline
        // per Pan Docs: Window visible if ly >= WY and LCDC.5 set
        let window_vert_active = win_on && ly >= self.wy;

        // Track if window contributed any pixel on this line
        let mut window_used_this_line = false;

        // Render pixels
        for x in 0..LCD_WIDTH {
            let window_active_here = window_vert_active && (x as u8) >= win_left;

            if window_active_here {
                window_used_this_line = true;
            }

            let (map_base, tile_col, fine_x, tile_row, fine_y) = if window_active_here {
                // Use the persistent window internal counter (DMG-accurate)
                let wy_internal = self.window_line_counter as usize;
                let wx0 = (x as u8).wrapping_sub(win_left) as usize;

                let tile_col = (wx0 / 8) % 32;
                let tile_row = (wy_internal / 8) % 32;
                let fine_x = wx0 & 7;
                let fine_y = wy_internal & 7;

                (win_map_base, tile_col, fine_x, tile_row, fine_y)
            } else {
                // Background with scrolling
                let bg_x = (x as u8).wrapping_add(self.scx);
                let tile_col = ((bg_x as usize) / 8) % 32;
                let fine_x = (bg_x as usize) & 7;

                (bg_map_base, tile_col, fine_x, bg_tile_row, bg_fine_y)
            };

            // Fetch tile id from the map (32x32 = 1024 bytes)
            let map_off = tile_row * 32 + tile_col;
            let tile_id = self.vram[map_base + map_off]; // assumes valid map_base + 0..1023

            // Resolve tile index into the tile_set based on LCDC bit4
            let tile_index: usize = if use_8000 {
                // Unsigned index, 0..255 -> tiles 0..255
                tile_id as usize
            } else {
                // Signed index, base at 0x9000 => tile #0 at 0x9000 = tile index 256
                let n = tile_id as i8 as i16;
                (256i16 + n) as usize // range 128..383
            };

            // Read pixel color from tile cache (defensive bounds)
            let px = if tile_index < TILE_COUNT {
                self.tile_set[tile_index][fine_y][fine_x]
            } else {
                TilePixelValue::Zero
            };

            // Record BG raw color index (0..3) for OBJ priority decisions.
            let bg_idx = match px {
                TilePixelValue::Zero => 0,
                TilePixelValue::One => 1,
                TilePixelValue::Two => 2,
                TilePixelValue::Three => 3,
            };
            self.bg_idx_line[x] = bg_idx;

            // Map via BGP palette (DMG) to shade for display
            out[x] = self.map_bgp(px);
        }

        // Increment the window line counter only if the window actually contributed pixels.
        if window_used_this_line {
            self.window_line_counter = self.window_line_counter.wrapping_add(1);
        }
    }

    /// Overlay OBJ (sprites) on top of the already-drawn BG/Window for scanline `ly`.
    /// `out` holds DMG shades (0..3). Uses `bg_idx_line` for OBJ↔BG priority.
    /// DMG OBJ→OBJ priority: smaller X in front; ties -> lower OAM index.  [Pan Docs / gbdev]
    pub fn render_scanline_objs(&self, ly: u8, out: &mut [u8; LCD_WIDTH]) {
        // OBJ rendering enabled only if LCD is on and OBJ enable (LCDC bit1)
        let lcd_on = (self.lcdc & 0x80) != 0;
        let obj_on = (self.lcdc & 0x02) != 0;
        if !lcd_on || !obj_on {
            return;
        }

        let obj_8x16 = (self.lcdc & 0x04) != 0; // LCDC bit2
        let obj_h = if obj_8x16 { 16 } else { 8 };

        // 1) Select up to 10 sprites that intersect this scanline, in OAM order.
        let mut selected: [usize; 10] = [usize::MAX; 10];
        let mut count = 0usize;
        for i in 0..40 {
            let base = i * 4;
            let oam_y = self.oam[base] as i16;
            let y = oam_y - 16; // on-screen Y
            let ly_i16 = ly as i16;
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

        // 2) DMG OBJ→OBJ priority among the selected set:
        //    smaller X wins (front), tie -> lower OAM index.
        //    We'll sort by (x asc, idx asc), then draw in reverse (back-to-front),
        //    so that the "frontmost" sprite is drawn last.
        let mut order: [(usize, i16); 10] = [(usize::MAX, 0); 10];
        for si in 0..count {
            let i = selected[si];
            let base = i * 4;
            let oam_x = self.oam[base + 1] as i16;
            let x = oam_x - 8; // on-screen X (can be <0 or >=160 but still participates in priority)
            order[si] = (i, x);
        }
        // Simple selection sort over the first `count` entries
        for a in 0..count {
            let mut min = a;
            for b in a + 1..count {
                let (i_b, x_b) = order[b];
                let (i_m, x_m) = order[min];
                if x_b < x_m || (x_b == x_m && i_b < i_m) {
                    min = b;
                }
            }
            if min != a {
                order.swap(a, min);
            }
        }

        // 3) Draw in reverse sorted order (back-to-front).
        for si in (0..count).rev() {
            let (i, _) = order[si];
            let base = i * 4;
            let oam_y = self.oam[base] as i16;
            let oam_x = self.oam[base + 1] as i16;
            let tile = self.oam[base + 2];
            let attr = self.oam[base + 3];

            let y = oam_y - 16;
            let x = oam_x - 8;

            let behind_bg = (attr & 0x80) != 0; // bit7
            let yflip = (attr & 0x40) != 0; // bit6
            let xflip = (attr & 0x20) != 0; // bit5
            let use_obp1 = (attr & 0x10) != 0; // bit4

            // Line inside the sprite
            let mut line = (ly as i16) - y;
            if yflip {
                line = obj_h - 1 - line;
            }

            // Resolve tile index and row within the tile cache.
            // DMG OBJ always uses 0x8000 tile data (unsigned indices 0..255).
            // In 8x16, the base tile is even; lines >= 8 use the next tile.
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

            // Defensive bounds
            if tile_index >= TILE_COUNT || row_in_tile >= 8 {
                continue;
            }

            // For each pixel in the tile row
            for px in 0..8 {
                let screen_x = x + ((if xflip { 7 - px } else { px }) as i16);
                if screen_x < 0 || screen_x >= (LCD_WIDTH as i16) {
                    continue;
                }
                let sx = screen_x as usize;

                // OBJ pixel (0..3), 0 is transparent
                let obj_px = self.tile_set[tile_index][row_in_tile][px as usize];
                let obj_idx = match obj_px {
                    TilePixelValue::Zero => 0,
                    TilePixelValue::One => 1,
                    TilePixelValue::Two => 2,
                    TilePixelValue::Three => 3,
                };
                if obj_idx == 0 {
                    continue; // transparent
                }

                // BG↔OBJ priority: if behind_bg=1, sprite is behind BG unless BG color index is 0.
                if behind_bg && self.bg_idx_line[sx] != 0 {
                    continue;
                }

                // Map through OBP0/OBP1 (DMG)
                let shade = self.map_obp(obj_idx, use_obp1);
                out[sx] = shade;
            }
        }
    }

    // ---------------------------
    // Decode helpers
    // ---------------------------

    /// Decode one tile row from the two bitplanes (low=b0, high=b1):
    /// For pixel k (0..7):
    ///   bit = 7 - k
    ///   color_id = ((b1>>bit)&1)<<1 | ((b0>>bit)&1)
    ///   map 0..3 -> Zero..Three
    #[inline]
    fn decode_tile_row(b0: u8, b1: u8) -> [TilePixelValue; 8] {
        let mut row = [TilePixelValue::Zero; 8];
        // Bit 7 => pixel 0; Bit 0 => pixel 7 (left-to-right)
        for k in 0..8 {
            let bit = 7 - k;
            let lsb = (b0 >> bit) & 1;
            let msb = (b1 >> bit) & 1;
            row[k] = match (lsb, msb) {
                (0, 0) => TilePixelValue::Zero,
                (1, 0) => TilePixelValue::One,
                (0, 1) => TilePixelValue::Two,
                (1, 1) => TilePixelValue::Three,
                _ => unreachable!(),
            };
        }
        row
    }

    /// DMG BG palette mapping (BGP, FF47).
    /// BGP bits: [7:6]=color3, [5:4]=color2, [3:2]=color1, [1:0]=color0.
    /// Returns shade index 0..3 (0=white, 3=black in typical DMG palette).
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

    /// DMG OBJ palette mapping (OBP0/OBP1).
    /// Same bit packing as BGP. OBJ color index 0 is transparent by rule.
    #[inline]
    fn map_obp(&self, px_idx: u8, use_obp1: bool) -> u8 {
        let pal = if use_obp1 { self.obp1 } else { self.obp0 };
        (pal >> (px_idx * 2)) & 0b11
    }

    // ---------------------------
    // Debug overlay configuration
    // ---------------------------
    /// Replace the current debug overlay configuration.
    pub fn set_debug_config(&mut self, cfg: DebugOverlayConfig) {
        // clamp shades to 0..3 to avoid invalid DMG shade indices
        fn clamp2(x: u8) -> u8 {
            if x > 3 { 3 } else { x }
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
    /// Borrow the debug configuration for in-place edits.
    #[inline]
    pub fn debug_config_mut(&mut self) -> &mut DebugOverlayConfig {
        &mut self.debug
    }
    /// Read-only access to the debug configuration.
    #[inline]
    pub fn debug_config(&self) -> &DebugOverlayConfig {
        &self.debug
    }

    /// Apply optional debug overlays to the already rendered scanline in `out`.
    /// Call this after `render_scanline` and `render_scanline_objs` for line `ly`.
    /// Overlays are drawn using simple blending that prefers darker shades (max of DMG shade indices).
    pub fn overlay_debug_scanline(&self, ly: u8, out: &mut [u8; LCD_WIDTH]) {
        // early exit if nothing enabled
        if
            !(
                self.debug.show_bg_tile_grid ||
                self.debug.show_window_bounds ||
                self.debug.show_bg_axes ||
                self.debug.show_sprite_boxes
            )
        {
            return;
        }
        #[inline]
        fn blend(over: u8, base: u8) -> u8 {
            if over > base { over } else { base }
        }

        // 1) BG tile grid (every 8 px in screen space), accounts for SCX/SCY wrapping per Pan Docs.
        if self.debug.show_bg_tile_grid {
            let scx = self.scx as usize;
            let scy = self.scy as usize;
            let shade = self.debug.shade_grid & 0b11;
            // vertical grid: columns where (SCX + x) % 8 == 0
            for x in 0..LCD_WIDTH {
                if ((scx + x) & 7) == 0 {
                    out[x] = blend(shade, out[x]);
                }
            }
            // horizontal grid: this line if (SCY + LY) % 8 == 0
            if ((scy + (ly as usize)) & 7) == 0 {
                for x in 0..LCD_WIDTH {
                    out[x] = blend(shade, out[x]);
                }
            }
        }

        // 2) Window bounds (WX-7 quirk and WY per Pan Docs). Draw top and left borders when window is enabled and in-range.
        if self.debug.show_window_bounds {
            let win_on = (self.lcdc & 0x20) != 0; // LCDC.5
            if win_on {
                let win_left = self.wx.wrapping_sub(7); // WX-7 quirk
                let wy = self.wy; // top of window
                let shade = self.debug.shade_window & 0b11;
                // Top border: when ly == WY, cover from max(0, win_left) to 159
                if ly == wy {
                    let start_x = if win_left >= 160 { 160 } else { win_left as usize };
                    for x in start_x..LCD_WIDTH {
                        out[x] = blend(shade, out[x]);
                    }
                }
                // Left border: for ly >= WY, single pixel at x = win_left
                if ly >= wy {
                    if win_left < 160 {
                        out[win_left as usize] = blend(shade, out[win_left as usize]);
                    }
                }
            }
        }

        // 3) BG axes: X= -SCX (mod 256), Y= -SCY (mod 256), if they fall on-screen this frame/scanline
        if self.debug.show_bg_axes {
            let shade = self.debug.shade_axes & 0b11;
            // vertical axis x_v = (-SCX) mod 256; draw if 0..159
            let x_v = ((256usize - (self.scx as usize)) & 255) as usize;
            if x_v < LCD_WIDTH {
                out[x_v] = blend(shade, out[x_v]);
            }
            // horizontal axis y_h = (-SCY) mod 256; draw if equals ly and 0..143
            let y_h = ((256usize - (self.scy as usize)) & 255) as u8;
            if y_h == ly {
                for x in 0..LCD_WIDTH {
                    out[x] = blend(shade, out[x]);
                }
            }
        }

        // 4) Sprite (OBJ) boxes: outline 8-wide (or 16-high) rectangles for any OAM entry intersecting this scanline
        if self.debug.show_sprite_boxes {
            let obj_8x16 = (self.lcdc & 0x04) != 0; // LCDC.2
            let obj_h: i16 = if obj_8x16 { 16 } else { 8 };
            let shade = self.debug.shade_sprite_box & 0b11;
            let ly_i16 = ly as i16;
            for i in 0..40 {
                let base = i * 4;
                let oam_y = self.oam[base] as i16;
                let oam_x = self.oam[base + 1] as i16;
                let y = oam_y - 16; // on-screen Y per Pan Docs
                let x = oam_x - 8; // on-screen X per Pan Docs
                // intersect this scanline?
                if ly_i16 < y || ly_i16 >= y + obj_h {
                    continue;
                }
                let left = x;
                let right = x + 7;
                // vertical borders at left and right
                if left >= 0 && left < (LCD_WIDTH as i16) {
                    let sx = left as usize;
                    out[sx] = blend(shade, out[sx]);
                }
                if right >= 0 && right < (LCD_WIDTH as i16) {
                    let sx = right as usize;
                    out[sx] = blend(shade, out[sx]);
                }
                // top border when this is the first visible line of the OBJ on-screen
                if ly_i16 == y {
                    let start = left.max(0) as usize;
                    let end = right.min((LCD_WIDTH as i16) - 1) as usize;
                    for sx in start..=end {
                        out[sx] = blend(shade, out[sx]);
                    }
                }
                // bottom border when last line
                if ly_i16 == y + obj_h - 1 {
                    let start = left.max(0) as usize;
                    let end = right.min((LCD_WIDTH as i16) - 1) as usize;
                    for sx in start..=end {
                        out[sx] = blend(shade, out[sx]);
                    }
                }
            }
        }
    }
}
