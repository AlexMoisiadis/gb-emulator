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
    // 8-bit ALU (placeholder kept)
    ADD(ArithmeticTarget),

    // Control flow
    JP(JumpTest),
    JR(JumpTest),
    CALL(JumpTest),
    RET(JumpTest),

    // Loads (generic 8-bit)
    LD(LoadType),

    // 16-bit immediate load (LD rr,d16)
    LD16Imm(Reg16),

    // 8-bit INC/DEC targets (registers and (HL))
    INC8(LoadByteTarget),
    DEC8(LoadByteTarget),

    // 16-bit inc/dec (you currently use BC here)
    INC(IncDecTarget),

    // HL auto-increment / auto-decrement addressing
    // 0x22: LD (HL+),A
    LdHliA,
    // 0x2A: LD A,(HL+)
    LdAHli,
    // 0x32: LD (HL-),A
    LdHldA,
    // 0x3A: LD A,(HL-)
    LdAHld,

    // Addressed loads
    // 0x08: LD (a16),SP
    LdA16Sp,
    // 0xF9: LD SP,HL
    LdSpHl,

    // ALU group (A, r|(HL)|d8)
    AddA(LoadByteSource),
    AdcA(LoadByteSource),
    SubA(LoadByteSource),
    SbcA(LoadByteSource),
    AndA(LoadByteSource),
    XorA(LoadByteSource),
    OrA(LoadByteSource),
    CpA(LoadByteSource),

    EI, // 0xFB
    DI, // 0xF3
    RETI, // 0xD9

    // RST vectors
    Rst(u16),

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

    #[inline]
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

    #[inline]
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
        // --- Special: NOP ---
        if byte == 0x00 {
            return Ok(Instruction::NOP);
        }

        // --- LD r,r' block (0x40..=0x7F), except 0x76=HALT ---
        if (0x40..=0x7f).contains(&byte) {
            if byte == 0x76 {
                return Ok(Instruction::HALT);
            }
            let dst_code = (byte >> 3) & 0b111;
            let src_code = byte & 0b111;
            let dst = Self::reg_code_to_target(dst_code);
            let src = Self::reg_code_to_source(src_code);
            return Ok(Instruction::LD(LoadType::Byte(dst, src)));
        }

        match byte {
            // -------- Addressed loads & pointer helpers --------
            0x02 =>
                Ok(
                    Instruction::LD(
                        LoadType::Byte(LoadByteTarget::MemReg16(Reg16::BC), LoadByteSource::A)
                    )
                ), // LD (BC),A
            0x0a =>
                Ok(
                    Instruction::LD(
                        LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemReg16(Reg16::BC))
                    )
                ), // LD A,(BC)
            0x12 =>
                Ok(
                    Instruction::LD(
                        LoadType::Byte(LoadByteTarget::MemReg16(Reg16::DE), LoadByteSource::A)
                    )
                ), // LD (DE),A
            0x1a =>
                Ok(
                    Instruction::LD(
                        LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemReg16(Reg16::DE))
                    )
                ), // LD A,(DE)

            // HL+ / HL-
            0x22 => Ok(Instruction::LdHliA), // LD (HL+),A
            0x2a => Ok(Instruction::LdAHli), // LD A,(HL+)
            0x32 => Ok(Instruction::LdHldA), // LD (HL-),A
            0x3a => Ok(Instruction::LdAHld), // LD A,(HL-)

            // Absolute addressing (a16)
            0xea =>
                Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::MemImm16, LoadByteSource::A))), // LD (a16),A
            0xfa =>
                Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemImm16))), // LD A,(a16)

            // High-RAM I/O (FF00 + a8) and (FF00 + C)
            0xe0 => Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::MemImm8, LoadByteSource::A))), // LDH (a8),A
            0xf0 => Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemImm8))), // LDH A,(a8)
            0xe2 =>
                Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::MemHighC, LoadByteSource::A))), // LD (C),A
            0xf2 =>
                Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemHighC))), // LD A,(C)

            // LD (a16),SP and LD SP,HL
            0x08 => Ok(Instruction::LdA16Sp), // LD (a16),SP
            0xf9 => Ok(Instruction::LdSpHl), // LD SP,HL

            // -------- Immediate loads (LD r,d8) --------
            0x06 => Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::B, LoadByteSource::D8))),
            0x0e => Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::C, LoadByteSource::D8))),
            0x16 => Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::D, LoadByteSource::D8))),
            0x1e => Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::E, LoadByteSource::D8))),
            0x26 => Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::H, LoadByteSource::D8))),
            0x2e => Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::L, LoadByteSource::D8))),
            0x3e => Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::A, LoadByteSource::D8))),

            // -------- 16-bit immediate loads (LD rr,d16) --------
            0x01 => Ok(Instruction::LD16Imm(Reg16::BC)),
            0x11 => Ok(Instruction::LD16Imm(Reg16::DE)),
            0x21 => Ok(Instruction::LD16Imm(Reg16::HL)),
            0x31 => Ok(Instruction::LD16Imm(Reg16::SP)),

            // -------- 8-bit INC/DEC (registers and (HL)) --------
            0x04 => Ok(Instruction::INC8(LoadByteTarget::B)),
            0x0c => Ok(Instruction::INC8(LoadByteTarget::C)),
            0x14 => Ok(Instruction::INC8(LoadByteTarget::D)),
            0x1c => Ok(Instruction::INC8(LoadByteTarget::E)),
            0x24 => Ok(Instruction::INC8(LoadByteTarget::H)),
            0x2c => Ok(Instruction::INC8(LoadByteTarget::L)),
            0x34 => Ok(Instruction::INC8(LoadByteTarget::MemReg16(Reg16::HL))),
            0x3c => Ok(Instruction::INC8(LoadByteTarget::A)),

            0x05 => Ok(Instruction::DEC8(LoadByteTarget::B)),
            0x0d => Ok(Instruction::DEC8(LoadByteTarget::C)),
            0x15 => Ok(Instruction::DEC8(LoadByteTarget::D)),
            0x1d => Ok(Instruction::DEC8(LoadByteTarget::E)),
            0x25 => Ok(Instruction::DEC8(LoadByteTarget::H)),
            0x2d => Ok(Instruction::DEC8(LoadByteTarget::L)),
            0x35 => Ok(Instruction::DEC8(LoadByteTarget::MemReg16(Reg16::HL))),
            0x3d => Ok(Instruction::DEC8(LoadByteTarget::A)),

            // 16-bit INC (you currently use BC here)
            0x03 => Ok(Instruction::INC(IncDecTarget::BC)),

            // -------- ALU group (register/(HL) tables) --------
            // 0x80..=0x87: ADD A,r
            0x80..=0x87 => {
                let s = Self::reg_code_to_source(byte & 0b111);
                Ok(Instruction::AddA(s))
            }
            // 0x88..=0x8F: ADC A,r
            0x88..=0x8f => {
                let s = Self::reg_code_to_source(byte & 0b111);
                Ok(Instruction::AdcA(s))
            }
            // 0x90..=0x97: SUB A,r
            0x90..=0x97 => {
                let s = Self::reg_code_to_source(byte & 0b111);
                Ok(Instruction::SubA(s))
            }
            // 0x98..=0x9F: SBC A,r
            0x98..=0x9f => {
                let s = Self::reg_code_to_source(byte & 0b111);
                Ok(Instruction::SbcA(s))
            }
            // 0xA0..=0xA7: AND A,r
            0xa0..=0xa7 => {
                let s = Self::reg_code_to_source(byte & 0b111);
                Ok(Instruction::AndA(s))
            }
            // 0xA8..=0xAF: XOR A,r
            0xa8..=0xaf => {
                let s = Self::reg_code_to_source(byte & 0b111);
                Ok(Instruction::XorA(s))
            }
            // 0xB0..=0xB7: OR A,r
            0xb0..=0xb7 => {
                let s = Self::reg_code_to_source(byte & 0b111);
                Ok(Instruction::OrA(s))
            }
            // 0xB8..=0xBF: CP A,r
            0xb8..=0xbf => {
                let s = Self::reg_code_to_source(byte & 0b111);
                Ok(Instruction::CpA(s))
            }

            // ----- RST vectors -----
            0xc7 => Ok(Instruction::Rst(0x00)), // RST 00h
            0xcf => Ok(Instruction::Rst(0x08)), // RST 08h
            0xd7 => Ok(Instruction::Rst(0x10)), // RST 10h
            0xdf => Ok(Instruction::Rst(0x18)), // RST 18h
            0xe7 => Ok(Instruction::Rst(0x20)), // RST 20h
            0xef => Ok(Instruction::Rst(0x28)), // RST 28h
            0xf7 => Ok(Instruction::Rst(0x30)), // RST 30h
            0xff => Ok(Instruction::Rst(0x38)), // RST 38h

            // ALU immediates (A, d8)
            0xc6 => Ok(Instruction::AddA(LoadByteSource::D8)),
            0xce => Ok(Instruction::AdcA(LoadByteSource::D8)),
            0xd6 => Ok(Instruction::SubA(LoadByteSource::D8)),
            0xde => Ok(Instruction::SbcA(LoadByteSource::D8)),
            0xe6 => Ok(Instruction::AndA(LoadByteSource::D8)),
            0xee => Ok(Instruction::XorA(LoadByteSource::D8)),
            0xf6 => Ok(Instruction::OrA(LoadByteSource::D8)),
            0xfe => Ok(Instruction::CpA(LoadByteSource::D8)),

            // EI / DI / RETI
            0xfb => Ok(Instruction::EI),
            0xf3 => Ok(Instruction::DI),
            0xd9 => Ok(Instruction::RETI),

            // -------- Relative jumps --------
            0x18 => Ok(Instruction::JR(JumpTest::Always)), // JR +e8
            0x20 => Ok(Instruction::JR(JumpTest::NotZero)), // JR NZ,+e8
            0x28 => Ok(Instruction::JR(JumpTest::Zero)), // JR Z,+e8
            0x30 => Ok(Instruction::JR(JumpTest::NotCarry)), // JR NC,+e8
            0x38 => Ok(Instruction::JR(JumpTest::Carry)), // JR C,+e8

            // -------- Jumps: unconditional + conditional --------
            0xc3 => Ok(Instruction::JP(JumpTest::Always)), // JP a16
            0xc2 => Ok(Instruction::JP(JumpTest::NotZero)), // JP NZ,a16
            0xca => Ok(Instruction::JP(JumpTest::Zero)), // JP Z,a16
            0xd2 => Ok(Instruction::JP(JumpTest::NotCarry)), // JP NC,a16
            0xda => Ok(Instruction::JP(JumpTest::Carry)), // JP C,a16

            // -------- CALL: unconditional + conditional --------
            0xcd => Ok(Instruction::CALL(JumpTest::Always)), // CALL a16
            0xc4 => Ok(Instruction::CALL(JumpTest::NotZero)), // CALL NZ,a16
            0xcc => Ok(Instruction::CALL(JumpTest::Zero)), // CALL Z,a16
            0xd4 => Ok(Instruction::CALL(JumpTest::NotCarry)), // CALL NC,a16
            0xdc => Ok(Instruction::CALL(JumpTest::Carry)), // CALL C,a16

            // -------- RET: unconditional + conditional --------
            0xc9 => Ok(Instruction::RET(JumpTest::Always)), // RET
            0xc0 => Ok(Instruction::RET(JumpTest::NotZero)), // RET NZ
            0xc8 => Ok(Instruction::RET(JumpTest::Zero)), // RET Z
            0xd0 => Ok(Instruction::RET(JumpTest::NotCarry)), // RET NC
            0xd8 => Ok(Instruction::RET(JumpTest::Carry)), // RET C

            // -------- Stack: POP/PUSH --------
            0xc1 => Ok(Instruction::POP(StackTarget::BC)),
            0xd1 => Ok(Instruction::POP(StackTarget::DE)),
            0xe1 => Ok(Instruction::POP(StackTarget::HL)),
            0xf1 => Ok(Instruction::POP(StackTarget::AF)),

            0xc5 => Ok(Instruction::PUSH(StackTarget::BC)),
            0xd5 => Ok(Instruction::PUSH(StackTarget::DE)),
            0xe5 => Ok(Instruction::PUSH(StackTarget::HL)),
            0xf5 => Ok(Instruction::PUSH(StackTarget::AF)),

            _ => Err(DecodeError::UnknownOpcode(byte, false)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_ld_rr_prime_basic() {
        assert!(
            matches!(
                Instruction::from_byte(0x78, false),
                Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::A, LoadByteSource::B)))
            )
        );
        assert!(
            matches!(
                Instruction::from_byte(0x47, false),
                Ok(Instruction::LD(LoadType::Byte(LoadByteTarget::B, LoadByteSource::A)))
            )
        );
        assert!(
            matches!(
                Instruction::from_byte(0x7e, false),
                Ok(
                    Instruction::LD(
                        LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemReg16(Reg16::HL))
                    )
                )
            )
        );
        assert!(
            matches!(
                Instruction::from_byte(0x70, false),
                Ok(
                    Instruction::LD(
                        LoadType::Byte(LoadByteTarget::MemReg16(Reg16::HL), LoadByteSource::B)
                    )
                )
            )
        );
        assert!(matches!(Instruction::from_byte(0x76, false), Ok(Instruction::HALT)));
    }

    #[test]
    fn decodes_inc_dec_8bit_subset() {
        assert!(
            matches!(Instruction::from_byte(0x04, false), Ok(Instruction::INC8(LoadByteTarget::B)))
        );
        assert!(
            matches!(
                Instruction::from_byte(0x35, false),
                Ok(Instruction::DEC8(LoadByteTarget::MemReg16(Reg16::HL)))
            )
        );
        assert!(
            matches!(Instruction::from_byte(0x3d, false), Ok(Instruction::DEC8(LoadByteTarget::A)))
        );
    }

    #[test]
    fn decodes_hl_plus_minus_variants() {
        assert!(matches!(Instruction::from_byte(0x22, false), Ok(Instruction::LdHliA)));
        assert!(matches!(Instruction::from_byte(0x2a, false), Ok(Instruction::LdAHli)));
        assert!(matches!(Instruction::from_byte(0x32, false), Ok(Instruction::LdHldA)));
        assert!(matches!(Instruction::from_byte(0x3a, false), Ok(Instruction::LdAHld)));
    }

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

    #[test]
    fn decodes_ld_rr_d16_and_sp_hl_a16_sp() {
        assert!(matches!(Instruction::from_byte(0x01, false), Ok(Instruction::LD16Imm(Reg16::BC))));
        assert!(matches!(Instruction::from_byte(0x11, false), Ok(Instruction::LD16Imm(Reg16::DE))));
        assert!(matches!(Instruction::from_byte(0x21, false), Ok(Instruction::LD16Imm(Reg16::HL))));
        assert!(matches!(Instruction::from_byte(0x31, false), Ok(Instruction::LD16Imm(Reg16::SP))));
        assert!(matches!(Instruction::from_byte(0x08, false), Ok(Instruction::LdA16Sp)));
        assert!(matches!(Instruction::from_byte(0xf9, false), Ok(Instruction::LdSpHl)));
    }

    #[test]
    fn decodes_alu_tables_and_immediates() {
        use LoadByteSource::*;
        assert!(matches!(Instruction::from_byte(0x87, false), Ok(Instruction::AddA(A))));
        assert!(
            matches!(
                Instruction::from_byte(0x86, false),
                Ok(Instruction::AddA(LoadByteSource::MemReg16(Reg16::HL)))
            )
        );
        assert!(matches!(Instruction::from_byte(0xce, false), Ok(Instruction::AdcA(D8))));
        assert!(matches!(Instruction::from_byte(0xd6, false), Ok(Instruction::SubA(D8))));
        assert!(matches!(Instruction::from_byte(0xe6, false), Ok(Instruction::AndA(D8))));
        assert!(matches!(Instruction::from_byte(0xee, false), Ok(Instruction::XorA(D8))));
        assert!(matches!(Instruction::from_byte(0xf6, false), Ok(Instruction::OrA(D8))));
        assert!(matches!(Instruction::from_byte(0xfe, false), Ok(Instruction::CpA(D8))));
    }

    #[test]
    fn decodes_jr_and_conditional_jumps_calls_rets() {
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

        assert!(
            matches!(Instruction::from_byte(0xc3, false), Ok(Instruction::JP(JumpTest::Always)))
        );
        assert!(
            matches!(Instruction::from_byte(0xc2, false), Ok(Instruction::JP(JumpTest::NotZero)))
        );
        assert!(matches!(Instruction::from_byte(0xca, false), Ok(Instruction::JP(JumpTest::Zero))));
        assert!(
            matches!(Instruction::from_byte(0xd2, false), Ok(Instruction::JP(JumpTest::NotCarry)))
        );
        assert!(
            matches!(Instruction::from_byte(0xda, false), Ok(Instruction::JP(JumpTest::Carry)))
        );

        assert!(
            matches!(Instruction::from_byte(0xcd, false), Ok(Instruction::CALL(JumpTest::Always)))
        );
        assert!(
            matches!(Instruction::from_byte(0xc4, false), Ok(Instruction::CALL(JumpTest::NotZero)))
        );
        assert!(
            matches!(Instruction::from_byte(0xcc, false), Ok(Instruction::CALL(JumpTest::Zero)))
        );
        assert!(
            matches!(Instruction::from_byte(0xd4, false), Ok(Instruction::CALL(JumpTest::NotCarry)))
        );
        assert!(
            matches!(Instruction::from_byte(0xdc, false), Ok(Instruction::CALL(JumpTest::Carry)))
        );

        assert!(
            matches!(Instruction::from_byte(0xc9, false), Ok(Instruction::RET(JumpTest::Always)))
        );
        assert!(
            matches!(Instruction::from_byte(0xc0, false), Ok(Instruction::RET(JumpTest::NotZero)))
        );
        assert!(
            matches!(Instruction::from_byte(0xc8, false), Ok(Instruction::RET(JumpTest::Zero)))
        );
        assert!(
            matches!(Instruction::from_byte(0xd0, false), Ok(Instruction::RET(JumpTest::NotCarry)))
        );
        assert!(
            matches!(Instruction::from_byte(0xd8, false), Ok(Instruction::RET(JumpTest::Carry)))
        );
    }

    #[test]
    fn decodes_push_pop_pairs() {
        assert!(
            matches!(Instruction::from_byte(0xc1, false), Ok(Instruction::POP(StackTarget::BC)))
        );
        assert!(
            matches!(Instruction::from_byte(0xd1, false), Ok(Instruction::POP(StackTarget::DE)))
        );
        assert!(
            matches!(Instruction::from_byte(0xe1, false), Ok(Instruction::POP(StackTarget::HL)))
        );
        assert!(
            matches!(Instruction::from_byte(0xf1, false), Ok(Instruction::POP(StackTarget::AF)))
        );

        assert!(
            matches!(Instruction::from_byte(0xc5, false), Ok(Instruction::PUSH(StackTarget::BC)))
        );
        assert!(
            matches!(Instruction::from_byte(0xd5, false), Ok(Instruction::PUSH(StackTarget::DE)))
        );
        assert!(
            matches!(Instruction::from_byte(0xe5, false), Ok(Instruction::PUSH(StackTarget::HL)))
        );
        assert!(
            matches!(Instruction::from_byte(0xf5, false), Ok(Instruction::PUSH(StackTarget::AF)))
        );
    }

    #[test]
    fn decodes_rst_vectors() {
        assert!(matches!(Instruction::from_byte(0xc7, false), Ok(Instruction::Rst(0x0000))));
        assert!(matches!(Instruction::from_byte(0xcf, false), Ok(Instruction::Rst(0x0008))));
        assert!(matches!(Instruction::from_byte(0xd7, false), Ok(Instruction::Rst(0x0010))));
        assert!(matches!(Instruction::from_byte(0xdf, false), Ok(Instruction::Rst(0x0018))));
        assert!(matches!(Instruction::from_byte(0xe7, false), Ok(Instruction::Rst(0x0020))));
        assert!(matches!(Instruction::from_byte(0xef, false), Ok(Instruction::Rst(0x0028))));
        assert!(matches!(Instruction::from_byte(0xf7, false), Ok(Instruction::Rst(0x0030))));
        assert!(matches!(Instruction::from_byte(0xff, false), Ok(Instruction::Rst(0x0038))));
    }
}
