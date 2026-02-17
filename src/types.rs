#[derive(Debug, Clone, Copy)]
pub enum ArithmeticTarget {
    A, B, C, D, E, H, L,
}

#[derive(Debug, Clone, Copy)]
pub enum JumpTest {
    NotZero,
    Zero,
    NotCarry,
    Carry,
    Always,
}

// Flag bit positions (high nibble of F)
pub const ZERO_FLAG_BYTE_POSITION: u8 = 7;
pub const SUBTRACT_FLAG_BYTE_POSITION: u8 = 6;
pub const HALF_CARRY_FLAG_BYTE_POSITION: u8 = 5;
pub const CARRY_FLAG_BYTE_POSITION: u8 = 4;