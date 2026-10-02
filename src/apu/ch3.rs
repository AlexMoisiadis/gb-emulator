// src/apu/ch3.rs

pub struct Ch3 {
    dac_enabled: bool,
    freq: u16,
    freq_timer: u32,
    wave_pos: u8, // 0–31 nibble index
    wave_ram: [u8; 16],
    output_level: u8, // 0=mute, 1=100%, 2=50%, 3=25%
    length_counter: u16, // CH3 uses 256-step length
    length_enabled: bool,
    pub enabled: bool,
    /// T-cycles left in the window during which the CPU may reach wave RAM
    /// while the channel is active (DMG). 0 = reads give 0xFF, writes drop.
    wave_access_ttl: u8,
}

impl Ch3 {
    pub fn new() -> Self {
        Self {
            dac_enabled: false,
            freq: 0,
            freq_timer: 0,
            wave_pos: 0,
            wave_ram: [0u8; 16],
            output_level: 0,
            length_counter: 0,
            length_enabled: false,
            enabled: false,
            wave_access_ttl: 0,
        }
    }

    /// Reset on APU power-off. DMG preserves wave RAM AND the length counter
    /// across power-cycle.
    pub fn power_off_reset(&mut self) {
        self.dac_enabled = false;
        self.freq = 0;
        self.freq_timer = 0;
        self.wave_pos = 0;
        self.output_level = 0;
        self.length_enabled = false;
        self.enabled = false;
        self.wave_access_ttl = 0;
        // wave_ram and length_counter intentionally NOT cleared
    }

    pub fn tick(&mut self, tcycles: u32) {
        if !self.enabled || !self.dac_enabled {
            self.wave_access_ttl = 0;
            return;
        }
        let mut remaining = tcycles;
        while remaining > 0 {
            // CH3 timer reloads at (2048 - freq) * 2
            let advance = remaining.min(self.freq_timer).max(1);
            self.freq_timer = self.freq_timer.saturating_sub(advance);
            remaining = remaining.saturating_sub(advance);
            // Age the window before opening a new one, so a fetch landing at
            // the end of this chunk keeps its full lifetime.
            self.wave_access_ttl = self.wave_access_ttl.saturating_sub(advance.min(255) as u8);
            if self.freq_timer == 0 {
                self.freq_timer = (2048 - (self.freq as u32)) * 2;
                self.wave_pos = (self.wave_pos + 1) & 31;
                self.wave_access_ttl = 2;
            }
        }
    }

    pub fn clock_length(&mut self) -> bool {
        if self.length_enabled && self.length_counter > 0 {
            self.length_counter -= 1;
            if self.length_counter == 0 {
                self.enabled = false;
                return true;
            }
        }
        false
    }

    pub fn sample(&self) -> u8 {
        if !self.enabled || !self.dac_enabled {
            return 0;
        }
        let byte = self.wave_ram[(self.wave_pos / 2) as usize];
        let nibble = if (self.wave_pos & 1) == 0 { byte >> 4 } else { byte & 0x0f };
        // output_level: 0=mute, 1=100%, 2=50%, 3=25%
        match self.output_level {
            0 => 0,
            1 => nibble,
            2 => nibble >> 1,
            3 => nibble >> 2,
            _ => 0,
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled && self.dac_enabled
    }

    // ---- Wave RAM -----------------------------------------------------------
    pub fn read_wave_ram(&self, addr: u16) -> u8 {
        if self.enabled && self.dac_enabled {
            // DMG: while CH3 is active wave RAM is reachable only during the
            // ~2 T-cycle window in which the channel latches a sample byte.
            // Outside it the bus floats and reads return 0xFF.
            if self.wave_access_ttl == 0 {
                return 0xff;
            }
            self.wave_ram[(self.wave_pos / 2) as usize]
        } else {
            self.wave_ram[(addr - 0xff30) as usize]
        }
    }
    pub fn write_wave_ram(&mut self, addr: u16, val: u8) {
        if self.enabled && self.dac_enabled {
            // DMG: outside the latch window the write is dropped entirely;
            // inside it, it is redirected to the byte being fetched.
            if self.wave_access_ttl == 0 {
                return;
            }
            self.wave_ram[(self.wave_pos / 2) as usize] = val;
        } else {
            self.wave_ram[(addr - 0xff30) as usize] = val;
        }
    }

    // ---- Register reads -----------------------------------------------------
    pub fn read_nr30(&self) -> u8 {
        0x7f | (if self.dac_enabled { 0x80 } else { 0 })
    }
    pub fn read_nr32(&self) -> u8 {
        0x9f | ((self.output_level & 0x03) << 5)
    }
    pub fn read_nr34(&self) -> u8 {
        0xbf | (if self.length_enabled { 0x40 } else { 0 })
    }

    // ---- Register writes ----------------------------------------------------
    pub fn write_nr30(&mut self, val: u8) {
        self.dac_enabled = (val & 0x80) != 0;
        if !self.dac_enabled {
            self.enabled = false;
        }
    }
    pub fn write_nr31(&mut self, val: u8, _apu_on: bool) {
        self.length_counter = 256 - (val as u16);
    }
    pub fn write_nr32(&mut self, val: u8) {
        self.output_level = (val >> 5) & 0x03;
    }
    pub fn write_nr33(&mut self, val: u8) {
        self.freq = (self.freq & 0x0700) | (val as u16);
    }
    pub fn write_nr34(&mut self, val: u8, fs_step: u8) -> bool {
        self.freq = (self.freq & 0x00ff) | (((val & 0x07) as u16) << 8);
        let prev_length_enabled = self.length_enabled;
        self.length_enabled = (val & 0x40) != 0;
        // 0→1 enable clock fires BEFORE trigger sequence.
        if !prev_length_enabled && self.length_enabled && (fs_step & 1 == 1) {
            #[cfg(feature = "trace_apu")]
            eprintln!("[CH3] 0->1 len_en clock: len_before={}", self.length_counter);
            self.clock_length();
        }
        if (val & 0x80) != 0 {
            #[cfg(feature = "trace_apu")]
            eprintln!("[CH3] trigger fs_step={} len={} len_en={}", fs_step, self.length_counter, self.length_enabled);
            self.trigger(fs_step);
            #[cfg(feature = "trace_apu")]
            eprintln!("[CH3] trigger done: len={} enabled={}", self.length_counter, self.enabled);
            return true;
        }
        false
    }

    fn trigger(&mut self, fs_step: u8) {
        // DMG: re-triggering while active corrupts wave RAM, but only when the
        // trigger lands in the 2 T-cycles *before* a sample fetch — offset by
        // 2 T from the read/write window, which covers the 2 T after a fetch
        // (see wave_access_ttl). The bytes copied are the ones the upcoming
        // fetch is about to address.
        if self.enabled && self.dac_enabled && self.freq_timer <= 2 {
            let pos = (self.wave_pos + 1) & 31;
            let offset = ((pos >> 1) as usize) & 0x0f;
            if offset < 4 {
                self.wave_ram[0] = self.wave_ram[offset];
            } else {
                // Beyond byte 3 the whole aligned 4-byte block is copied down.
                let base = offset & !3;
                for i in 0..4 {
                    self.wave_ram[i] = self.wave_ram[base + i];
                }
            }
        }
        let is_first_half = fs_step & 1 == 1;
        // Reload-only extra clock; see PulseChannel::trigger.
        if self.length_counter == 0 {
            #[cfg(feature = "trace_apu")]
            eprintln!("[CH3] trigger reload len 0->256");
            self.length_counter = 256;
            if self.length_enabled && is_first_half {
                #[cfg(feature = "trace_apu")]
                eprintln!("[CH3] trigger extra clock on reload: 256->255");
                self.length_counter -= 1;
            }
        }
        self.enabled = self.dac_enabled;
        // Advance-then-latch convention: the first sample fetch sits 6 T-cycles
        // after the trigger. Must stay paired with the `wave_pos >> 1`
        // corruption source below — a mixed pair passes 09 and fails 10.
        self.freq_timer = (2048 - (self.freq as u32)) * 2 + 6;
        self.wave_pos = 0;
        self.wave_access_ttl = 0;
    }
}
