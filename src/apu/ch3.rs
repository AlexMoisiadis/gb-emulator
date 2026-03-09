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
        }
    }

    pub fn tick(&mut self, tcycles: u32) {
        if !self.enabled || !self.dac_enabled {
            return;
        }
        let mut remaining = tcycles;
        while remaining > 0 {
            // CH3 timer reloads at (2048 - freq) * 2
            let advance = remaining.min(self.freq_timer);
            self.freq_timer -= advance;
            remaining -= advance;
            if self.freq_timer == 0 {
                self.freq_timer = (2048 - (self.freq as u32)) * 2;
                self.wave_pos = (self.wave_pos + 1) & 31;
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
        self.wave_ram[(addr - 0xff30) as usize]
    }
    pub fn write_wave_ram(&mut self, addr: u16, val: u8) {
        self.wave_ram[(addr - 0xff30) as usize] = val;
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
    pub fn write_nr34(&mut self, val: u8) -> bool {
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
            self.length_counter = 256;
        }
        self.freq_timer = (2048 - (self.freq as u32)) * 2;
        self.wave_pos = 0;
    }
}
