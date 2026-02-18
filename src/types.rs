// src/types.rs
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

#[derive(Debug, Clone, Copy)]
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

#[derive(Debug, Clone, Copy)]
pub enum IncDecTarget {
    BC,
}

#[derive(Debug, Clone, Copy)]
pub enum PrefixTarget {
    B,
}

// New: 16-bit register names for addressing/data ops
#[derive(Debug, Clone, Copy)]
pub enum Reg16 {
    AF,
    BC,
    DE,
    HL,
    SP,
    PC,
}

// 8-bit load targets/sources
#[derive(Debug, Clone, Copy)]
pub enum LoadByteTarget {
    A,
    B,
    C,
    D,
    E,
    H,
    L,
    HLI, // legacy placeholder for (HL) if you need it
    MemReg16(Reg16), // (BC), (DE), (HL)
    MemImm8, // (a8)
    MemImm16, // (a16)
}

#[derive(Debug, Clone, Copy)]
pub enum LoadByteSource {
    A,
    B,
    C,
    D,
    E,
    H,
    L,
    D8,
    HLI, // legacy placeholder for (HL)
    MemReg16(Reg16), // (BC), (DE), (HL)
    MemImm8, // (a8)
    MemImm16, // (a16)
}

#[derive(Debug, Clone, Copy)]
pub enum LoadType {
    Byte(LoadByteTarget, LoadByteSource),
}

// Flag bit positions (high nibble of F)
pub const ZERO_FLAG_BYTE_POSITION: u8 = 7;
pub const SUBTRACT_FLAG_BYTE_POSITION: u8 = 6;
pub const HALF_CARRY_FLAG_BYTE_POSITION: u8 = 5;
pub const CARRY_FLAG_BYTE_POSITION: u8 = 4;
