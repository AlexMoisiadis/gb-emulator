// src/input/joypad.rs

bitflags::bitflags! {
    pub struct JoypadInterrupt: u8 {
        const JOYPAD = 0b0001_0000; // IF bit 4
    }
}

#[derive(Copy, Clone, Debug)]
pub enum Button {
    Right,
    Left,
    Up,
    Down,
    A,
    B,
    Select,
    Start,
}

pub struct Joypad {
    /// P1 register upper bits (selection bits only: bits 4–5 writable)
    select: u8,

    /// Current physical button state (1 = released, 0 = pressed)
    buttons: u8,

    /// Previous output state (for edge detection)
    last_output: u8,
}

impl Joypad {
    pub fn new() -> Self {
        Self {
            select: 0x30, // nothing selected initially
            buttons: 0xff, // all released (active low)
            last_output: 0xff,
        }
    }

    /// Press a button (active low)
    pub fn press(&mut self, button: Button) {
        self.set_button(button, true);
    }

    /// Release a button
    pub fn release(&mut self, button: Button) {
        self.set_button(button, false);
    }

    fn set_button(&mut self, button: Button, pressed: bool) {
        let mask = match button {
            Button::Right => 0b0000_0001,
            Button::Left => 0b0000_0010,
            Button::Up => 0b0000_0100,
            Button::Down => 0b0000_1000,
            Button::A => 0b0001_0000,
            Button::B => 0b0010_0000,
            Button::Select => 0b0100_0000,
            Button::Start => 0b1000_0000,
        };

        if pressed {
            self.buttons &= !mask;
        } else {
            self.buttons |= mask;
        }

        #[cfg(feature = "trace_input")]
        eprintln!(
            "[JOY] {:?} {}  buttons={:#010b}",
            button,
            if pressed {
                "DOWN"
            } else {
                "UP"
            },
            self.buttons
        );
    }

    /// CPU write to FF00
    pub fn write(&mut self, value: u8) {
        // Only bits 4–5 writable
        self.select = value & 0x30;
        #[cfg(feature = "trace_input")]
        eprintln!(
            "[JOY] write FF00 <= {:#04x}  (P14={} P15={})",
            value,
            ((value & 0x10) == 0) as u8, // 1 = directions selected
            ((value & 0x20) == 0) as u8 // 1 = buttons selected
        );
    }

    /// CPU read from FF00
    pub fn read(&self) -> u8 {
        let mut lo = 0x0f; // all 4 input lines idle-high

        if (self.select & 0x10) == 0 {
            // P14 low — direction keys selected
            lo &= self.buttons & 0x0f;
        }
        if (self.select & 0x20) == 0 {
            // P15 low — action buttons selected
            lo &= (self.buttons >> 4) & 0x0f;
        }

        // bits 7-6: always 1 (unused on DMG)
        // bits 5-4: reflect select lines written by CPU
        // bits 3-0: input lines (0 = pressed, 1 = released)

        let result = 0xc0 | self.select | lo;
        #[cfg(feature = "trace_input")]
        eprintln!(
            "[JOY] read  FF00 => {:#04x}  select={:#04x} buttons={:#08b}",
            result,
            self.select,
            self.buttons
        );
        result
    }
    /// Call once per CPU tick to check interrupt edge
    pub fn update_interrupt(&mut self) -> Option<JoypadInterrupt> {
        let current = self.read();

        // Interrupt when any bit transitions from 1 -> 0
        let falling_edge = self.last_output & !current;

        self.last_output = current;

        if (falling_edge & 0x0f) != 0 {
            Some(JoypadInterrupt::JOYPAD)
        } else {
            None
        }
    }
}
