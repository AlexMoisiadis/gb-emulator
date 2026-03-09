// src/apu/ch2.rs

const DUTY_TABLE: [[u8; 8]; 4] = [
    [0, 0, 0, 0, 0, 0, 0, 1],
    [1, 0, 0, 0, 0, 0, 0, 1],
    [1, 0, 0, 0, 1, 1, 1, 1],
    [0, 1, 1, 1, 1, 1, 1, 0],
];

pub struct Ch2 {
    duty: u8,
    duty_pos: u8,
    freq: u16,
    freq_timer: u32,
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

impl Ch2 {
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
        DUTY_TABLE[self.duty as usize][self.duty_pos as usize] * self.current_vol
    }

    pub fn enabled(&self) -> bool {
        self.enabled && self.dac_enabled
    }

    pub fn read_nr21(&self) -> u8 {
        0x3f | (self.duty << 6)
    }
    pub fn read_nr22(&self) -> u8 {
        (self.env_initial_vol << 4) | (if self.env_add_mode { 0x08 } else { 0 }) | self.env_period
    }
    pub fn read_nr24(&self) -> u8 {
        0xbf | (if self.length_enabled { 0x40 } else { 0 })
    }

    pub fn write_nr21(&mut self, val: u8, apu_on: bool) {
        if apu_on {
            self.duty = (val >> 6) & 0x03;
        }
        self.length_counter = 64 - (val & 0x3f);
    }
    pub fn write_nr22(&mut self, val: u8) {
        self.env_initial_vol = (val >> 4) & 0x0f;
        self.env_add_mode = (val & 0x08) != 0;
        self.env_period = val & 0x07;
        self.dac_enabled = (val & 0xf8) != 0;
        if !self.dac_enabled {
            self.enabled = false;
        }
    }
    pub fn write_nr23(&mut self, val: u8) {
        self.freq = (self.freq & 0x0700) | (val as u16);
    }
    pub fn write_nr24(&mut self, val: u8) -> bool {
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
    }
}
