// src/apu/ch2.rs
//
// Square-wave channel 2 — identical to Ch1 but without sweep.
// All logic lives in PulseChannel; this struct is a thin wrapper.

use super::pulse::PulseChannel;

pub struct Ch2 {
    pub pulse: PulseChannel,
}

impl Ch2 {
    pub fn new() -> Self {
        Self { pulse: PulseChannel::new() }
    }

    pub fn tick(&mut self, tcycles: u32) {
        self.pulse.tick(tcycles);
    }
    pub fn clock_length(&mut self) -> bool {
        self.pulse.clock_length()
    }
    pub fn clock_envelope(&mut self) {
        self.pulse.clock_envelope();
    }
    pub fn sample(&self) -> u8 {
        self.pulse.sample()
    }
    pub fn enabled(&self) -> bool {
        self.pulse.active()
    }

    // Register reads
    pub fn read_nr21(&self) -> u8 {
        self.pulse.read_duty_length()
    }
    pub fn read_nr22(&self) -> u8 {
        self.pulse.read_envelope()
    }
    pub fn read_nr24(&self) -> u8 {
        self.pulse.read_freq_hi()
    }

    // Register writes
    pub fn write_nr21(&mut self, val: u8, apu_on: bool) {
        self.pulse.write_duty_length(val, apu_on);
    }
    pub fn write_nr22(&mut self, val: u8) {
        self.pulse.write_envelope(val);
    }
    pub fn write_nr23(&mut self, val: u8) {
        self.pulse.write_freq_lo(val);
    }
    /// Returns true if a trigger occurred.
    pub fn write_nr24(&mut self, val: u8, fs_step: u8) -> bool {
        self.pulse.write_freq_hi(val, fs_step)
    }
}
