// src/apu/mod.rs

pub mod ch1;
pub mod ch2;
pub mod ch3;
pub mod ch4;
pub mod frame_seq;
pub mod mixer;
pub mod output;
pub mod pulse;

use ch1::Ch1;
use ch2::Ch2;
use ch3::Ch3;
use ch4::Ch4;
use frame_seq::FrameSequencer;

/// Master APU.  Wire into MemoryBus and call `tick()` each CPU step.
pub struct Apu {
    pub ch1: Ch1,
    pub ch2: Ch2,
    pub ch3: Ch3,
    pub ch4: Ch4,

    frame_seq: FrameSequencer,

    // ---- NR50 / NR51 / NR52 ------------------------------------------------
    /// NR50 (FF24): Vin panning + master volume
    ///   Bit 7:   Vin → left  (unused on DMG)
    ///   Bits 6–4: left  master volume (0–7)
    ///   Bit 3:   Vin → right (unused on DMG)
    ///   Bits 2–0: right master volume (0–7)
    nr50: u8,

    /// NR51 (FF25): channel → speaker panning
    ///   Bit 7: CH4 left   Bit 3: CH4 right
    ///   Bit 6: CH3 left   Bit 2: CH3 right
    ///   Bit 5: CH2 left   Bit 1: CH2 right
    ///   Bit 4: CH1 left   Bit 0: CH1 right
    nr51: u8,

    /// NR52 (FF26):
    ///   Bit 7 R/W: master enable (0 = APU off, all registers reset)
    ///   Bits 3–0 R:  channel enabled flags (set by trigger, cleared by length)
    nr52: u8,

    // ---- Sample generation -------------------------------------------------
    /// Target host sample rate (e.g. 44100.0 or 48000.0)

    /// Fractional T-cycle accumulator; when ≥ threshold we emit a sample.
    sample_accum: f32,

    /// T-cycles per sample = 4_194_304 / sample_rate
    cycles_per_sample: f32,

    /// Filled by tick(), drained by the audio thread via `drain_samples()`.
    /// Interleaved stereo: [L, R, L, R, …]  each sample in –1.0 … +1.0
    pub sample_buffer: Vec<f32>,
}

impl Apu {
    // -------------------------------------------------------------------------
    // Construction
    // -------------------------------------------------------------------------

    pub fn new(sample_rate: f32) -> Self {
        Self {
            ch1: Ch1::new(),
            ch2: Ch2::new(),
            ch3: Ch3::new(),
            ch4: Ch4::new(),
            frame_seq: FrameSequencer::new(),
            nr50: 0x77, // both master volumes at max after boot
            nr51: 0xf3, // DMG boot ROM leaves CH1/CH2/CH3/CH4 routed right, CH1/CH2 left
            nr52: 0xf1, // APU on, CH1 active
            sample_accum: 0.0,
            cycles_per_sample: 4_194_304.0 / sample_rate,
            sample_buffer: Vec::with_capacity(4096),
        }
    }

    // -------------------------------------------------------------------------
    // Main tick — called from bus.rs once per CPU instruction
    // -------------------------------------------------------------------------

    pub fn tick(&mut self, tcycles: u32) {
        // When the master enable bit is clear the APU is completely off.
        if (self.nr52 & 0x80) == 0 {
            // Still advance the sample accumulator so the audio thread stays
            // in sync, but output silence.
            self.push_silence(tcycles);
            return;
        }

        // 1) Clock the frame sequencer; it returns which units fired this step.
        let fs = self.frame_seq.tick(tcycles);

        // 2) Tick all channel frequency timers.
        self.ch1.tick(tcycles);
        self.ch2.tick(tcycles);
        self.ch3.tick(tcycles);
        self.ch4.tick(tcycles);

        // 3) Frame-sequencer triggered units.
        if fs.clock_length {
            #[cfg(feature = "trace_apu")]
            eprintln!("[APU] FS clock_length (step={})", self.frame_seq.step().wrapping_sub(1) & 7);
            if self.ch1.clock_length() {
                self.nr52 &= !0x01;
                #[cfg(feature = "trace_apu")]
                eprintln!("[APU] ch1 silenced by length");
            }
            if self.ch2.clock_length() {
                self.nr52 &= !0x02;
                #[cfg(feature = "trace_apu")]
                eprintln!("[APU] ch2 silenced by length");
            }
            if self.ch3.clock_length() {
                self.nr52 &= !0x04;
                #[cfg(feature = "trace_apu")]
                eprintln!("[APU] ch3 silenced by length");
            }
            if self.ch4.clock_length() {
                self.nr52 &= !0x08;
                #[cfg(feature = "trace_apu")]
                eprintln!("[APU] ch4 silenced by length");
            }
        }
        if fs.clock_envelope {
            self.ch1.clock_envelope();
            self.ch2.clock_envelope();
            self.ch4.clock_envelope();
            // CH3 has no envelope
        }
        if fs.clock_sweep {
            // clock_sweep returns true if the channel was disabled by overflow
            if self.ch1.clock_sweep() {
                self.nr52 &= !0x01;
            }
        }

        // 4) Emit samples at the host sample rate.
        self.sample_accum += tcycles as f32;
        while self.sample_accum >= self.cycles_per_sample {
            self.sample_accum -= self.cycles_per_sample;
            self.emit_sample();
        }
    }

    // -------------------------------------------------------------------------
    // Register access — called from mmu.rs for 0xFF10–0xFF3F
    // -------------------------------------------------------------------------

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            // ---- CH1 (NR10–NR14) -------------------------------------------
            0xff10 => self.ch1.read_nr10(),
            0xff11 => self.ch1.read_nr11(),
            0xff12 => self.ch1.read_nr12(),
            0xff13 => 0xff, // NR13 write-only
            0xff14 => self.ch1.read_nr14(),

            // ---- CH2 (NR20–NR24) — NR20 unused ----------------------------
            0xff15 => 0xff,
            0xff16 => self.ch2.read_nr21(),
            0xff17 => self.ch2.read_nr22(),
            0xff18 => 0xff, // NR23 write-only
            0xff19 => self.ch2.read_nr24(),

            // ---- CH3 (NR30–NR34) -------------------------------------------
            0xff1a => self.ch3.read_nr30(),
            0xff1b => 0xff, // NR31 write-only
            0xff1c => self.ch3.read_nr32(),
            0xff1d => 0xff, // NR33 write-only
            0xff1e => self.ch3.read_nr34(),

            // ---- CH4 (NR40–NR44) — NR40 unused ----------------------------
            0xff1f => 0xff,
            0xff20 => 0xff, // NR41 write-only
            0xff21 => self.ch4.read_nr42(),
            0xff22 => self.ch4.read_nr43(),
            0xff23 => self.ch4.read_nr44(),

            // ---- Mixer -----------------------------------------------------
            0xff24 => self.nr50,
            0xff25 => self.nr51,

            // NR52: top bit R/W, bits 3–0 read-only channel status, rest 1
            0xff26 => {
                let ch_bits =
                    (self.ch1.enabled() as u8) |
                    ((self.ch2.enabled() as u8) << 1) |
                    ((self.ch3.enabled() as u8) << 2) |
                    ((self.ch4.enabled() as u8) << 3);
                let result = (self.nr52 & 0x80) | 0x70 | ch_bits;
                #[cfg(feature = "trace_apu")]
                eprintln!("[APU] NR52 read={:#04x} (ch1={} ch2={} ch3={} ch4={})",
                    result, self.ch1.enabled(), self.ch2.enabled(), self.ch3.enabled(), self.ch4.enabled());
                result
            }

            // ---- Wave RAM (FF30–FF3F) ---------------------------------------
            0xff30..=0xff3f => self.ch3.read_wave_ram(addr),

            _ => 0xff,
        }
    }

    pub fn write(&mut self, addr: u16, val: u8) {
        // Writes to FF11–FF25 are ignored while the APU is off, EXCEPT the
        // length counters (FF11/FF16/FF1B/FF20) which are always writable.
        let apu_on = (self.nr52 & 0x80) != 0;

        match addr {
            // ---- CH1 -------------------------------------------------------
            0xff10 => {
                if apu_on {
                    self.ch1.write_nr10(val);
                }
            }
            0xff11 => {
                self.ch1.write_nr11(val, apu_on);
            } // length always writable
            0xff12 => {
                if apu_on {
                    self.ch1.write_nr12(val);
                }
            }
            0xff13 => {
                if apu_on {
                    self.ch1.write_nr13(val);
                }
            }
            0xff14 => {
                if apu_on {
                    #[cfg(feature = "trace_apu")]
                    eprintln!("[APU] NR14 write={:#04x} fs_step={} len_en={} trigger={}", val, self.frame_seq.step(), (val & 0x40) != 0, (val & 0x80) != 0);
                    let triggered = self.ch1.write_nr14(val, self.frame_seq.step());
                    if triggered {
                        self.nr52 |= 0x01;
                    }
                    #[cfg(feature = "trace_apu")]
                    eprintln!("[APU] NR14 after: ch1.enabled={}", self.ch1.enabled());
                }
            }

            // ---- CH2 -------------------------------------------------------
            0xff15 => {} // unused
            0xff16 => {
                self.ch2.write_nr21(val, apu_on);
            }
            0xff17 => {
                if apu_on {
                    self.ch2.write_nr22(val);
                }
            }
            0xff18 => {
                if apu_on {
                    self.ch2.write_nr23(val);
                }
            }
            0xff19 => {
                if apu_on {
                    #[cfg(feature = "trace_apu")]
                    eprintln!("[APU] NR24 write={:#04x} fs_step={} len_en={} trigger={}", val, self.frame_seq.step(), (val & 0x40) != 0, (val & 0x80) != 0);
                    let triggered = self.ch2.write_nr24(val, self.frame_seq.step());
                    if triggered {
                        self.nr52 |= 0x02;
                    }
                    #[cfg(feature = "trace_apu")]
                    eprintln!("[APU] NR24 after: ch2.enabled={}", self.ch2.enabled());
                }
            }

            // ---- CH3 -------------------------------------------------------
            0xff1a => {
                if apu_on {
                    self.ch3.write_nr30(val);
                }
            }
            0xff1b => {
                self.ch3.write_nr31(val, apu_on);
            }
            0xff1c => {
                if apu_on {
                    self.ch3.write_nr32(val);
                }
            }
            0xff1d => {
                if apu_on {
                    self.ch3.write_nr33(val);
                }
            }
            0xff1e => {
                if apu_on {
                    #[cfg(feature = "trace_apu")]
                    eprintln!("[APU] NR34 write={:#04x} fs_step={} len_en={} trigger={}", val, self.frame_seq.step(), (val & 0x40) != 0, (val & 0x80) != 0);
                    let triggered = self.ch3.write_nr34(val, self.frame_seq.step());
                    if triggered {
                        self.nr52 |= 0x04;
                    }
                    #[cfg(feature = "trace_apu")]
                    eprintln!("[APU] NR34 after: ch3.enabled={}", self.ch3.enabled());
                }
            }

            // ---- CH4 -------------------------------------------------------
            0xff1f => {} // unused
            0xff20 => {
                self.ch4.write_nr41(val, apu_on);
            }
            0xff21 => {
                if apu_on {
                    self.ch4.write_nr42(val);
                }
            }
            0xff22 => {
                if apu_on {
                    self.ch4.write_nr43(val);
                }
            }
            0xff23 => {
                if apu_on {
                    #[cfg(feature = "trace_apu")]
                    eprintln!("[APU] NR44 write={:#04x} fs_step={} len_en={} trigger={}", val, self.frame_seq.step(), (val & 0x40) != 0, (val & 0x80) != 0);
                    let triggered = self.ch4.write_nr44(val, self.frame_seq.step());
                    if triggered {
                        self.nr52 |= 0x08;
                    }
                    #[cfg(feature = "trace_apu")]
                    eprintln!("[APU] NR44 after: ch4.enabled={}", self.ch4.enabled());
                }
            }

            // ---- Mixer -----------------------------------------------------
            0xff24 => {
                if apu_on {
                    self.nr50 = val;
                }
            }
            0xff25 => {
                if apu_on {
                    self.nr51 = val;
                }
            }

            // NR52 — only bit 7 is writable; writing 0 resets everything
            0xff26 => {
                let was_on = (self.nr52 & 0x80) != 0;
                let now_on = (val & 0x80) != 0;
                #[cfg(feature = "trace_apu")]
                eprintln!("[APU] NR52 write={:#04x} was_on={} now_on={}", val, was_on, now_on);
                self.nr52 = (self.nr52 & 0x7f) | (val & 0x80);
                if was_on && !now_on {
                    self.power_off_reset();
                }
            }

            // ---- Wave RAM --------------------------------------------------
            0xff30..=0xff3f => self.ch3.write_wave_ram(addr, val),

            _ => {}
        }
    }

    // -------------------------------------------------------------------------
    // Audio thread interface
    // -------------------------------------------------------------------------

    /// Called when the CPU writes to DIV (0xFF04) — resets the frame sequencer
    /// timer, since on DMG hardware the FS is driven by the same internal counter.
    pub fn div_reset(&mut self) {
        self.frame_seq.div_reset();
    }

    /// Drain pending interleaved stereo samples into `out`.
    /// The audio callback should call this and write the result to its buffer.
    pub fn drain_samples(&mut self, out: &mut Vec<f32>) {
        out.extend_from_slice(&self.sample_buffer);
        self.sample_buffer.clear();
    }

    // -------------------------------------------------------------------------
    // Private helpers
    // -------------------------------------------------------------------------

    /// Mix and push one stereo sample pair.
    fn emit_sample(&mut self) {
        // Raw 0–15 output from each channel (0 when disabled / DAC off).
        let samples: [f32; 4] = [
            if self.ch1.enabled() { self.ch1.sample() as f32 } else { 0.0 },
            if self.ch2.enabled() { self.ch2.sample() as f32 } else { 0.0 },
            if self.ch3.enabled() { self.ch3.sample() as f32 } else { 0.0 },
            if self.ch4.enabled() { self.ch4.sample() as f32 } else { 0.0 },
        ];

        // NR51 panning: left = bits 7–4 (ch4..ch1), right = bits 3–0 (ch4..ch1).
        const LEFT_BITS:  [u8; 4] = [0x10, 0x20, 0x40, 0x80];
        const RIGHT_BITS: [u8; 4] = [0x01, 0x02, 0x04, 0x08];

        let nr51 = self.nr51;
        let left_panned:  [f32; 4] = std::array::from_fn(|i| if (nr51 & LEFT_BITS[i])  != 0 { samples[i] } else { 0.0 });
        let right_panned: [f32; 4] = std::array::from_fn(|i| if (nr51 & RIGHT_BITS[i]) != 0 { samples[i] } else { 0.0 });

        let left  = mixer::mix(left_panned[0],  left_panned[1],  left_panned[2],  left_panned[3],  (self.nr50 >> 4) & 0x07);
        let right = mixer::mix(right_panned[0], right_panned[1], right_panned[2], right_panned[3], self.nr50 & 0x07);

        self.sample_buffer.push(left);
        self.sample_buffer.push(right);
    }

    /// Push silence samples without mixing (APU is off).
    fn push_silence(&mut self, tcycles: u32) {
        self.sample_accum += tcycles as f32;
        while self.sample_accum >= self.cycles_per_sample {
            self.sample_accum -= self.cycles_per_sample;
            self.sample_buffer.push(0.0);
            self.sample_buffer.push(0.0);
        }
    }

    /// Reset all registers and channel state when NR52 bit 7 is written 0.
    fn power_off_reset(&mut self) {
        self.ch1 = Ch1::new();
        self.ch2 = Ch2::new();
        self.ch3.reset_registers(); // wave RAM is preserved on DMG power-off
        self.ch4 = Ch4::new();
        self.frame_seq = FrameSequencer::new();
        self.nr50 = 0;
        self.nr51 = 0;
        // nr52 retains the 0x80=0 bit (APU off); channel bits are cleared
        self.nr52 = 0;
    }
}
