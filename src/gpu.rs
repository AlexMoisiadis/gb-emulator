// src/gpu.rs
use std::array::from_fn;

const VRAM_BEGIN: usize = 0x8000;
const VRAM_END: usize = 0x9fff;
const VRAM_SIZE: usize = VRAM_END - VRAM_BEGIN + 1;

type Tile = [[TilePixelValue; 8]; 8];

#[derive(Copy, Clone)]
pub enum TilePixelValue {
    Zero,
    One,
    Two,
    Three,
}

fn empty_tile() -> Tile {
    [[TilePixelValue::Zero; 8]; 8]
}

pub struct GPU {
    vram: [u8; VRAM_SIZE],
    tile_set: [Tile; 384],
}

impl GPU {
    pub fn new() -> Self {
        Self { vram: [0; VRAM_SIZE], tile_set: from_fn(|_| empty_tile()) }
    }

    #[inline]
    pub fn read_vram(&self, addr: usize) -> u8 {
        self.vram[addr]
    }

    pub fn write_vram(&mut self, index: usize, value: u8) {
        self.vram[index] = value;

        // Only tile area (first 0x1800 bytes) updates the tileset
        if index >= 0x1800 {
            return;
        }

        let normalised_index = index & 0xfffe;
        let byte1 = self.vram[normalised_index];
        let byte2 = self.vram[normalised_index + 1];

        let tile_index = normalised_index / 16;
        let row_index = (normalised_index % 16) / 2;

        if tile_index >= self.tile_set.len() {
            return;
        }

        for pixel_index in 0..8 {
            let mask = 1 << (7 - pixel_index);
            let lsb = (byte1 & mask) != 0;
            let msb = (byte2 & mask) != 0;
            let value = match (lsb, msb) {
                (true, true) => TilePixelValue::Three,
                (false, true) => TilePixelValue::Two,
                (true, false) => TilePixelValue::One,
                (false, false) => TilePixelValue::Zero,
            };
            self.tile_set[tile_index][row_index][pixel_index] = value;
        }
    }
}
