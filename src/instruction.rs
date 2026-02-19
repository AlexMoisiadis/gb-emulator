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
    DEC(IncDecTarget),

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
    RRC(PrefixTarget),
    RL(PrefixTarget),
    RR(PrefixTarget),
    SLA(PrefixTarget),
    SRA(PrefixTarget),
    SWAP(PrefixTarget),
    SRL(PrefixTarget),
    BIT(u8, PrefixTarget),
    RES(u8, PrefixTarget),
    SET(u8, PrefixTarget),

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
        // CB instructions have this pattern: [operation][bit/register]
        // high 3 bits: operation group (RLC/RRC/RL/RR/SLA/SRA/SWAP/SRL)
        // mid 3 bits: bit index for BIT/RES/SET
        // low 3 bits: target register
        let op_group = byte >> 6;
        let sub_op = (byte >> 3) & 0b111;
        let reg_code = byte & 0b111;

        let target = Self::decode_prefix_target(reg_code);

        match op_group {
            0b00 =>
                match sub_op {
                    0b000 => Ok(Instruction::RLC(target)),
                    0b001 => Ok(Instruction::RRC(target)),
                    0b010 => Ok(Instruction::RL(target)),
                    0b011 => Ok(Instruction::RR(target)),
                    0b100 => Ok(Instruction::SLA(target)),
                    0b101 => Ok(Instruction::SRA(target)),
                    0b110 => Ok(Instruction::SWAP(target)),
                    0b111 => Ok(Instruction::SRL(target)),
                    _ => unreachable!(),
                }
            0b01 => Ok(Instruction::BIT(sub_op, target)),
            0b10 => Ok(Instruction::RES(sub_op, target)),
            0b11 => Ok(Instruction::SET(sub_op, target)),
            _ => unreachable!(),
        }
    }

    fn from_byte_not_prefixed(byte: u8) -> Result<Instruction, DecodeError> {
        // --- Single-byte explicit instructions ---
        match byte {
            0x00 => {
                return Ok(Instruction::NOP);
            }
            0x76 => {
                return Ok(Instruction::HALT);
            }
            0xfb => {
                return Ok(Instruction::EI);
            }
            0xf3 => {
                return Ok(Instruction::DI);
            }
            0xd9 => {
                return Ok(Instruction::RETI);
            }
            0x08 => {
                return Ok(Instruction::LdA16Sp);
            }
            0xf9 => {
                return Ok(Instruction::LdSpHl);
            }
            // Unconditional control flow (missing previously)
            0xc3 => {
                return Ok(Instruction::JP(JumpTest::Always));
            }
            0xcd => {
                return Ok(Instruction::CALL(JumpTest::Always));
            }
            0xc9 => {
                return Ok(Instruction::RET(JumpTest::Always));
            }

            // RST vectors
            0xc7 | 0xcf | 0xd7 | 0xdf | 0xe7 | 0xef | 0xf7 | 0xff => {
                return Ok(Instruction::Rst((byte & 0b0011_1000) as u16));
            }
            _ => {}
        }

        // --- 8-bit INC/DEC (explicitly enumerate the well-formed pattern) ---
        // INC r:  0x04,0x0C,0x14,0x1C,0x24,0x2C,0x34,0x3C
        // DEC r:  0x05,0x0D,0x15,0x1D,0x25,0x2D,0x35,0x3D
        // Pattern: INC => (byte & 0b11_000_111) == 0b00_000_100
        //          DEC => (byte & 0b11_000_111) == 0b00_000_101
        let low3 = byte & 0b111;
        let hi2 = (byte >> 6) & 0b11;
        if hi2 == 0 && low3 == 0b100 {
            let dst = Self::reg_code_to_target((byte >> 3) & 0b111);
            return Ok(Instruction::INC8(dst));
        }
        if hi2 == 0 && low3 == 0b101 {
            let dst = Self::reg_code_to_target((byte >> 3) & 0b111);
            return Ok(Instruction::DEC8(dst));
        }

        // --- LD r,r' (and HALT handled earlier) ---
        if (0x40..=0x7f).contains(&byte) {
            let dst = Self::reg_code_to_target((byte >> 3) & 0b111);
            let src = Self::reg_code_to_source(byte & 0b111);
            return Ok(Instruction::LD(LoadType::Byte(dst, src)));
        }

        // --- LD r,d8 (immediate 8-bit) ---
        // Valid when low 3 bits == 110, covering 0x06,0x0E,0x16,0x1E,0x26,0x2E,0x36,0x3E
        if (0x06..=0x3e).contains(&byte) && (byte & 0b111) == 0b110 {
            let dst = Self::reg_code_to_target((byte >> 3) & 0b111);
            return Ok(Instruction::LD(LoadType::Byte(dst, LoadByteSource::D8)));
        }

        // --- 16-bit INC/DEC ---
        match byte {
            0x03 => {
                return Ok(Instruction::INC(IncDecTarget::BC));
            }
            0x13 => {
                return Ok(Instruction::INC(IncDecTarget::DE));
            }
            0x23 => {
                return Ok(Instruction::INC(IncDecTarget::HL));
            }
            0x33 => {
                return Ok(Instruction::INC(IncDecTarget::SP));
            }
            0x0b => {
                return Ok(Instruction::DEC(IncDecTarget::BC));
            }
            0x1b => {
                return Ok(Instruction::DEC(IncDecTarget::DE));
            }
            0x2b => {
                return Ok(Instruction::DEC(IncDecTarget::HL));
            }
            0x3b => {
                return Ok(Instruction::DEC(IncDecTarget::SP));
            }
            _ => {}
        }

        // --- ALU ops: A, r|(HL) ---
        let alu_ops = [
            (0x80, Instruction::AddA as fn(LoadByteSource) -> Instruction),
            (0x88, Instruction::AdcA as fn(LoadByteSource) -> Instruction),
            (0x90, Instruction::SubA as fn(LoadByteSource) -> Instruction),
            (0x98, Instruction::SbcA as fn(LoadByteSource) -> Instruction),
            (0xa0, Instruction::AndA as fn(LoadByteSource) -> Instruction),
            (0xa8, Instruction::XorA as fn(LoadByteSource) -> Instruction),
            (0xb0, Instruction::OrA as fn(LoadByteSource) -> Instruction),
            (0xb8, Instruction::CpA as fn(LoadByteSource) -> Instruction),
        ];
        for &(base, ctor) in &alu_ops {
            if (byte & 0xf8) == base {
                let src = Self::reg_code_to_source(byte & 0b111);
                return Ok(ctor(src));
            }
        }

        // --- ALU immediate (A,d8) ---
        let alu_imm = [
            (0xc6, Instruction::AddA as fn(LoadByteSource) -> Instruction),
            (0xce, Instruction::AdcA),
            (0xd6, Instruction::SubA),
            (0xde, Instruction::SbcA),
            (0xe6, Instruction::AndA),
            (0xee, Instruction::XorA),
            (0xf6, Instruction::OrA),
            (0xfe, Instruction::CpA),
        ];
        for &(op, ctor) in &alu_imm {
            if byte == op {
                return Ok(ctor(LoadByteSource::D8));
            }
        }

        // --- Relative jumps JR (explicit 5 opcodes) ---
        match byte {
            0x18 => {
                return Ok(Instruction::JR(JumpTest::Always));
            }
            0x20 => {
                return Ok(Instruction::JR(JumpTest::NotZero));
            }
            0x28 => {
                return Ok(Instruction::JR(JumpTest::Zero));
            }
            0x30 => {
                return Ok(Instruction::JR(JumpTest::NotCarry));
            }
            0x38 => {
                return Ok(Instruction::JR(JumpTest::Carry));
            }
            _ => {}
        }

        // --- JP, CALL, RET (conditional opcodes) ---
        match byte {
            // JP cc, a16
            0xc2 => {
                return Ok(Instruction::JP(JumpTest::NotZero));
            }
            0xca => {
                return Ok(Instruction::JP(JumpTest::Zero));
            }
            0xd2 => {
                return Ok(Instruction::JP(JumpTest::NotCarry));
            }
            0xda => {
                return Ok(Instruction::JP(JumpTest::Carry));
            }
            // CALL cc, a16
            0xc4 => {
                return Ok(Instruction::CALL(JumpTest::NotZero));
            }
            0xcc => {
                return Ok(Instruction::CALL(JumpTest::Zero));
            }
            0xd4 => {
                return Ok(Instruction::CALL(JumpTest::NotCarry));
            }
            0xdc => {
                return Ok(Instruction::CALL(JumpTest::Carry));
            }
            // RET cc
            0xc0 => {
                return Ok(Instruction::RET(JumpTest::NotZero));
            }
            0xc8 => {
                return Ok(Instruction::RET(JumpTest::Zero));
            }
            0xd0 => {
                return Ok(Instruction::RET(JumpTest::NotCarry));
            }
            0xd8 => {
                return Ok(Instruction::RET(JumpTest::Carry));
            }
            _ => {}
        }

        // --- Addressed loads & pointers ---
        match byte {
            0x02 => {
                return Ok(
                    Instruction::LD(
                        LoadType::Byte(LoadByteTarget::MemReg16(Reg16::BC), LoadByteSource::A)
                    )
                );
            }
            0x0a => {
                return Ok(
                    Instruction::LD(
                        LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemReg16(Reg16::BC))
                    )
                );
            }
            0x12 => {
                return Ok(
                    Instruction::LD(
                        LoadType::Byte(LoadByteTarget::MemReg16(Reg16::DE), LoadByteSource::A)
                    )
                );
            }
            0x1a => {
                return Ok(
                    Instruction::LD(
                        LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemReg16(Reg16::DE))
                    )
                );
            }
            0x22 => {
                return Ok(Instruction::LdHliA);
            }
            0x2a => {
                return Ok(Instruction::LdAHli);
            }
            0x32 => {
                return Ok(Instruction::LdHldA);
            }
            0x3a => {
                return Ok(Instruction::LdAHld);
            }
            0xea => {
                return Ok(
                    Instruction::LD(LoadType::Byte(LoadByteTarget::MemImm16, LoadByteSource::A))
                );
            }
            0xfa => {
                return Ok(
                    Instruction::LD(LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemImm16))
                );
            }
            0xe0 => {
                return Ok(
                    Instruction::LD(LoadType::Byte(LoadByteTarget::MemImm8, LoadByteSource::A))
                );
            }
            0xf0 => {
                return Ok(
                    Instruction::LD(LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemImm8))
                );
            }
            0xe2 => {
                return Ok(
                    Instruction::LD(LoadType::Byte(LoadByteTarget::MemHighC, LoadByteSource::A))
                );
            }
            0xf2 => {
                return Ok(
                    Instruction::LD(LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemHighC))
                );
            }
            0x01 => {
                return Ok(Instruction::LD16Imm(Reg16::BC));
            }
            0x11 => {
                return Ok(Instruction::LD16Imm(Reg16::DE));
            }
            0x21 => {
                return Ok(Instruction::LD16Imm(Reg16::HL));
            }
            0x31 => {
                return Ok(Instruction::LD16Imm(Reg16::SP));
            }
            _ => {}
        }

        Err(DecodeError::UnknownOpcode(byte, false))
    }

    #[inline]
    fn decode_prefix_target(code: u8) -> PrefixTarget {
        match code & 0b111 {
            0b000 => PrefixTarget::B,
            0b001 => PrefixTarget::C,
            0b010 => PrefixTarget::D,
            0b011 => PrefixTarget::E,
            0b100 => PrefixTarget::H,
            0b101 => PrefixTarget::L,
            0b110 => PrefixTarget::HL,
            0b111 => PrefixTarget::A,
            _ => unreachable!(),
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
}
