/// Game Boy timer block: DIV/TIMA/TMA/TAC with edge-based increments.
///
/// Implementation notes:
/// - `div_counter` is a 16-bit cycle counter; DIV = (div_counter >> 8).
/// - TIMA increments on the **falling edge** of a selected `div_counter` bit when TAC enable=1:
///     TAC[1:0] -> bit index used:
///         00 -> bit 9  (4096 Hz)   (every 1024 cycles)
///         01 -> bit 3  (262144 Hz) (every 16 cycles)
///         10 -> bit 5  (65536 Hz)  (every 64 cycles)
///         11 -> bit 7  (16384 Hz)  (every 256 cycles)
/// - On TIMA overflow (FF->00), TIMA reloads from TMA and IF[TIMER] is requested.

pub struct Timer {
    div_counter: u16,
    tima: u8,
    tma: u8,
    tac: u8, // [2]=enable, [1:0]=freq
    last_edge_bit: bool, // NEW: remembers selected bit across calls & writes
}

impl Timer {
    pub fn new() -> Self {
        Self {
            div_counter: 0,
            tima: 0,
            tma: 0,
            tac: 0,
            last_edge_bit: false, // init to 0 (matches DIV=0 at power-on/reset)
        }
    }

    #[inline]
    fn timer_enable(&self) -> bool {
        (self.tac & 0b0000_0100) != 0
    }

    #[inline]
    fn bit_index_for_tac(&self) -> u8 {
        match self.tac & 0b11 {
            0b00 => 9, // 4096 Hz
            0b01 => 3, // 262144 Hz
            0b10 => 5, // 65536 Hz
            0b11 => 7, // 16384 Hz
            _ => 9,
        }
    }

    #[inline]
    fn selected_bit_now(&self) -> bool {
        let bit = self.bit_index_for_tac();
        ((self.div_counter >> bit) & 1) != 0
    }

    /// Advance by `cycles`. Returns true if TIMA overflowed (caller sets IF[2]).
    pub fn tick(&mut self, cycles: u32) -> bool {
        // Take the previously cached sampled bit
        let mut overflow = false;

        // Advance divider
        self.div_counter = self.div_counter.wrapping_add(cycles as u16);

        if self.timer_enable() {
            let current = self.selected_bit_now();
            // TIMA increments on falling edge: last 1 -> current 0
            if self.last_edge_bit && !current {
                let (next, of) = self.tima.overflowing_add(1);
                if of {
                    self.tima = self.tma;
                    overflow = true;
                } else {
                    self.tima = next;
                }
            }
            // Update the remembered bit for next time
            self.last_edge_bit = current;
        } else {
            // If disabled, just keep sampling updated so an enable later
            // won’t fabricate a falling edge immediately.
            self.last_edge_bit = self.selected_bit_now();
        }

        overflow
    }

    pub fn read_io(&self, addr: u16) -> u8 {
        match addr {
            0xff04 => (self.div_counter >> 8) as u8,
            0xff05 => self.tima,
            0xff06 => self.tma,
            0xff07 => self.tac | 0b1111_1000,
            _ => 0xff,
        }
    }

    pub fn write_io(&mut self, addr: u16, val: u8) {
        match addr {
            0xff04 => {
                // Writing DIV resets the whole divider; selected bit is now 0.
                self.div_counter = 0;
            }
            0xff05 => {
                self.tima = val;
            }
            0xff06 => {
                self.tma = val;
            }
            0xff07 => {
                self.tac = val & 0b0000_0111;
            }
            _ => {}
        }
        // IMPORTANT: resample after any write that can change the observed bit
        self.last_edge_bit = self.selected_bit_now();
    }
}
