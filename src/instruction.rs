use crate::types::{ ArithmeticTarget, JumpTest, IncDecTarget, PrefixTarget, LoadType, StackTarget };

pub enum Instruction {
    ADD(ArithmeticTarget),
    JP(JumpTest),
    LD(LoadType),
    INC(IncDecTarget),
    RLC(PrefixTarget),
    PUSH(StackTarget),
    POP(StackTarget),
    CALL(JumpTest),
    RET(JumpTest),
    // Add more as you implement them...
}

pub enum DecodeError {
    UnknownOpcode(u8),
    // You can add variants for prefixed tables, invalid combinations, etc.
}

impl Instruction {
    pub fn from_byte(byte: u8, prefixed: bool) -> Option<Instruction> {
        if prefixed {
            Instruction::from_byte_prefixed(byte)
        } else {
            Instruction::from_byte_not_prefixed(byte)
        }
    }

    pub fn from_byte_prefixed(byte: u8) -> Option<Instruction> {
        match byte {
            0x00 => Some(Instruction::RLC(PrefixTarget::B)),
            _ => /* TODO: Add mapping for rest of instructions */ None,
        }
    }

    pub fn from_byte_not_prefixed(byte: u8) -> Option<Instruction> {
        match byte {
            0x02 => Some(Instruction::INC(IncDecTarget::BC)),
            _ => /* TODO: Add mapping for rest of instructions */ None,
        }
    }
}
