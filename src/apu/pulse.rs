// src/apu/pulse.rs
//
// Shared state and logic for square-wave pulse channels (Ch1 and Ch2).
// Ch1 adds sweep on top; Ch2 uses this as its sole implementation.

/// Duty cycle waveforms — each is 8 steps, value 0 or 1.
pub const DUTY_TABLE: [[u8; 8]; 4] = [
    [0, 0, 0, 0, 0, 0, 0, 1], // 12.5%
    [1, 0, 0, 0, 0, 0, 0, 1], // 25%
    [1, 0, 0, 0, 1, 1, 1, 1], // 50%
    [0, 1, 1, 1, 1, 1, 1, 0], // 75%
];

pub struct PulseChannel {
    pub duty: u8,      // 0–3
    pub duty_pos: u8,  // 0–7, position in duty waveform
    pub freq: u16,     // 11-bit frequency register value
    pub freq_timer: u32, // T-cycle countdown

    pub length_counter: u8,
    pub length_enabled: bool,

    pub env_initial_vol: u8,
    pub env_add_mode: bool,
    pub env_period: u8,
    pub env_timer: u8,
    pub current_vol: u8,

    pub enabled: bool,
    pub dac_enabled: bool,
}

impl PulseChannel {
    pub fn new() -> Self {
        Self {
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
                // Pulse channels reload at (2048 - freq) * 4 T-cycles
                self.freq_timer = (2048 - self.freq as u32) * 4;
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

    // -------------------------------------------------------------------------
    // Sample output
    // -------------------------------------------------------------------------

    pub fn sample(&self) -> u8 {
        if !self.enabled || !self.dac_enabled {
            return 0;
        }
        DUTY_TABLE[self.duty as usize][self.duty_pos as usize] * self.current_vol
    }

    pub fn active(&self) -> bool {
        self.enabled && self.dac_enabled
    }

    // -------------------------------------------------------------------------
    // Register reads
    // -------------------------------------------------------------------------

    pub fn read_duty_length(&self) -> u8 {
        0x3f | (self.duty << 6)
    }

    pub fn read_envelope(&self) -> u8 {
        (self.env_initial_vol << 4)
            | (if self.env_add_mode { 0x08 } else { 0 })
            | self.env_period
    }

    pub fn read_freq_hi(&self) -> u8 {
        0xbf | (if self.length_enabled { 0x40 } else { 0 })
    }

    // -------------------------------------------------------------------------
    // Register writes
    // -------------------------------------------------------------------------

    /// NRx1: duty (only when APU on) + length counter (always writable).
    pub fn write_duty_length(&mut self, val: u8, apu_on: bool) {
        if apu_on {
            self.duty = (val >> 6) & 0x03;
        }
        self.length_counter = 64 - (val & 0x3f);
    }

    /// NRx2: envelope / DAC control.
    pub fn write_envelope(&mut self, val: u8) {
        self.env_initial_vol = (val >> 4) & 0x0f;
        self.env_add_mode = (val & 0x08) != 0;
        self.env_period = val & 0x07;
        self.dac_enabled = (val & 0xf8) != 0;
        if !self.dac_enabled {
            self.enabled = false;
        }
    }

    /// NRx3: low 8 bits of frequency.
    pub fn write_freq_lo(&mut self, val: u8) {
        self.freq = (self.freq & 0x0700) | (val as u16);
    }

    /// NRx4: high 3 bits of frequency + length enable + trigger.
    /// Calls `self.trigger()` internally if the trigger bit is set.
    /// Returns true if a trigger occurred (caller may need extra init, e.g. sweep).
    pub fn write_freq_hi(&mut self, val: u8) -> bool {
        self.freq = (self.freq & 0x00ff) | (((val & 0x07) as u16) << 8);
        self.length_enabled = (val & 0x40) != 0;
        if (val & 0x80) != 0 {
            self.trigger();
            return true;
        }
        false
    }

    /// Common trigger: enable channel, reload length/freq_timer/envelope.
    pub fn trigger(&mut self) {
        self.enabled = self.dac_enabled;
        if self.length_counter == 0 {
            self.length_counter = 64;
        }
        self.freq_timer = (2048 - self.freq as u32) * 4;
        self.env_timer = self.env_period;
        self.current_vol = self.env_initial_vol;
    }
}
