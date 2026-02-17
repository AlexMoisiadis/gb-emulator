#[derive(Debug, Clone, Copy)]
pub enum ArithmeticTarget {
    A,
    B,
    C,
    D,
    E,
    H,
    L,
}

pub enum StackTarget {
    BC,
    DE,
}

#[derive(Debug, Clone, Copy)]
pub enum JumpTest {
    NotZero,
    Zero,
    NotCarry,
    Carry,
    Always,
}

pub enum IncDecTarget {
    BC,
}
pub enum PrefixTarget {
    B,
}

pub enum LoadByteTarget {
    A,
    B,
    C,
    D,
    E,
    H,
    L,
    HLI,
}

pub enum LoadByteSource {
    A,
    B,
    C,
    D,
    E,
    H,
    L,
    D8,
    HLI,
}

pub enum LoadType {
    Byte(LoadByteTarget, LoadByteSource),
}

// Flag bit positions (high nibble of F)
pub const ZERO_FLAG_BYTE_POSITION: u8 = 7;
pub const SUBTRACT_FLAG_BYTE_POSITION: u8 = 6;
pub const HALF_CARRY_FLAG_BYTE_POSITION: u8 = 5;
pub const CARRY_FLAG_BYTE_POSITION: u8 = 4;
