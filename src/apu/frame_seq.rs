// src/apu/frame_seq.rs

/// Fired events returned by `FrameSequencer::tick()`.
#[derive(Default)]
pub struct FsEvents {
    pub clock_length: bool,
    pub clock_envelope: bool,
    pub clock_sweep: bool,
}

/// 512 Hz frame sequencer — steps 0–7, each step = 8192 T-cycles.
/// Drives length counters, envelope, and CH1 sweep at the correct rates.
pub struct FrameSequencer {
    /// T-cycle countdown to the next step.
    timer: u32,
    /// Current step (0–7).
    step: u8,
}

impl FrameSequencer {
    pub fn new() -> Self {
        Self {
            timer: 8192,
            step: 0,
        }
    }

    /// Advance by `tcycles` T-cycles.  Returns which units fired.
    pub fn tick(&mut self, tcycles: u32) -> FsEvents {
        let mut ev = FsEvents::default();

        // The timer can tick multiple times if tcycles is large, though in
        // practice it's always ≤ 24 (one instruction worth).
        let mut remaining = tcycles;
        while remaining > 0 {
            let advance = remaining.min(self.timer);
            self.timer -= advance;
            remaining -= advance;

            if self.timer == 0 {
                self.timer = 8192;
                self.fire(&mut ev);
                self.step = (self.step + 1) & 7;
            }
        }
        ev
    }

    fn fire(&self, ev: &mut FsEvents) {
        // Step:  0  1  2  3  4  5  6  7
        // Len:   ✓     ✓     ✓     ✓
        // Sweep:       ✓           ✓
        // Env:                           ✓  (step 7)
        match self.step {
            0 => {
                ev.clock_length = true;
            }
            1 => {}
            2 => {
                ev.clock_length = true;
                ev.clock_sweep = true;
            }
            3 => {}
            4 => {
                ev.clock_length = true;
            }
            5 => {}
            6 => {
                ev.clock_length = true;
                ev.clock_sweep = true;
            }
            7 => {
                ev.clock_envelope = true;
            }
            _ => unreachable!(),
        }
    }
}
