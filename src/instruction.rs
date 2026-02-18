// src/instruction.rs
use crate::types::{
    ArithmeticTarget,
    JumpTest,
    IncDecTarget,
    PrefixTarget,
    LoadType,
    StackTarget,
    LoadByteTarget,
    LoadByteSource,
    Reg16,
};

#[derive(Debug, Clone, Copy)]
pub enum Instruction {
    // 8-bit ALU
    ADD(ArithmeticTarget),

    // Control flow
    JP(JumpTest),
    CALL(JumpTest),
    RET(JumpTest),

    // Loads
    LD(LoadType),

    // 16-bit inc/dec
    INC(IncDecTarget),

    // CB prefix
    RLC(PrefixTarget),

    // Stack
    PUSH(StackTarget),
    POP(StackTarget),

    NOP, // 0x00
    HALT, // 0x76
}

#[derive(Debug, Clone, Copy)]
pub enum DecodeError {
    UnknownOpcode(u8, bool), // (opcode, was_cb_prefixed?)
}

impl Instruction {
    pub fn from_byte(byte: u8, prefixed: bool) -> Result<Instruction, DecodeError> {
        if prefixed { Self::from_byte_prefixed(byte) } else { Self::from_byte_not_prefixed(byte) }
    }

    fn from_byte_prefixed(byte: u8) -> Result<Instruction, DecodeError> {
        match byte {
            0x00 => Ok(Instruction::RLC(PrefixTarget::B)), // CB 00: RLC B
            _ => Err(DecodeError::UnknownOpcode(byte, true)),
        }
    }

    fn from_byte_not_prefixed(byte: u8) -> Result<Instruction, DecodeError> {
        match byte {
            0x02 =>
                Ok(
                    Instruction::LD(
                        LoadType::Byte(LoadByteTarget::MemReg16(Reg16::BC), LoadByteSource::A)
                    )
                ),
            0x03 => Ok(Instruction::INC(IncDecTarget::BC)),

            // LD A,(BC)
            0x0a =>
                Ok(
                    Instruction::LD(
                        LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemReg16(Reg16::BC))
                    )
                ),

            // LD (DE),A
            0x12 =>
                Ok(
                    Instruction::LD(
                        LoadType::Byte(LoadByteTarget::MemReg16(Reg16::DE), LoadByteSource::A)
                    )
                ),

            // LD A,(DE)
            0x1a =>
                Ok(
                    Instruction::LD(
                        LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemReg16(Reg16::DE))
                    )
                ),

            0x06 => Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::B, LoadByteSource::D8))),
            0x3e => Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::A, LoadByteSource::D8))),

            0x0e => Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::C, LoadByteSource::D8))),
            0x16 => Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::D, LoadByteSource::D8))),
            0x1e => Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::E, LoadByteSource::D8))),
            0x26 => Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::H, LoadByteSource::D8))),
            0x2e => Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::L, LoadByteSource::D8))),

            0xea =>
                Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::MemImm16, LoadByteSource::A))),
            0xfa =>
                Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemImm16))),

            0xc3 => Ok(Instruction::JP(JumpTest::Always)),
            0xc9 => Ok(Instruction::RET(JumpTest::Always)),
            0xcd => Ok(Instruction::CALL(JumpTest::Always)),

            0x00 => Ok(Instruction::NOP),
            _ => Err(DecodeError::UnknownOpcode(byte, false)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decodes_control_flow_and_loads() {
        // Unprefixed
        assert!(
            matches!(Instruction::from_byte(0xc3, false), Ok(Instruction::JP(JumpTest::Always)))
        );
        assert!(
            matches!(Instruction::from_byte(0xc9, false), Ok(Instruction::RET(JumpTest::Always)))
        );
        assert!(
            matches!(Instruction::from_byte(0xcd, false), Ok(Instruction::CALL(JumpTest::Always)))
        );
        assert!(matches!(Instruction::from_byte(0x02, false), Ok(Instruction::LD(_))));
        assert!(matches!(Instruction::from_byte(0x03, false), Ok(Instruction::INC(_))));
        assert!(matches!(Instruction::from_byte(0xea, false), Ok(Instruction::LD(_))));
        assert!(matches!(Instruction::from_byte(0xfa, false), Ok(Instruction::LD(_))));
    }

    #[test]
    fn decodes_ld_a_d8() {
        let insn = Instruction::from_byte(0x3e, false);
        assert!(
            matches!(
                insn,
                Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::A, LoadByteSource::D8)))
            )
        );
    }

    #[test]
    fn decodes_ld_r_d8_group() {
        assert!(
            matches!(
                Instruction::from_byte(0x06, false),
                Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::B, LoadByteSource::D8)))
            )
        );
        assert!(
            matches!(
                Instruction::from_byte(0x0e, false),
                Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::C, LoadByteSource::D8)))
            )
        );
        assert!(
            matches!(
                Instruction::from_byte(0x16, false),
                Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::D, LoadByteSource::D8)))
            )
        );
        assert!(
            matches!(
                Instruction::from_byte(0x1e, false),
                Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::E, LoadByteSource::D8)))
            )
        );
        assert!(
            matches!(
                Instruction::from_byte(0x26, false),
                Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::H, LoadByteSource::D8)))
            )
        );
        assert!(
            matches!(
                Instruction::from_byte(0x2e, false),
                Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::L, LoadByteSource::D8)))
            )
        );
        assert!(
            matches!(
                Instruction::from_byte(0x3e, false),
                Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::A, LoadByteSource::D8)))
            )
        );
    }
}
