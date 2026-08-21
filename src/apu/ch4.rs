// src/apu/ch4.rs

/// Divisor table for NR43 bits 2–0.
const DIVISORS: [u32; 8] = [8, 16, 32, 48, 64, 80, 96, 112];

pub struct Ch4 {
    freq_timer: u32,
    clock_shift: u8,
    lfsr_width: bool, // false=15-bit, true=7-bit
    divisor_code: u8,
    lfsr: u16, // 15-bit shift register
    length_counter: u8,
    length_enabled: bool,
    env_initial_vol: u8,
    env_add_mode: bool,
    env_period: u8,
    env_timer: u8,
    current_vol: u8,
    pub enabled: bool,
    dac_enabled: bool,
}

impl Ch4 {
    pub fn new() -> Self {
        Self {
            freq_timer: 0,
            clock_shift: 0,
            lfsr_width: false,
            divisor_code: 0,
            lfsr: 0x7fff,
            length_counter: 0,
            length_enabled: false,
            env_initial_vol: 0,
            env_add_mode: false,
            env_period: 0,
            env_timer: 0,
            current_vol: 0,
            enabled: false,
            dac_enabled: false,
        }
    }

    pub fn tick(&mut self, tcycles: u32) {
        if !self.enabled || !self.dac_enabled {
            return;
        }
        let mut remaining = tcycles;
        while remaining > 0 {
            let advance = remaining.min(self.freq_timer);
            self.freq_timer -= advance;
            remaining -= advance;
            if self.freq_timer == 0 {
                self.reload_timer();
                // Clock LFSR
                let xor_bit = (self.lfsr & 1) ^ ((self.lfsr >> 1) & 1);
                self.lfsr >>= 1;
                self.lfsr |= xor_bit << 14;
                if self.lfsr_width {
                    // 7-bit mode: also write to bit 6
                    self.lfsr = (self.lfsr & !0x40) | (xor_bit << 6);
                }
            }
        }
    }

    fn reload_timer(&mut self) {
        let divisor = DIVISORS[self.divisor_code as usize];
        self.freq_timer = divisor << self.clock_shift;
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

    pub fn clock_envelope(&mut self) {
        if self.env_period == 0 {
            return;
        }
        if self.env_timer > 0 {
            self.env_timer -= 1;
        }
        if self.env_timer == 0 {
            self.env_timer = self.env_period;
            if self.env_add_mode && self.current_vol < 15 {
                self.current_vol += 1;
            } else if !self.env_add_mode && self.current_vol > 0 {
                self.current_vol -= 1;
            }
        }
    }

    pub fn sample(&self) -> u8 {
        if !self.enabled || !self.dac_enabled {
            return 0;
        }
        // Output is the inverse of LFSR bit 0
        if (self.lfsr & 1) == 0 {
            self.current_vol
        } else {
            0
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled && self.dac_enabled
    }

    pub fn read_nr42(&self) -> u8 {
        (self.env_initial_vol << 4) | (if self.env_add_mode { 0x08 } else { 0 }) | self.env_period
    }
    pub fn read_nr43(&self) -> u8 {
        (self.clock_shift << 4) | (if self.lfsr_width { 0x08 } else { 0 }) | self.divisor_code
    }
    pub fn read_nr44(&self) -> u8 {
        0xbf | (if self.length_enabled { 0x40 } else { 0 })
    }

    pub fn write_nr41(&mut self, val: u8, _apu_on: bool) {
        self.length_counter = 64 - (val & 0x3f);
    }
    pub fn write_nr42(&mut self, val: u8) {
        self.env_initial_vol = (val >> 4) & 0x0f;
        self.env_add_mode = (val & 0x08) != 0;
        self.env_period = val & 0x07;
        self.dac_enabled = (val & 0xf8) != 0;
        if !self.dac_enabled {
            self.enabled = false;
        }
    }
    pub fn write_nr43(&mut self, val: u8) {
        self.clock_shift = (val >> 4) & 0x0f;
        self.lfsr_width = (val & 0x08) != 0;
        self.divisor_code = val & 0x07;
    }
    pub fn write_nr44(&mut self, val: u8, fs_step: u8) -> bool {
        let prev_length_enabled = self.length_enabled;
        self.length_enabled = (val & 0x40) != 0;
        // 0→1 enable clock fires BEFORE trigger sequence.
        if !prev_length_enabled && self.length_enabled && (fs_step & 1 == 1) {
            #[cfg(feature = "trace_apu")]
            eprintln!("[CH4] 0->1 len_en clock: len_before={}", self.length_counter);
            self.clock_length();
        }
        if (val & 0x80) != 0 {
            #[cfg(feature = "trace_apu")]
            eprintln!("[CH4] trigger fs_step={} len={} len_en={}", fs_step, self.length_counter, self.length_enabled);
            self.trigger(fs_step);
            #[cfg(feature = "trace_apu")]
            eprintln!("[CH4] trigger done: len={} enabled={}", self.length_counter, self.enabled);
            return true;
        }
        false
    }

    fn trigger(&mut self, fs_step: u8) {
        let is_first_half = fs_step & 1 == 1;
        // Reload-only extra clock; see PulseChannel::trigger.
        if self.length_counter == 0 {
            #[cfg(feature = "trace_apu")]
            eprintln!("[CH4] trigger reload len 0->64");
            self.length_counter = 64;
            if self.length_enabled && is_first_half {
                #[cfg(feature = "trace_apu")]
                eprintln!("[CH4] trigger extra clock on reload: 64->63");
                self.length_counter -= 1;
            }
        }
        self.enabled = self.dac_enabled;
        self.reload_timer();
        self.env_timer = self.env_period;
        self.current_vol = self.env_initial_vol;
        self.lfsr = 0x7fff;
    }
}
