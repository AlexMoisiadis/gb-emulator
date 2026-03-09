// src/apu/ch1.rs

/// Duty cycle waveforms — each is 8 steps, value 0 or 1.
const DUTY_TABLE: [[u8; 8]; 4] = [
    [0, 0, 0, 0, 0, 0, 0, 1], // 12.5%
    [1, 0, 0, 0, 0, 0, 0, 1], // 25%
    [1, 0, 0, 0, 1, 1, 1, 1], // 50%
    [0, 1, 1, 1, 1, 1, 1, 0], // 75%
];

pub struct Ch1 {
    // --- Sweep (NR10) ---
    sweep_period: u8,
    sweep_negate: bool,
    sweep_shift: u8,
    sweep_timer: u8,
    sweep_shadow_freq: u16,
    sweep_enabled: bool,

    // --- Duty / freq (NR11, NR13, NR14) ---
    duty: u8, // 0–3
    duty_pos: u8, // 0–7
    freq: u16, // 11-bit frequency register value
    freq_timer: u32, // T-cycle countdown

    // --- Length (NR11, NR14) ---
    length_counter: u8,
    length_enabled: bool,

    // --- Envelope (NR12) ---
    env_initial_vol: u8,
    env_add_mode: bool,
    env_period: u8,
    env_timer: u8,
    current_vol: u8,

    // --- State ---
    pub enabled: bool,
    dac_enabled: bool,
}

impl Ch1 {
    pub fn new() -> Self {
        Self {
            sweep_period: 0,
            sweep_negate: false,
            sweep_shift: 0,
            sweep_timer: 0,
            sweep_shadow_freq: 0,
            sweep_enabled: false,
            duty: 2,
            duty_pos: 0,
            freq: 0,
            freq_timer: 0,
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

    // -------------------------------------------------------------------------
    // Tick — advance frequency timer by tcycles
    // -------------------------------------------------------------------------
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
                self.freq_timer = (2048 - (self.freq as u32)) * 4;
                self.duty_pos = (self.duty_pos + 1) & 7;
            }
        }
    }

    // -------------------------------------------------------------------------
    // Frame-sequencer clocked units
    // -------------------------------------------------------------------------

    /// Returns true if the channel was silenced (length expired).
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

    /// Returns true if channel was disabled by frequency overflow.
    pub fn clock_sweep(&mut self) -> bool {
        if self.sweep_timer > 0 {
            self.sweep_timer -= 1;
        }
        if self.sweep_timer == 0 {
            let period = if self.sweep_period > 0 { self.sweep_period } else { 8 };
            self.sweep_timer = period;
            if self.sweep_enabled && self.sweep_period > 0 {
                let new_freq = self.calc_sweep_freq();
                if new_freq > 2047 {
                    self.enabled = false;
                    return true;
                }
                if self.sweep_shift > 0 {
                    self.sweep_shadow_freq = new_freq;
                    self.freq = new_freq;
                    // Overflow check again after writing
                    if self.calc_sweep_freq() > 2047 {
                        self.enabled = false;
                        return true;
                    }
                }
            }
        }
        false
    }

    fn calc_sweep_freq(&self) -> u16 {
        let delta = self.sweep_shadow_freq >> self.sweep_shift;
        if self.sweep_negate {
            self.sweep_shadow_freq.wrapping_sub(delta)
        } else {
            self.sweep_shadow_freq.wrapping_add(delta)
        }
    }

    // -------------------------------------------------------------------------
    // Sample output
    // -------------------------------------------------------------------------
    pub fn sample(&self) -> u8 {
        if !self.enabled || !self.dac_enabled {
            return 0;
        }
        DUTY_TABLE[self.duty as usize][self.duty_pos as usize] * self.current_vol
    }

    pub fn enabled(&self) -> bool {
        self.enabled && self.dac_enabled
    }

    // -------------------------------------------------------------------------
    // Register reads
    // -------------------------------------------------------------------------
    pub fn read_nr10(&self) -> u8 {
        0x80 |
            (self.sweep_period << 4) |
            (if self.sweep_negate { 0x08 } else { 0 }) |
            self.sweep_shift
    }
    pub fn read_nr11(&self) -> u8 {
        0x3f | (self.duty << 6)
    }
    pub fn read_nr12(&self) -> u8 {
        (self.env_initial_vol << 4) | (if self.env_add_mode { 0x08 } else { 0 }) | self.env_period
    }
    pub fn read_nr14(&self) -> u8 {
        0xbf | (if self.length_enabled { 0x40 } else { 0 })
    }

    // -------------------------------------------------------------------------
    // Register writes
    // -------------------------------------------------------------------------
    pub fn write_nr10(&mut self, val: u8) {
        self.sweep_period = (val >> 4) & 0x07;
        self.sweep_negate = (val & 0x08) != 0;
        self.sweep_shift = val & 0x07;
    }

    /// `apu_on`: length counter load is allowed even when APU is off.
    pub fn write_nr11(&mut self, val: u8, apu_on: bool) {
        if apu_on {
            self.duty = (val >> 6) & 0x03;
        }
        self.length_counter = 64 - (val & 0x3f);
    }

    pub fn write_nr12(&mut self, val: u8) {
        self.env_initial_vol = (val >> 4) & 0x0f;
        self.env_add_mode = (val & 0x08) != 0;
        self.env_period = val & 0x07;
        self.dac_enabled = (val & 0xf8) != 0;
        if !self.dac_enabled {
            self.enabled = false;
        }
    }

    pub fn write_nr13(&mut self, val: u8) {
        self.freq = (self.freq & 0x0700) | (val as u16);
    }

    /// Returns true if a trigger occurred.
    pub fn write_nr14(&mut self, val: u8) -> bool {
        self.freq = (self.freq & 0x00ff) | (((val & 0x07) as u16) << 8);
        self.length_enabled = (val & 0x40) != 0;
        if (val & 0x80) != 0 {
            self.trigger();
            return true;
        }
        false
    }

    fn trigger(&mut self) {
        self.enabled = self.dac_enabled;
        if self.length_counter == 0 {
            self.length_counter = 64;
        }
        self.freq_timer = (2048 - (self.freq as u32)) * 4;
        self.env_timer = self.env_period;
        self.current_vol = self.env_initial_vol;
        // Sweep init
        self.sweep_shadow_freq = self.freq;
        let period = if self.sweep_period > 0 { self.sweep_period } else { 8 };
        self.sweep_timer = period;
        self.sweep_enabled = self.sweep_period > 0 || self.sweep_shift > 0;
        // Overflow check on trigger
        if self.sweep_shift > 0 && self.calc_sweep_freq() > 2047 {
            self.enabled = false;
        }
    }
}
