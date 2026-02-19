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
    divider: u8, // FF04
    tima: u8, // FF05
    tma: u8, // FF06
    tac: u8, // FF07
    enabled: bool, // TAC enable flag
    step: u32, // Number of cycles per TIMA increment
    internalcnt: u32, // Accumulated cycles for TIMA
    internaldiv: u32, // Accumulated cycles for DIV
}

impl Timer {
    pub fn new() -> Self {
        Self {
            divider: 0,
            tima: 0,
            tma: 0,
            tac: 0,
            enabled: false,
            step: 1024, // default frequency (00 -> 4096 Hz)
            internalcnt: 0,
            internaldiv: 0,
        }
    }

    #[inline]
    fn update_step(&mut self) {
        // Update step and enabled from TAC
        self.enabled = (self.tac & 0b100) != 0;
        self.step = match self.tac & 0b11 {
            0b00 => 1024,
            0b01 => 16,
            0b10 => 64,
            0b11 => 256,
            _ => 1024,
        };
    }

    pub fn tick(&mut self, cycles: u32) -> bool {
        let mut overflowed = false;

        // DIV increment (every 256 CPU cycles)
        self.internaldiv += cycles;
        while self.internaldiv >= 256 {
            self.divider = self.divider.wrapping_add(1);
            self.internaldiv -= 256;
        }

        // TIMA increment if enabled
        if self.enabled {
            self.internalcnt += cycles;
            while self.internalcnt >= self.step {
                let (next, of) = self.tima.overflowing_add(1);
                self.tima = next;
                if of {
                    self.tima = self.tma;
                    overflowed = true;
                }
                self.internalcnt -= self.step;
            }
        }

        overflowed
    }

    pub fn read_io(&self, addr: u16) -> u8 {
        match addr {
            0xff04 => self.divider,
            0xff05 => self.tima,
            0xff06 => self.tma,
            0xff07 =>
                0xf8 |
                    (if self.enabled { 0x4 } else { 0 }) |
                    (match self.tac & 0b11 {
                        0b00 => 0,
                        0b01 => 1,
                        0b10 => 2,
                        0b11 => 3,
                        _ => 0,
                    }),
            _ => 0xff,
        }
    }

    pub fn write_io(&mut self, addr: u16, val: u8) {
        match addr {
            0xff04 => {
                self.divider = 0;
                self.internaldiv = 0; // reset internal DIV counter
            }
            0xff05 => {
                self.tima = val;
            }
            0xff06 => {
                self.tma = val;
            }
            0xff07 => {
                self.tac = val & 0b111;
                self.update_step();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn div_increments_every_256_cpu_cycles() {
        let mut t = Timer::new();

        // DIV is high byte of div_counter. 256 cycles -> DIV + 1
        let div0 = t.read_io(0xff04);
        let _of = t.tick(256);
        let div1 = t.read_io(0xff04);
        assert_eq!(div1, div0.wrapping_add(1));

        // Another 512 cycles -> DIV + 2 more
        let _ = t.tick(512);
        let div2 = t.read_io(0xff04);
        assert_eq!(div2, div1.wrapping_add(2));
    }

    #[test]
    fn writing_div_resets_divider_and_resamples_selected_bit() {
        let mut t = Timer::new();

        // Enable at 16,384 Hz (bit7)
        t.write_io(0xff07, 0b0000_0111);

        // 256 cycles -> DIV + 1 in the high byte
        let _ = t.tick(256);
        assert_eq!(t.read_io(0xff04), 0x01);

        // Reset DIV
        t.write_io(0xff04, 0xab);
        assert_eq!(t.read_io(0xff04), 0x00);

        // No spurious TIMA increment after reset
        t.write_io(0xff05, 0x00);
        let before = t.read_io(0xff05);
        let _ = t.tick(4);
        assert_eq!(t.read_io(0xff05), before);
    }

    #[test]
    fn tima_increments_at_each_tac_rate_on_falling_edges() {
        // Try all 4 TAC rates. Each is defined by a bit of the div_counter.
        // TIMA increments on falling edge of that bit.
        // To force two falling edges:
        //   - For bit N, period = 2^(N+1) cycles; falling edge every 2^(N+1) cycles.
        //   - We'll step exactly 2 * period cycles and expect TIMA += 2 (if enabled).

        let rates = [
            (0b0000_0100, 9), // bit9  (4096 Hz)
            (0b0000_0101, 3), // bit3  (262144 Hz)
            (0b0000_0110, 5), // bit5  (65536 Hz)
            (0b0000_0111, 7), // bit7  (16384 Hz)
        ];

        for &(tac, bit) in &rates {
            let mut t = Timer::new();
            t.write_io(0xff07, tac);
            t.write_io(0xff05, 0x10);

            let t0 = t.read_io(0xff05);

            // Two FALLING edges require 2^(bit+2) cycles from an arbitrary phase.
            let total = 1u32 << (bit + 2);
            let mut spent = 0;
            while spent < total {
                let _ = t.tick(4);
                spent += 4;
            }

            let t1 = t.read_io(0xff05);
            assert_eq!(t1, t0.wrapping_add(2), "TAC={:#05b} should add two", tac & 0b111);
        }
    }

    #[test]
    fn tima_overflow_reloads_from_tma_and_signals_overflow() {
        let mut t = Timer::new();

        // Enable 262,144 Hz (bit3). Falling-to-falling is 32 cycles from a clean sample.
        t.write_io(0xff07, 0b0000_0101); // enable + 01
        t.write_io(0xff06, 0x42); // TMA
        t.write_io(0xff05, 0xff); // TIMA

        let overflowed = t.tick(32);
        assert!(overflowed, "overflow should be reported to caller");
        assert_eq!(t.read_io(0xff05), 0x42, "TIMA reloaded from TMA on overflow");
    }

    #[test]
    fn enabling_timer_does_not_immediately_increment_tima() {
        let mut t = Timer::new();

        // Put TIMA at known value, set DIV such that selected bit is currently 0
        t.write_io(0xff05, 0x33);
        t.write_io(0xff07, 0b0000_0000); // disabled, freq=00 (bit9)
        // Make sure div_counter is a value with bit9=0
        // If we advance less than 512 cycles, bit9 (1<<9) remains 0.
        let _ = t.tick(128);
        // Now enable at same freq
        t.write_io(0xff07, 0b0000_0100); // enable + 00

        // No immediate falling edge should be counted on enable.
        let before = t.read_io(0xff05);
        let _ = t.tick(4);
        assert_eq!(t.read_io(0xff05), before, "no spurious increment on enable");
    }

    #[test]
    fn changing_tac_does_not_double_count_the_switch() {
        let mut t = Timer::new();

        // Start with 16,384 Hz (bit7), and ensure we are at a clean edge boundary
        t.write_io(0xff07, 0b0000_0111); // enable + 11 (bit7)
        t.write_io(0xff05, 0x00);

        // Advance exactly 256 cycles to pass one *full* bit7 period (so we end where we started on bit7)
        let _ = t.tick(256);

        // Switch to 262,144 Hz (bit3). Resampling must avoid a phantom edge.
        t.write_io(0xff07, 0b0000_0101); // enable + 01 (bit3)

        // Now advance 12 cycles (< 16 cycles period), so still no falling edge for bit3.
        let _ = t.tick(12);

        // TIMA should still be 0 (no false increment on switch + not enough cycles yet)
        assert_eq!(t.read_io(0xff05), 0x00);

        // Advance 4 more cycles (total 16) -> one falling edge at bit3
        let _ = t.tick(4);
        assert_eq!(
            t.read_io(0xff05),
            0x01,
            "one increment after a proper falling edge post-switch"
        );
    }

    #[test]
    fn disabling_timer_stops_increments_until_reenabled() {
        let mut t = Timer::new();

        // Enable at 262,144 Hz (bit3). First: produce one increment.
        t.write_io(0xff07, 0b0000_0101);
        t.write_io(0xff05, 0x00);

        let _ = t.tick(32); // reliable +1 from clean sample
        assert_eq!(t.read_io(0xff05), 0x01);

        // Disable timer
        t.write_io(0xff07, 0b0000_0001); // disable, keep freq bits
        let _ = t.tick(64);
        assert_eq!(t.read_io(0xff05), 0x01, "no change while disabled");

        // Re-enable; advance to the next falling edge boundary
        t.write_io(0xff07, 0b0000_0101);
        let _ = t.tick(32);
        assert_eq!(t.read_io(0xff05), 0x02);
    }
}
