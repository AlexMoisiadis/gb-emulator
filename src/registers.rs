use crate::types::{
    CARRY_FLAG_BYTE_POSITION, HALF_CARRY_FLAG_BYTE_POSITION, SUBTRACT_FLAG_BYTE_POSITION,
    ZERO_FLAG_BYTE_POSITION,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct FlagsRegister {
    pub zero: bool,
    pub subtract: bool,
    pub half_carry: bool,
    pub carry: bool,
}

impl From<FlagsRegister> for u8 {
    fn from(flag: FlagsRegister) -> u8 {
        ((flag.zero as u8) << ZERO_FLAG_BYTE_POSITION)
            | ((flag.subtract as u8) << SUBTRACT_FLAG_BYTE_POSITION)
            | ((flag.half_carry as u8) << HALF_CARRY_FLAG_BYTE_POSITION)
            | ((flag.carry as u8) << CARRY_FLAG_BYTE_POSITION)
    }
}

impl From<u8> for FlagsRegister {
    fn from(byte: u8) -> Self {
        let zero = ((byte >> ZERO_FLAG_BYTE_POSITION) & 0b1) != 0;
        let subtract = ((byte >> SUBTRACT_FLAG_BYTE_POSITION) & 0b1) != 0;
        let half_carry = ((byte >> HALF_CARRY_FLAG_BYTE_POSITION) & 0b1) != 0;
        let carry = ((byte >> CARRY_FLAG_BYTE_POSITION) & 0b1) != 0;

        FlagsRegister {
            zero,
            subtract,
            half_carry,
            carry,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Registers {
    pub a: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub f: FlagsRegister,
    pub h: u8,
    pub l: u8,
}

impl Default for Registers {
    fn default() -> Self {
        Self {
            a: 0,
            b: 0,
            c: 0,
            d: 0,
            e: 0,
            f: FlagsRegister::default(),
            h: 0,
            l: 0,
        }
    }
}

impl Registers {
    pub fn get_bc(&self) -> u16 {
        ((self.b as u16) << 8) | (self.c as u16)
    }

    pub fn set_bc(&mut self, value: u16) {
        self.b = ((value & 0xFF00) >> 8) as u8;
        self.c = (value & 0x00FF) as u8;
    }

    // Add more pairs (DE/HL) as needed
}