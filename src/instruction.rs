use crate::types::{
    ArithmeticTarget,
    IncDecTarget,
    JumpTest,
    LoadByteSource,
    LoadByteTarget,
    LoadType,
    PrefixTarget,
    Reg16,
    StackTarget,
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

    // indirect jump HL
    JPHL,

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

    DAA,
    CPL,
    SCF,
    CCF,

    AddSpR8,
    LdHlSpR8,

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
    AddHL(Reg16),
}

#[derive(Debug, Clone, Copy)]
pub enum DecodeError {
    UnknownOpcode(u8, bool), // (opcode, was_cb_prefixed?)
}

// -------------------------
// Small decode-time tables
// -------------------------

struct AluSpec {
    reg_base: u8, // base for r/(HL) forms (mask 0xF8)
    imm_op: u8, // opcode for immediate d8 form
    ctor: fn(LoadByteSource) -> Instruction, // constructor
}

const ALU_SPECS: &[AluSpec] = &[
    AluSpec { reg_base: 0x80, imm_op: 0xc6, ctor: Instruction::AddA },
    AluSpec { reg_base: 0x88, imm_op: 0xce, ctor: Instruction::AdcA },
    AluSpec { reg_base: 0x90, imm_op: 0xd6, ctor: Instruction::SubA },
    AluSpec { reg_base: 0x98, imm_op: 0xde, ctor: Instruction::SbcA },
    AluSpec { reg_base: 0xa0, imm_op: 0xe6, ctor: Instruction::AndA },
    AluSpec { reg_base: 0xa8, imm_op: 0xee, ctor: Instruction::XorA },
    AluSpec { reg_base: 0xb0, imm_op: 0xf6, ctor: Instruction::OrA },
    AluSpec { reg_base: 0xb8, imm_op: 0xfe, ctor: Instruction::CpA },
];

const JR_TABLE: &[(u8, JumpTest)] = &[
    (0x18, JumpTest::Always),
    (0x20, JumpTest::NotZero),
    (0x28, JumpTest::Zero),
    (0x30, JumpTest::NotCarry),
    (0x38, JumpTest::Carry),
];

const JPCC_TABLE: &[(u8, JumpTest)] = &[
    (0xc2, JumpTest::NotZero),
    (0xca, JumpTest::Zero),
    (0xd2, JumpTest::NotCarry),
    (0xda, JumpTest::Carry),
];

const CALLCC_TABLE: &[(u8, JumpTest)] = &[
    (0xc4, JumpTest::NotZero),
    (0xcc, JumpTest::Zero),
    (0xd4, JumpTest::NotCarry),
    (0xdc, JumpTest::Carry),
];

const RETCC_TABLE: &[(u8, JumpTest)] = &[
    (0xc0, JumpTest::NotZero),
    (0xc8, JumpTest::Zero),
    (0xd0, JumpTest::NotCarry),
    (0xd8, JumpTest::Carry),
];

#[derive(Clone, Copy)]
struct PairLoad {
    opcode: u8,
    target: LoadByteTarget,
    source: LoadByteSource,
}

const PAIR_LOADS: &[PairLoad] = &[
    PairLoad {
        opcode: 0x02,
        target: LoadByteTarget::MemReg16(Reg16::BC),
        source: LoadByteSource::A,
    },
    PairLoad {
        opcode: 0x0a,
        target: LoadByteTarget::A,
        source: LoadByteSource::MemReg16(Reg16::BC),
    },
    PairLoad {
        opcode: 0x12,
        target: LoadByteTarget::MemReg16(Reg16::DE),
        source: LoadByteSource::A,
    },
    PairLoad {
        opcode: 0x1a,
        target: LoadByteTarget::A,
        source: LoadByteSource::MemReg16(Reg16::DE),
    },
];

// Register-code (r) to target/source lookup tables for 8-bit LD group
const REG_TARGETS: [LoadByteTarget; 8] = [
    LoadByteTarget::B,
    LoadByteTarget::C,
    LoadByteTarget::D,
    LoadByteTarget::E,
    LoadByteTarget::H,
    LoadByteTarget::L,
    LoadByteTarget::MemReg16(Reg16::HL), // (HL)
    LoadByteTarget::A,
];

const REG_SOURCES: [LoadByteSource; 8] = [
    LoadByteSource::B,
    LoadByteSource::C,
    LoadByteSource::D,
    LoadByteSource::E,
    LoadByteSource::H,
    LoadByteSource::L,
    LoadByteSource::MemReg16(Reg16::HL), // (HL)
    LoadByteSource::A,
];

// CB-prefix register code -> target
const PREFIX_TARGETS: [PrefixTarget; 8] = [
    PrefixTarget::B,
    PrefixTarget::C,
    PrefixTarget::D,
    PrefixTarget::E,
    PrefixTarget::H,
    PrefixTarget::L,
    PrefixTarget::HL, // (HL)
    PrefixTarget::A,
];

impl Instruction {
    pub fn mnemonic(&self) -> String {
        use Instruction::*;
        match self {
            NOP => "NOP".to_string(),
            HALT => "HALT".to_string(),
            // …add representative mnemonics for your implemented variants
            LD(LoadType::Byte(t, s)) => format!("LD {:?}, {:?}", t, s),
            JP(cond) => format!("JP {:?}", cond),
            JR(cond) => format!("JR {:?}", cond),
            CALL(cond) => format!("CALL {:?}", cond),
            RET(cond) => format!("RET {:?}", cond),
            // etc.
            _ => format!("{:?}", self),
        }
    }

    pub fn from_byte(byte: u8, prefixed: bool) -> Result<Instruction, DecodeError> {
        if prefixed { Self::from_byte_prefixed(byte) } else { Self::from_byte_not_prefixed(byte) }
    }

    fn from_byte_prefixed(byte: u8) -> Result<Instruction, DecodeError> {
        // CB instructions have this pattern: [operation][bit/register]
        // high 2 bits: operation group (RLC/RRC/RL/RR/SLA/SRA/SWAP/SRL)
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
            0x27 => {
                return Ok(Instruction::DAA);
            }
            0x2f => {
                return Ok(Instruction::CPL);
            }
            0x37 => {
                return Ok(Instruction::SCF);
            }
            0x3f => {
                return Ok(Instruction::CCF);
            }
            0xc5 => {
                return Ok(Instruction::PUSH(StackTarget::BC));
            }
            0xd5 => {
                return Ok(Instruction::PUSH(StackTarget::DE));
            }
            0xe5 => {
                return Ok(Instruction::PUSH(StackTarget::HL));
            }
            0xf5 => {
                return Ok(Instruction::PUSH(StackTarget::AF));
            }
            0xc1 => {
                return Ok(Instruction::POP(StackTarget::BC));
            }
            0xd1 => {
                return Ok(Instruction::POP(StackTarget::DE));
            }
            0xe1 => {
                return Ok(Instruction::POP(StackTarget::HL));
            }
            0xf1 => {
                return Ok(Instruction::POP(StackTarget::AF));
            }
            0xe8 => {
                return Ok(Instruction::AddSpR8);
            }
            0xf8 => {
                return Ok(Instruction::LdHlSpR8);
            }
            0x09 => {
                return Ok(Instruction::AddHL(Reg16::BC));
            }
            0x19 => {
                return Ok(Instruction::AddHL(Reg16::DE));
            }
            0x29 => {
                return Ok(Instruction::AddHL(Reg16::HL));
            }
            0x39 => {
                return Ok(Instruction::AddHL(Reg16::SP));
            }
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
            0xe9 => {
                return Ok(Instruction::JPHL);
            }

            // Unconditional control flow
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

        // --- 8-bit INC/DEC (register and (HL)) ---
        // INC r:  0x04,0x0C,0x14,0x1C,0x24,0x2C,0x34,0x3C
        // DEC r:  0x05,0x0D,0x15,0x1D,0x25,0x2D,0x35,0x3D
        // Mask pattern on bits [7..6,2..0]:
        //   INC: (byte & 0b1100_0111) == 0b0000_0100
        //   DEC: (byte & 0b1100_0111) == 0b0000_0101
        if (byte & 0b1100_0111) == 0b0000_0100 {
            let dst = Self::reg_code_to_target((byte >> 3) & 0b111);
            return Ok(Instruction::INC8(dst));
        }
        if (byte & 0b1100_0111) == 0b0000_0101 {
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

        // --- ALU ops: A, r|(HL) and A, d8 via unified specs ---
        for spec in ALU_SPECS {
            if (byte & 0xf8) == spec.reg_base {
                let src = Self::reg_code_to_source(byte & 0b111);
                return Ok((spec.ctor)(src));
            }
            if byte == spec.imm_op {
                return Ok((spec.ctor)(LoadByteSource::D8));
            }
        }

        // --- JR (relative) via table ---
        if let Some((_, jt)) = JR_TABLE.iter().find(|(op, _)| *op == byte) {
            return Ok(Instruction::JR(*jt));
        }

        // --- JP/CALL/RET conditional via tables ---
        if let Some((_, jt)) = JPCC_TABLE.iter().find(|(op, _)| *op == byte) {
            return Ok(Instruction::JP(*jt));
        }
        if let Some((_, jt)) = CALLCC_TABLE.iter().find(|(op, _)| *op == byte) {
            return Ok(Instruction::CALL(*jt));
        }
        if let Some((_, jt)) = RETCC_TABLE.iter().find(|(op, _)| *op == byte) {
            return Ok(Instruction::RET(*jt));
        }

        // --- Addressed loads & pointers ---

        // BC/DE pair loads (table)
        if let Some(p) = PAIR_LOADS.iter().find(|p| p.opcode == byte) {
            return Ok(Instruction::LD(LoadType::Byte(p.target, p.source)));
        }

        // HL auto inc/dec and other addressing forms
        match byte {
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
        PREFIX_TARGETS[(code & 0b111) as usize]
    }

    #[inline]
    fn reg_code_to_target(code: u8) -> LoadByteTarget {
        REG_TARGETS[(code & 0b111) as usize]
    }

    #[inline]
    fn reg_code_to_source(code: u8) -> LoadByteSource {
        REG_SOURCES[(code & 0b111) as usize]
    }
}
