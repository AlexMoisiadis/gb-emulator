// src/apu/ch1.rs

use super::pulse::PulseChannel;

pub struct Ch1 {
    pub pulse: PulseChannel,

    // Sweep (NR10)
    sweep_period: u8,
    sweep_negate: bool,
    sweep_shift: u8,
    sweep_timer: u8,
    sweep_shadow_freq: u16,
    sweep_enabled: bool,
    /// Set when a sweep calculation uses negate mode; cleared on trigger.
    /// Clearing NR10 negate after this disables the channel (DMG quirk).
    sweep_negate_used: bool,
}

impl Ch1 {
    pub fn new() -> Self {
        Self {
            pulse: PulseChannel::new(),
            sweep_period: 0,
            sweep_negate: false,
            sweep_shift: 0,
            sweep_timer: 0,
            sweep_shadow_freq: 0,
            sweep_enabled: false,
            sweep_negate_used: false,
        }
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
                    self.pulse.enabled = false;
                    return true;
                }
                if self.sweep_shift > 0 {
                    self.sweep_shadow_freq = new_freq;
                    self.pulse.freq = new_freq;
                    // Overflow check again after writing
                    if self.calc_sweep_freq() > 2047 {
                        self.pulse.enabled = false;
                        return true;
                    }
                }
            }
        }
        false
    }

    fn calc_sweep_freq(&mut self) -> u16 {
        let delta = self.sweep_shadow_freq >> self.sweep_shift;
        if self.sweep_negate {
            self.sweep_negate_used = true;
            self.sweep_shadow_freq.wrapping_sub(delta)
        } else {
            self.sweep_shadow_freq.wrapping_add(delta)
        }
    }

    pub fn sample(&self) -> u8 {
        self.pulse.sample()
    }

    pub fn enabled(&self) -> bool {
        self.pulse.active()
    }

    // -------------------------------------------------------------------------
    // Register reads
    // -------------------------------------------------------------------------

    pub fn read_nr10(&self) -> u8 {
        0x80
            | (self.sweep_period << 4)
            | (if self.sweep_negate { 0x08 } else { 0 })
            | self.sweep_shift
    }
    pub fn read_nr11(&self) -> u8 { self.pulse.read_duty_length() }
    pub fn read_nr12(&self) -> u8 { self.pulse.read_envelope() }
    pub fn read_nr14(&self) -> u8 { self.pulse.read_freq_hi() }

    // -------------------------------------------------------------------------
    // Register writes
    // -------------------------------------------------------------------------

    pub fn write_nr10(&mut self, val: u8) {
        let new_negate = (val & 0x08) != 0;
        // DMG quirk: clearing negate after it was used in a sweep calculation
        // disables the channel immediately.
        if self.sweep_negate && !new_negate && self.sweep_negate_used {
            self.pulse.enabled = false;
        }
        self.sweep_period = (val >> 4) & 0x07;
        self.sweep_negate = new_negate;
        self.sweep_shift = val & 0x07;
    }

    /// Length counter load is allowed even when APU is off.
    pub fn write_nr11(&mut self, val: u8, apu_on: bool) {
        self.pulse.write_duty_length(val, apu_on);
    }

    pub fn write_nr12(&mut self, val: u8) {
        self.pulse.write_envelope(val);
    }

    pub fn write_nr13(&mut self, val: u8) {
        self.pulse.write_freq_lo(val);
    }

    /// Returns true if a trigger occurred.
    pub fn write_nr14(&mut self, val: u8, fs_step: u8) -> bool {
        let triggered = self.pulse.write_freq_hi(val, fs_step);
        if triggered {
            self.init_sweep();
        }
        triggered
    }

    /// Sweep state initialised on trigger (after pulse trigger has run).
    fn init_sweep(&mut self) {
        self.sweep_shadow_freq = self.pulse.freq;
        let period = if self.sweep_period > 0 { self.sweep_period } else { 8 };
        self.sweep_timer = period;
        self.sweep_enabled = self.sweep_period > 0 || self.sweep_shift > 0;
        // Clear before the overflow check so that if negate is used here,
        // the flag persists — clearing NR10 negate after this must disable the channel.
        self.sweep_negate_used = false;
        if self.sweep_shift > 0 && self.calc_sweep_freq() > 2047 {
            self.pulse.enabled = false;
        }
    }
}
