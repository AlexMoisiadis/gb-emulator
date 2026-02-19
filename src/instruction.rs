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
    JR(JumpTest),
    CALL(JumpTest),
    RET(JumpTest),

    // Loads
    LD(LoadType),

    // NEW: 16-bit immediate load
    LD16Imm(Reg16),

    // 16-bit inc/dec
    INC(IncDecTarget),

    // 8 bit
    INC8(LoadByteTarget),
    DEC8(LoadByteTarget),

    // ---- NEW: HL auto-increment addressing ----
    // 0x22: LD (HL+),A
    LdHliA,
    // 0x2A: LD A,(HL+)
    LdAHli,

    // NEW: HL auto-decrement addressing
    LdHldA, // 0x32: LD (HL-),A
    LdAHld, // 0x3A: LD A,(HL-)

    // NEW: LD (a16),SP
    LdA16Sp, // 0x08

    // NEW: LD SP,HL
    LdSpHl, // 0xF9

    // CB prefix
    RLC(PrefixTarget),

    // Stack
    PUSH(StackTarget),
    POP(StackTarget),

    // Misc
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

    fn reg_code_to_target(code: u8) -> LoadByteTarget {
        match code & 0b111 {
            0b000 => LoadByteTarget::B,
            0b001 => LoadByteTarget::C,
            0b010 => LoadByteTarget::D,
            0b011 => LoadByteTarget::E,
            0b100 => LoadByteTarget::H,
            0b101 => LoadByteTarget::L,
            0b110 => LoadByteTarget::MemReg16(Reg16::HL), // (HL)
            0b111 => LoadByteTarget::A,
            _ => unreachable!(),
        }
    }

    fn reg_code_to_source(code: u8) -> LoadByteSource {
        match code & 0b111 {
            0b000 => LoadByteSource::B,
            0b001 => LoadByteSource::C,
            0b010 => LoadByteSource::D,
            0b011 => LoadByteSource::E,
            0b100 => LoadByteSource::H,
            0b101 => LoadByteSource::L,
            0b110 => LoadByteSource::MemReg16(Reg16::HL), // (HL)
            0b111 => LoadByteSource::A,
            _ => unreachable!(),
        }
    }

    fn from_byte_not_prefixed(byte: u8) -> Result<Instruction, DecodeError> {
        // --- Special opcodes we already added ---
        if byte == 0x00 {
            return Ok(Instruction::NOP);
        }

        // --- LD r,r' block (0x40–0x7F), except 0x76 = HALT ---
        if (0x40..=0x7f).contains(&byte) {
            if byte == 0x76 {
                return Ok(Instruction::HALT); // HALT instead of LD (HL),(HL)
            }
            let dst_code = (byte >> 3) & 0b111;
            let src_code = byte & 0b111;
            let dst = Self::reg_code_to_target(dst_code);
            let src = Self::reg_code_to_source(src_code);
            return Ok(Instruction::LD(LoadType::Byte(dst, src)));
        }

        match byte {
            // ---- HL auto-increment loads ----
            0x22 => Ok(Instruction::LdHliA), // LD (HL+),A
            0x2a => Ok(Instruction::LdAHli), // LD A,(HL+)

            0xf9 => Ok(Instruction::LdSpHl),

            // --- 8-bit INC r ---
            0x04 => Ok(Instruction::INC8(LoadByteTarget::B)),
            0x0c => Ok(Instruction::INC8(LoadByteTarget::C)),
            0x14 => Ok(Instruction::INC8(LoadByteTarget::D)),
            0x1c => Ok(Instruction::INC8(LoadByteTarget::E)),
            0x24 => Ok(Instruction::INC8(LoadByteTarget::H)),
            0x2c => Ok(Instruction::INC8(LoadByteTarget::L)),
            0x34 => Ok(Instruction::INC8(LoadByteTarget::MemReg16(Reg16::HL))),
            0x3c => Ok(Instruction::INC8(LoadByteTarget::A)),

            // NEW: LD (a16), SP
            0x08 => Ok(Instruction::LdA16Sp),

            // --- 8-bit DEC r ---
            0x05 => Ok(Instruction::DEC8(LoadByteTarget::B)),
            0x0d => Ok(Instruction::DEC8(LoadByteTarget::C)),
            0x15 => Ok(Instruction::DEC8(LoadByteTarget::D)),
            0x1d => Ok(Instruction::DEC8(LoadByteTarget::E)),
            0x25 => Ok(Instruction::DEC8(LoadByteTarget::H)),
            0x2d => Ok(Instruction::DEC8(LoadByteTarget::L)),
            0x35 => Ok(Instruction::DEC8(LoadByteTarget::MemReg16(Reg16::HL))),
            0x3d => Ok(Instruction::DEC8(LoadByteTarget::A)),

            0x32 => Ok(Instruction::LdHldA), // LD (HL-),A
            0x3a => Ok(Instruction::LdAHld), // LD A,(HL-)

            0x02 =>
                Ok(
                    Instruction::LD(
                        LoadType::Byte(LoadByteTarget::MemReg16(Reg16::BC), LoadByteSource::A)
                    )
                ),
            0x03 => Ok(Instruction::INC(IncDecTarget::BC)),

            0x06 => Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::B, LoadByteSource::D8))),
            0x0e => Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::C, LoadByteSource::D8))),
            0x16 => Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::D, LoadByteSource::D8))),
            0x1e => Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::E, LoadByteSource::D8))),
            0x26 => Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::H, LoadByteSource::D8))),
            0x2e => Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::L, LoadByteSource::D8))),
            0x3e => Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::A, LoadByteSource::D8))),

            // --- JR e8 and JR cc,e8 ---
            0x18 => Ok(Instruction::JR(JumpTest::Always)), // JR +e8
            0x20 => Ok(Instruction::JR(JumpTest::NotZero)), // JR NZ,+e8
            0x28 => Ok(Instruction::JR(JumpTest::Zero)), // JR Z,+e8
            0x30 => Ok(Instruction::JR(JumpTest::NotCarry)), // JR NC,+e8
            0x38 => Ok(Instruction::JR(JumpTest::Carry)), // JR C,+e8

            // --- LD rr, d16 ---
            0x01 => Ok(Instruction::LD16Imm(Reg16::BC)), // LD BC,d16
            0x11 => Ok(Instruction::LD16Imm(Reg16::DE)), // LD DE,d16
            0x21 => Ok(Instruction::LD16Imm(Reg16::HL)), // LD HL,d16
            0x31 => Ok(Instruction::LD16Imm(Reg16::SP)), // LD SP,d15

            // Indirect via BC/DE pairs
            0x0a =>
                Ok(
                    Instruction::LD(
                        LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemReg16(Reg16::BC))
                    )
                ),
            0x12 =>
                Ok(
                    Instruction::LD(
                        LoadType::Byte(LoadByteTarget::MemReg16(Reg16::DE), LoadByteSource::A)
                    )
                ),
            0x1a =>
                Ok(
                    Instruction::LD(
                        LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemReg16(Reg16::DE))
                    )
                ),

            // Absolute addressing
            0xea =>
                Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::MemImm16, LoadByteSource::A))),
            0xfa =>
                Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemImm16))),

            // Control flow
            0xc3 => Ok(Instruction::JP(JumpTest::Always)),
            0xc9 => Ok(Instruction::RET(JumpTest::Always)),
            0xcd => Ok(Instruction::CALL(JumpTest::Always)),

            // LDH (a8),A   => (FF00 + imm8) <- A
            0xe0 => Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::MemImm8, LoadByteSource::A))),

            // LDH A,(a8)   => A <- (FF00 + imm8)
            0xf0 => Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemImm8))),

            // LD (C),A     => (FF00 + C) <- A
            0xe2 =>
                Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::MemHighC, LoadByteSource::A))),

            // LD A,(C)     => A <- (FF00 + C)
            0xf2 =>
                Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemHighC))),

            _ => Err(DecodeError::UnknownOpcode(byte, false)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_hl_plus_variants() {
        assert!(matches!(Instruction::from_byte(0x22, false), Ok(Instruction::LdHliA)));
        assert!(matches!(Instruction::from_byte(0x2a, false), Ok(Instruction::LdAHli)));
    }

    #[test]
    fn decodes_ld_rr_prime_basic() {
        // A few samples across the block
        assert!(
            matches!(
                Instruction::from_byte(0x78, false), // LD A,B
                Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::A, LoadByteSource::B)))
            )
        );
        assert!(
            matches!(
                Instruction::from_byte(0x47, false), // LD B,A
                Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::B, LoadByteSource::A)))
            )
        );
        assert!(
            matches!(
                Instruction::from_byte(0x7e, false), // LD A,(HL)
                Ok(
                    Instruction::LD(
                        LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemReg16(Reg16::HL))
                    )
                )
            )
        );
        assert!(
            matches!(
                Instruction::from_byte(0x70, false), // LD (HL),B
                Ok(
                    Instruction::LD(
                        LoadType::Byte(LoadByteTarget::MemReg16(Reg16::HL), LoadByteSource::B)
                    )
                )
            )
        );
        assert!(
            matches!(
                Instruction::from_byte(0x76, false), // HALT
                Ok(Instruction::HALT)
            )
        );
    }

    #[test]
    fn decodes_jr_variants() {
        assert!(
            matches!(Instruction::from_byte(0x18, false), Ok(Instruction::JR(JumpTest::Always)))
        );
        assert!(
            matches!(Instruction::from_byte(0x20, false), Ok(Instruction::JR(JumpTest::NotZero)))
        );
        assert!(matches!(Instruction::from_byte(0x28, false), Ok(Instruction::JR(JumpTest::Zero))));
        assert!(
            matches!(Instruction::from_byte(0x30, false), Ok(Instruction::JR(JumpTest::NotCarry)))
        );
        assert!(
            matches!(Instruction::from_byte(0x38, false), Ok(Instruction::JR(JumpTest::Carry)))
        );
    }

    #[test]
    fn decodes_hl_minus_variants() {
        assert!(matches!(Instruction::from_byte(0x32, false), Ok(Instruction::LdHldA)));
        assert!(matches!(Instruction::from_byte(0x3a, false), Ok(Instruction::LdAHld)));
    }
}

#[cfg(test)]
mod tests_ldh_decode {
    use super::*;
    #[test]
    fn decodes_ldh_and_c_indexed() {
        assert!(
            matches!(
                Instruction::from_byte(0xe0, false),
                Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::MemImm8, LoadByteSource::A)))
            )
        );
        assert!(
            matches!(
                Instruction::from_byte(0xf0, false),
                Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemImm8)))
            )
        );
        assert!(
            matches!(
                Instruction::from_byte(0xe2, false),
                Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::MemHighC, LoadByteSource::A)))
            )
        );
        assert!(
            matches!(
                Instruction::from_byte(0xf2, false),
                Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemHighC)))
            )
        );
    }
}

#[cfg(test)]
mod tests_ld_a16_sp_decode {
    use super::*;
    #[test]
    fn decodes_ld_a16_sp() {
        assert!(matches!(Instruction::from_byte(0x08, false), Ok(Instruction::LdA16Sp)));
    }
}

#[cfg(test)]
mod tests_ld_sp_hl_decode {
    use super::*;
    #[test]
    fn decodes_ld_sp_hl() {
        assert!(matches!(Instruction::from_byte(0xf9, false), Ok(Instruction::LdSpHl)));
    }
}
