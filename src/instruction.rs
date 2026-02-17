use crate::types::{ArithmeticTarget, JumpTest};

#[derive(Debug, Clone, Copy)]
pub enum Instruction {
    ADD(ArithmeticTarget),
    JP(JumpTest),
    // Add more as you implement them...
}

#[derive(Debug)]
pub enum DecodeError {
    UnknownOpcode(u8),
    // You can add variants for prefixed tables, invalid combinations, etc.
}

impl Instruction {
    /// Decode unprefixed (or common) opcodes.
    pub fn from_byte(byte: u8) -> Result<Self, DecodeError> {
        // Example: return Err until you fill out the table
        Err(DecodeError::UnknownOpcode(byte))
    }

    /// Decode CB-prefixed opcodes (if your CPU has them).
    pub fn from_byte_prefixed(byte: u8) -> Result<Self, DecodeError> {
        Err(DecodeError::UnknownOpcode(byte))
    }

    /// If you want to keep both entry points (non-prefixed vs prefixed),
    /// you can keep this helper symmetrical with your original naming.
    pub fn from_byte_not_prefixed(byte: u8) -> Result<Self, DecodeError> {
        Self::from_byte(byte)
    }
}