// src/timer.rs

/// Game Boy timer block: DIV/TIMA/TMA/TAC with **edge-based increments**
/// and DMG quirks: DIV/TAC "glitch" ticks, delayed TMA reload, and
/// write-to-TIMA cancel window (1 M-cycle).
///
/// References:
/// - Pan Docs: Timer & Divider, Obscure Behaviour.  [1](https://gbdev.gg8.se/wiki/articles/Timer_and_Divider_Registers)[2](https://gbdev.io/pandocs/Timer_Obscure_Behaviour.html)
/// - GbdevWiki mirror (same content).              [3](https://gbdev.gg8.se/wiki/articles/Timer_Obscure_Behaviour)
pub struct Timer {
    // Public-facing registers
    divider: u8, // FF04 (visible DIV = high 8 bits of system counter)
    tima: u8, // FF05
    tma: u8, // FF06
    tac: u8, // FF07 (bits 2..0 used; 7..3 read as 1)

    // Internal state
    div_counter: u16, // 16-bit system counter (increments every M-cycle)
    timer_enabled: bool, // TAC.2
    timer_bit: u8, // selected source bit in div_counter: {9,3,5,7}
    prev_timer_bit_state: bool, // previous sampled bit for falling-edge detection

    // TIMA overflow quirk state (1 M-cycle delayed reload & IF)
    overflow_pending: bool, // an overflow occurred last cycle
    overflow_delay: u8, // counts down M-cycles until reload/IF (1 on DMG)
}

impl Timer {
    pub fn new() -> Self {
        Self {
            divider: 0,
            tima: 0,
            tma: 0,
            tac: 0,
            div_counter: 0,
            timer_enabled: false,
            timer_bit: 9, // 00 -> 4096 Hz
            prev_timer_bit_state: false,
            overflow_pending: false,
            overflow_delay: 0,
        }
    }

    /// Map TAC bits to the corresponding bit of div_counter
    #[inline]
    fn map_tac_bit(sel: u8) -> u8 {
        match sel & 0b11 {
            0b00 => 9, // 4096 Hz
            0b01 => 3, // 262144 Hz
            0b10 => 5, // 65536 Hz
            0b11 => 7, // 16384 Hz
            _ => 9,
        }
    }

    /// Refresh enable + source bit selection; also resample prev bit state.
    #[inline]
    fn update_tac_common(&mut self) {
        self.timer_enabled = (self.tac & 0b100) != 0;
        self.timer_bit = Self::map_tac_bit(self.tac);
        self.prev_timer_bit_state = ((self.div_counter >> self.timer_bit) & 1) != 0;
    }

    /// Advances the timer by the given number of M-cycles.
    /// Returns true if a TIMA overflow **IRQ request** should be raised now.
    ///
    /// NOTE: On DMG, when TIMA overflow occurs, **reload + IF** happen **one M-cycle later**.
    /// We report `true` on that delayed cycle (not on the cycle that overflowed).  [2](https://gbdev.io/pandocs/Timer_Obscure_Behaviour.html)
    pub fn tick(&mut self, cycles: u32) -> bool {
        let mut irq_timer = false;

        for _ in 0..cycles {
            // 1) System counter always increments (unless STOP; not modeled here)
            self.div_counter = self.div_counter.wrapping_add(1);
            self.divider = (self.div_counter >> 8) as u8;

            // 2) Handle pending overflow delay from a previous cycle
            if self.overflow_pending {
                if self.overflow_delay > 0 {
                    self.overflow_delay -= 1;
                }
                if self.overflow_delay == 0 {
                    // Complete the delayed reload and request IF
                    self.tima = self.tma;
                    irq_timer = true; // request IF this M-cycle
                    self.overflow_pending = false;
                    // NOTE: prev_timer_bit_state is unaffected here
                }
            }

            // 3) Live timer (falling-edge detection) when enabled
            if self.timer_enabled {
                let current = ((self.div_counter >> self.timer_bit) & 1) != 0;
                // Only increment on falling edge: prev=1 -> current=0
                if self.prev_timer_bit_state && !current {
                    self.tick_tima_once();
                }
                self.prev_timer_bit_state = current;
            }
        }

        irq_timer
    }

    /// Increment TIMA once; if it overflows, start the **delayed reload** window.
    #[inline]
    fn tick_tima_once(&mut self) {
        let (next, of) = self.tima.overflowing_add(1);
        self.tima = next;
        if of {
            // Hardware quirk: TIMA becomes 00 for one M-cycle; TMA reload + IF next cycle. [2](https://gbdev.io/pandocs/Timer_Obscure_Behaviour.html)
            self.tima = 0x00;
            self.overflow_pending = true;
            self.overflow_delay = 1; // 1 M-cycle later (not 4 clocks inside an M-cycle)
            #[cfg(feature = "debug_timing")]
            eprintln!(
                "[DEBUG TIMER] overflow: TIMA=00 for 1 cycle; reload=TMA={:#04x} next M-cycle",
                self.tma
            );
        } else {
            #[cfg(feature = "debug_timing")]
            eprintln!(
                "[DEBUG TIMER] TIMA increment at div_counter={}, new tima={:#04x}",
                self.div_counter,
                self.tima
            );
        }
    }

    pub fn read_io(&self, addr: u16) -> u8 {
        match addr {
            0xff04 => self.divider,
            0xff05 => self.tima,
            0xff06 => self.tma,
            0xff07 =>
                0b1111_1000 | (if self.timer_enabled { 0b100 } else { 0 }) | (self.tac & 0b11),
            _ => 0xff,
        }
    }

    pub fn write_io(&mut self, addr: u16, val: u8) {
        match addr {
            // DIV write: reset system counter *and* possibly cause an immediate tick
            // if the currently selected bit was 1 and the timer is enabled (DMG quirk). [2](https://gbdev.io/pandocs/Timer_Obscure_Behaviour.html)[3](https://gbdev.gg8.se/wiki/articles/Timer_Obscure_Behaviour)
            0xff04 => {
                let was_enabled = self.timer_enabled;
                let sel_bit = self.timer_bit;
                let old_bit_state = ((self.div_counter >> sel_bit) & 1) != 0;

                // Quirk: If enabled and old_bit was 1 -> falling edge when counter resets to 0.
                if was_enabled && old_bit_state {
                    self.tick_tima_once();
                }

                self.div_counter = 0;
                self.divider = 0;
                self.prev_timer_bit_state = false;

                #[cfg(feature = "debug_timing")]
                eprintln!(
                    "[DEBUG TIMER] DIV reset to 0 (glitch tick applied: {})",
                    was_enabled && old_bit_state
                );
            }

            // TIMA write: if within the 1-cycle overflow window, **cancel** the pending reload/IF. [2](https://gbdev.io/pandocs/Timer_Obscure_Behaviour.html)[3](https://gbdev.gg8.se/wiki/articles/Timer_Obscure_Behaviour)
            0xff05 => {
                if self.overflow_pending {
                    // Cancel the delayed reload behaviour per Pan Docs.
                    self.overflow_pending = false;
                    self.overflow_delay = 0;
                    #[cfg(feature = "debug_timing")]
                    eprintln!("[DEBUG TIMER] TIMA write canceled delayed reload/IF");
                }
                self.tima = val;
                #[cfg(feature = "debug_timing")]
                eprintln!("[DEBUG TIMER] TIMA written with {:#04x}", val);
            }

            0xff06 => {
                self.tma = val;
                #[cfg(feature = "debug_timing")]
                eprintln!("[DEBUG TIMER] TMA written with {:#04x}", val);
            }

            // TAC write: may cause an immediate tick (glitch) depending on old/new selection
            // and enable bit on DMG. Then update selection and resample state. [2](https://gbdev.io/pandocs/Timer_Obscure_Behaviour.html)[3](https://gbdev.gg8.se/wiki/articles/Timer_Obscure_Behaviour)
            0xff07 => {
                let _old_tac = self.tac;
                let old_enable = self.timer_enabled;
                let old_bit = self.timer_bit;
                let old_bit_state = ((self.div_counter >> old_bit) & 1) != 0;

                // Apply only the writable low 3 bits
                self.tac = val & 0b111;

                // Compute new settings (but don't resample prev yet; we want glitch calc first)
                let new_enable = (self.tac & 0b100) != 0;
                let new_bit = Self::map_tac_bit(self.tac);
                let new_bit_state = ((self.div_counter >> new_bit) & 1) != 0;

                // DMG quirk rules (see Pan Docs “Timer Obscure Behaviour”):
                // - If old_enable && !new_enable && old_bit_state==1  -> tick (disable glitch)
                // - If old_enable && new_enable && old_bit_state==1 && new_bit_state==0 -> tick
                //   (selecting between bits can cause 1->0 at mux output)
                // Note: CGB has slightly different behaviour; we implement DMG here. [2](https://gbdev.io/pandocs/Timer_Obscure_Behaviour.html)[3](https://gbdev.gg8.se/wiki/articles/Timer_Obscure_Behaviour)
                let mut glitch = false;
                if old_enable && !new_enable && old_bit_state {
                    glitch = true;
                } else if old_enable && new_enable && old_bit_state && !new_bit_state {
                    glitch = true;
                }

                if glitch {
                    self.tick_tima_once();
                }

                // Update enable/bit + resample prev state
                self.update_tac_common();

                #[cfg(feature = "debug_timing")]
                eprintln!(
                    "[DEBUG TIMER] TAC from {:#04x} -> {:#04x}, enabled {}->{}; bit {}->{}; glitch={}",
                    _old_tac,
                    self.tac,
                    old_enable,
                    new_enable,
                    old_bit,
                    new_bit,
                    glitch
                );
            }

            _ => {}
        }
    }
}
