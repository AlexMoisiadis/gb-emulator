// src/cpu.rs
use crate::instruction::{ DecodeError, Instruction };
use crate::registers::Registers;
use crate::types::{ LoadType, LoadByteTarget, LoadByteSource, IncDecTarget, Reg16, JumpTest };
use crate::bus::MemoryBus;

pub struct CPU {
    pub regs: Registers,
    pub pc: u16,
    pub sp: u16,
    pub ime: bool,
    pub halted: bool,
}

impl CPU {
    pub fn new() -> Self {
        Self {
            regs: Registers::default(),
            pc: 0x0000,
            sp: 0xfffe, // typical power-on stack (placeholder)
            ime: false,
            halted: false,
        }
    }

    #[inline]
    fn cond_met(&self, test: JumpTest) -> bool {
        match test {
            JumpTest::Always => true,
            JumpTest::Zero => self.regs.f.zero,
            JumpTest::NotZero => !self.regs.f.zero,
            JumpTest::Carry => self.regs.f.carry,
            JumpTest::NotCarry => !self.regs.f.carry,
        }
    }

    #[inline]
    fn alu_inc8(&mut self, v: u8) -> u8 {
        let res = v.wrapping_add(1);
        // Preserve carry
        let c = self.regs.f.carry;
        self.regs.f.zero = res == 0;
        self.regs.f.subtract = false;
        self.regs.f.half_carry = (v & 0x0f) == 0x0f;
        self.regs.f.carry = c;
        res
    }

    #[inline]
    fn alu_dec8(&mut self, v: u8) -> u8 {
        let res = v.wrapping_sub(1);
        // Preserve carry
        let c = self.regs.f.carry;
        self.regs.f.zero = res == 0;
        self.regs.f.subtract = true;
        self.regs.f.half_carry = (v & 0x0f) == 0x00;
        self.regs.f.carry = c;
        res
    }

    /// Execute a single instruction and return consumed cycles (placeholder counts).
    pub fn step(&mut self, bus: &mut MemoryBus) -> u32 {
        if self.halted {
            return 4;
        }

        // Capture PC before fetching because fetch8() increments PC.
        let pc_before = self.pc;

        let op = self.fetch8(bus); // reads [PC], then PC = PC + 1
        if op == 0xcb {
            let pc_cb = self.pc; // the PC of the CB byte
            let cb = self.fetch8(bus); // fetch the CB sub‑opcode
            eprintln!("STEP CB @ {:04X}: CB {:02X}", pc_cb, cb);

            match Instruction::from_byte(cb, true) {
                Ok(insn) => self.exec(insn, bus),
                Err(e) => self.trap_unknown(e),
            }
        } else {
            eprintln!("STEP    @ {:04X}: {:02X}", pc_before, op);

            match Instruction::from_byte(op, false) {
                Ok(insn) => self.exec(insn, bus),
                Err(e) => self.trap_unknown(e),
            }
        }
    }

    #[inline]
    fn fetch8(&mut self, bus: &mut MemoryBus) -> u8 {
        let b = bus.read_byte(self.pc);
        self.pc = self.pc.wrapping_add(1);
        b
    }

    #[inline]
    fn fetch16(&mut self, bus: &mut MemoryBus) -> u16 {
        // Game Boy is little-endian: low byte first, then high byte
        let lo = self.fetch8(bus) as u16;
        let hi = self.fetch8(bus) as u16;
        (hi << 8) | lo
    }

    fn push16(&mut self, bus: &mut MemoryBus, value: u16) {
        // pre-decrement stack push
        self.sp = self.sp.wrapping_sub(1);
        bus.write_byte(self.sp, (value >> 8) as u8);
        self.sp = self.sp.wrapping_sub(1);
        bus.write_byte(self.sp, (value & 0x00ff) as u8);
    }

    fn pop16(&mut self, bus: &mut MemoryBus) -> u16 {
        let lo = bus.read_byte(self.sp) as u16;
        self.sp = self.sp.wrapping_add(1);
        let hi = bus.read_byte(self.sp) as u16;
        self.sp = self.sp.wrapping_add(1);
        (hi << 8) | lo
    }

    fn exec(&mut self, insn: Instruction, bus: &mut MemoryBus) -> u32 {
        match insn {
            // ----- LD immediate/absolute special-cases -----
            Instruction::LD(LoadType::Byte(LoadByteTarget::A, LoadByteSource::D8)) => {
                let imm = self.fetch8(bus);
                self.regs.a = imm;
                8
            }
            Instruction::LD(LoadType::Byte(LoadByteTarget::B, LoadByteSource::D8)) => {
                let imm = self.fetch8(bus);
                self.regs.b = imm;
                8
            }
            Instruction::LD(LoadType::Byte(LoadByteTarget::C, LoadByteSource::D8)) => {
                let imm = self.fetch8(bus);
                self.regs.c = imm;
                8
            }
            Instruction::LD(LoadType::Byte(LoadByteTarget::D, LoadByteSource::D8)) => {
                let imm = self.fetch8(bus);
                self.regs.d = imm;
                8
            }
            Instruction::LD(LoadType::Byte(LoadByteTarget::E, LoadByteSource::D8)) => {
                let imm = self.fetch8(bus);
                self.regs.e = imm;
                8
            }
            Instruction::LD(LoadType::Byte(LoadByteTarget::H, LoadByteSource::D8)) => {
                let imm = self.fetch8(bus);
                self.regs.h = imm;
                8
            }
            Instruction::LD(LoadType::Byte(LoadByteTarget::L, LoadByteSource::D8)) => {
                let imm = self.fetch8(bus);
                self.regs.l = imm;
                8
            }

            Instruction::LD(LoadType::Byte(LoadByteTarget::MemImm16, LoadByteSource::A)) => {
                let addr = self.fetch16(bus);
                bus.write_byte(addr, self.regs.a);
                16
            }
            Instruction::LD(LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemImm16)) => {
                let addr = self.fetch16(bus);
                self.regs.a = bus.read_byte(addr);
                16
            }

            // Indirect via BC/DE pairs
            Instruction::LD(
                LoadType::Byte(LoadByteTarget::MemReg16(Reg16::BC), LoadByteSource::A),
            ) => {
                let addr = self.regs.get_bc();
                bus.write_byte(addr, self.regs.a);
                8
            }
            Instruction::LD(
                LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemReg16(Reg16::BC)),
            ) => {
                let addr = self.regs.get_bc();
                self.regs.a = bus.read_byte(addr);
                8
            }
            Instruction::LD(
                LoadType::Byte(LoadByteTarget::MemReg16(Reg16::DE), LoadByteSource::A),
            ) => {
                let addr = self.regs.get_de();
                bus.write_byte(addr, self.regs.a);
                8
            }
            Instruction::LD(
                LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemReg16(Reg16::DE)),
            ) => {
                let addr = self.regs.get_de();
                self.regs.a = bus.read_byte(addr);
                8
            }

            Instruction::LD(LoadType::Byte(LoadByteTarget::MemImm8, LoadByteSource::A)) => {
                // write_to_target will fetch the a8 for us
                self.write_to_target(bus, LoadByteTarget::MemImm8, self.regs.a);
                12
            }
            Instruction::LD(LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemImm8)) => {
                // read_from_source will fetch the a8 for us
                let v = self.read_from_source(bus, LoadByteSource::MemImm8);
                self.regs.a = v;
                12
            }

            // --- High-RAM I/O via C register: 8 cycles ---
            Instruction::LD(LoadType::Byte(LoadByteTarget::MemHighC, LoadByteSource::A)) => {
                let addr = 0xff00 | (self.regs.c as u16);
                bus.write_byte(addr, self.regs.a);
                8
            }
            Instruction::LD(LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemHighC)) => {
                let addr = 0xff00 | (self.regs.c as u16);
                self.regs.a = bus.read_byte(addr);
                8
            }

            // ----- Generic LD r,r' (handles (HL) too) -----
            Instruction::LD(LoadType::Byte(tgt, src)) => {
                let involves_hl_mem =
                    matches!(tgt, LoadByteTarget::MemReg16(Reg16::HL)) ||
                    matches!(src, LoadByteSource::MemReg16(Reg16::HL));

                let val = self.read_from_source(bus, src);
                self.write_to_target(bus, tgt, val);
                if involves_hl_mem {
                    8
                } else {
                    4
                }
            }

            // ----- 16-bit INC already present -----
            Instruction::INC(IncDecTarget::BC) => {
                let bc = self.regs.get_bc().wrapping_add(1);
                self.regs.set_bc(bc);
                8
            }

            // ----- Control flow -----
            Instruction::JP(JumpTest::Always) => {
                let addr = self.fetch16(bus);
                self.pc = addr;
                16
            }
            Instruction::CALL(JumpTest::Always) => {
                let addr = self.fetch16(bus);
                self.push16(bus, self.pc);
                self.pc = addr;
                24
            }
            Instruction::RET(JumpTest::Always) => {
                let addr = self.pop16(bus);
                self.pc = addr;
                16
            }

            // ----- Misc -----
            Instruction::NOP => 4,
            Instruction::HALT => {
                self.halted = true;
                4
            }

            Instruction::LD16Imm(Reg16::BC) => {
                let val = self.fetch16(bus);
                self.regs.set_bc(val);
                12
            }
            Instruction::LD16Imm(Reg16::DE) => {
                let val = self.fetch16(bus);
                self.regs.set_de(val);
                12
            }
            Instruction::LD16Imm(Reg16::HL) => {
                let val = self.fetch16(bus);
                self.regs.set_hl(val);
                12
            }
            Instruction::LD16Imm(Reg16::SP) => {
                let val = self.fetch16(bus);
                self.sp = val;
                12
            }

            // ----- 8-bit INC -----
            Instruction::INC8(LoadByteTarget::A) => {
                self.regs.a = self.alu_inc8(self.regs.a);
                4
            }
            Instruction::INC8(LoadByteTarget::B) => {
                self.regs.b = self.alu_inc8(self.regs.b);
                4
            }
            Instruction::INC8(LoadByteTarget::C) => {
                self.regs.c = self.alu_inc8(self.regs.c);
                4
            }
            Instruction::INC8(LoadByteTarget::D) => {
                self.regs.d = self.alu_inc8(self.regs.d);
                4
            }
            Instruction::INC8(LoadByteTarget::E) => {
                self.regs.e = self.alu_inc8(self.regs.e);
                4
            }
            Instruction::INC8(LoadByteTarget::H) => {
                self.regs.h = self.alu_inc8(self.regs.h);
                4
            }
            Instruction::INC8(LoadByteTarget::L) => {
                self.regs.l = self.alu_inc8(self.regs.l);
                4
            }
            Instruction::INC8(LoadByteTarget::MemReg16(Reg16::HL)) => {
                let addr = self.regs.get_hl();
                let v = bus.read_byte(addr);
                let r = self.alu_inc8(v);
                bus.write_byte(addr, r);
                12
            }

            // ----- 8-bit DEC -----
            Instruction::DEC8(LoadByteTarget::A) => {
                self.regs.a = self.alu_dec8(self.regs.a);
                4
            }
            Instruction::DEC8(LoadByteTarget::B) => {
                self.regs.b = self.alu_dec8(self.regs.b);
                4
            }
            Instruction::DEC8(LoadByteTarget::C) => {
                self.regs.c = self.alu_dec8(self.regs.c);
                4
            }
            Instruction::DEC8(LoadByteTarget::D) => {
                self.regs.d = self.alu_dec8(self.regs.d);
                4
            }
            Instruction::DEC8(LoadByteTarget::E) => {
                self.regs.e = self.alu_dec8(self.regs.e);
                4
            }
            Instruction::DEC8(LoadByteTarget::H) => {
                self.regs.h = self.alu_dec8(self.regs.h);
                4
            }
            Instruction::DEC8(LoadByteTarget::L) => {
                self.regs.l = self.alu_dec8(self.regs.l);
                4
            }
            Instruction::DEC8(LoadByteTarget::MemReg16(Reg16::HL)) => {
                let addr = self.regs.get_hl();
                let v = bus.read_byte(addr);
                let r = self.alu_dec8(v);
                bus.write_byte(addr, r);
                12
            }

            // ----- Relative jumps -----
            Instruction::JR(cond) => {
                // Always fetch the displacement (one byte, signed)
                let disp = self.fetch8(bus) as i8 as i32;

                if self.cond_met(cond) {
                    // PC currently points to the next instruction; add signed offset
                    let pc = self.pc as i32;
                    let new_pc = (pc + disp).rem_euclid(0x1_0000) as u16;
                    self.pc = new_pc;
                    12
                } else {
                    8
                }
            }

            // ---- NEW: HL auto-increment addressing ----
            Instruction::LdHliA => {
                let addr = self.regs.get_hl();
                bus.write_byte(addr, self.regs.a);
                let next = addr.wrapping_add(1);
                self.regs.set_hl(next);
                8
            }
            Instruction::LdAHli => {
                let addr = self.regs.get_hl();
                let v = bus.read_byte(addr);
                self.regs.a = v;
                let next = addr.wrapping_add(1);
                self.regs.set_hl(next);
                8
            }

            Instruction::LdHldA => {
                let addr = self.regs.get_hl();
                bus.write_byte(addr, self.regs.a);
                self.regs.set_hl(addr.wrapping_sub(1));
                8
            }
            Instruction::LdAHld => {
                let addr = self.regs.get_hl();
                self.regs.a = bus.read_byte(addr);
                self.regs.set_hl(addr.wrapping_sub(1));
                8
            }

            // NEW: LD (a16), SP  [0x08]  -> *(a16) = SP low, *(a16+1) = SP high
            Instruction::LdA16Sp => {
                let addr = self.fetch16(bus); // little-endian immediate
                let lo = (self.sp & 0x00ff) as u8;
                let hi = (self.sp >> 8) as u8;
                bus.write_byte(addr, lo);
                bus.write_byte(addr.wrapping_add(1), hi);
                20 // cycles
            }

            // src/cpu.rs (inside impl CPU, in exec match)
            Instruction::LdSpHl => {
                self.sp = self.regs.get_hl();
                8
            }

            _ => self.trap_unknown(DecodeError::UnknownOpcode(0x00, false)),
        }
    }

    fn trap_unknown(&self, e: DecodeError) -> ! {
        panic!("Unknown or unhandled instruction: {:?}", e);
    }

    fn write_to_target(&mut self, bus: &mut MemoryBus, tgt: LoadByteTarget, val: u8) {
        match tgt {
            LoadByteTarget::A => {
                self.regs.a = val;
            }
            LoadByteTarget::B => {
                self.regs.b = val;
            }
            LoadByteTarget::C => {
                self.regs.c = val;
            }
            LoadByteTarget::D => {
                self.regs.d = val;
            }
            LoadByteTarget::E => {
                self.regs.e = val;
            }
            LoadByteTarget::H => {
                self.regs.h = val;
            }
            LoadByteTarget::L => {
                self.regs.l = val;
            }
            LoadByteTarget::MemReg16(Reg16::HL) => {
                let addr = self.regs.get_hl();
                bus.write_byte(addr, val);
            }
            LoadByteTarget::MemReg16(Reg16::BC) => {
                let addr = self.regs.get_bc();
                bus.write_byte(addr, val);
            }
            LoadByteTarget::MemReg16(Reg16::DE) => {
                let addr = self.regs.get_de();
                bus.write_byte(addr, val);
            }
            LoadByteTarget::MemImm16 => {
                let addr = self.fetch16(bus);
                bus.write_byte(addr, val);
            }
            LoadByteTarget::MemImm8 => {
                let lo = self.fetch8(bus) as u16;
                let addr = 0xff00 | lo;
                bus.write_byte(addr, val);
            }

            LoadByteTarget::MemHighC => {
                // <-- NEW
                let addr = 0xff00 | (self.regs.c as u16);
                bus.write_byte(addr, val);
            }

            LoadByteTarget::HLI => {
                // Fallback placeholder if you still use HLI somewhere
                let addr = self.regs.get_hl();
                bus.write_byte(addr, val);
            }
            // Other Reg16 variants (AF, SP, PC) are not valid byte targets here
            LoadByteTarget::MemReg16(_) => {
                self.trap_unknown(DecodeError::UnknownOpcode(0x00, false))
            }
        }
    }

    fn read_from_source(&mut self, bus: &mut MemoryBus, src: LoadByteSource) -> u8 {
        match src {
            LoadByteSource::A => self.regs.a,
            LoadByteSource::B => self.regs.b,
            LoadByteSource::C => self.regs.c,
            LoadByteSource::D => self.regs.d,
            LoadByteSource::E => self.regs.e,
            LoadByteSource::H => self.regs.h,
            LoadByteSource::L => self.regs.l,
            LoadByteSource::MemReg16(Reg16::HL) => {
                let addr = self.regs.get_hl();
                bus.read_byte(addr)
            }
            LoadByteSource::MemReg16(Reg16::BC) => {
                let addr = self.regs.get_bc();
                bus.read_byte(addr)
            }
            LoadByteSource::MemReg16(Reg16::DE) => {
                let addr = self.regs.get_de();
                bus.read_byte(addr)
            }
            LoadByteSource::MemImm16 => {
                let addr = self.fetch16(bus);
                bus.read_byte(addr)
            }
            LoadByteSource::MemImm8 => {
                let low = self.fetch8(bus) as u16;
                let addr = 0xff00 | low;
                bus.read_byte(addr)
            }

            LoadByteSource::MemHighC => {
                // <-- NEW
                let addr = 0xff00 | (self.regs.c as u16);
                bus.read_byte(addr)
            }

            LoadByteSource::D8 => self.fetch8(bus),
            LoadByteSource::HLI => {
                // Fallback placeholder if you still use HLI somewhere
                let addr = self.regs.get_hl();
                bus.read_byte(addr)
            }
            // Other Reg16 variants (AF, SP, PC) are not valid byte sources here
            LoadByteSource::MemReg16(_) => {
                self.trap_unknown(DecodeError::UnknownOpcode(0x00, false))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_ld_a_d8() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();

        // Program: 3E 99  (LD A,0x99)
        let prog = [0x3e, 0x99];
        bus.load_rom(&prog);

        cpu.pc = 0x0000;
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.a, 0x99);
    }

    #[test]
    fn exec_ld_r_d8_group() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();

        // 06 11 | 0E 22 | 16 33 | 1E 44 | 26 55 | 2E 66 | 3E 77
        let prog = [
            0x06, 0x11, 0x0e, 0x22, 0x16, 0x33, 0x1e, 0x44, 0x26, 0x55, 0x2e, 0x66, 0x3e, 0x77,
        ];
        bus.load_rom(&prog);

        cpu.pc = 0x0000;
        for _ in 0..7 {
            cpu.step(&mut bus);
        }

        assert_eq!(cpu.regs.b, 0x11);
        assert_eq!(cpu.regs.c, 0x22);
        assert_eq!(cpu.regs.d, 0x33);
        assert_eq!(cpu.regs.e, 0x44);
        assert_eq!(cpu.regs.h, 0x55);
        assert_eq!(cpu.regs.l, 0x66);
        assert_eq!(cpu.regs.a, 0x77);
    }
}

#[cfg(test)]
mod tests_ld_indirect_bc_de {
    use super::*;

    #[test]
    fn ld_a_from_bc_and_de_then_store_back() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();

        // Data setup in VRAM area (OK to write)
        bus.write_byte(0x8001, 0xaa);
        bus.write_byte(0x8002, 0xbb);

        // Program: 0A (LD A,(BC)), 12 (LD (DE),A), 1A (LD A,(DE))
        let prog = [0x0a, 0x12, 0x1a];
        bus.load_rom(&prog);

        cpu.pc = 0x0000;
        cpu.regs.set_bc(0x8001);
        cpu.regs.set_de(0x8002);

        cpu.step(&mut bus); // 0x0A
        assert_eq!(cpu.regs.a, 0xaa);

        cpu.step(&mut bus); // 0x12
        assert_eq!(bus.read_byte(0x8002), 0xaa);

        cpu.step(&mut bus); // 0x1A
        assert_eq!(cpu.regs.a, 0xaa);
    }
}

#[cfg(test)]
mod tests_inc_dec_8bit_flags {
    use super::*;

    #[test]
    fn inc_sets_h_on_low_nibble_overflow_and_zero_on_00() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();

        // Pre-set carry to ensure INC doesn't change it
        cpu.regs.f.carry = true;

        // Program: 04 (INC B), 3C (INC A)
        let prog = [0x04, 0x3c];
        bus.load_rom(&prog);

        cpu.pc = 0x0000;
        cpu.regs.b = 0x0f; // will overflow low nibble -> H=1
        cpu.regs.a = 0xff; // -> 0x00 -> Z=1, H=1

        cpu.step(&mut bus); // INC B
        assert_eq!(cpu.regs.b, 0x10);
        assert_eq!(cpu.regs.f.zero, false);
        assert_eq!(cpu.regs.f.subtract, false);
        assert_eq!(cpu.regs.f.half_carry, true);
        assert_eq!(cpu.regs.f.carry, true); // unchanged

        cpu.step(&mut bus); // INC A
        assert_eq!(cpu.regs.a, 0x00);
        assert_eq!(cpu.regs.f.zero, true);
        assert_eq!(cpu.regs.f.subtract, false);
        assert_eq!(cpu.regs.f.half_carry, true);
        assert_eq!(cpu.regs.f.carry, true); // unchanged
    }

    #[test]
    fn dec_sets_h_on_borrow_from_bit4_and_zero_on_00() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();

        cpu.regs.f.carry = false; // ensure DEC preserves carry

        // Program: 0D (DEC C), 3D (DEC A)
        let prog = [0x0d, 0x3d];
        bus.load_rom(&prog);

        cpu.pc = 0x0000;
        cpu.regs.c = 0x00; // -> 0xFF; borrow -> H=1
        cpu.regs.a = 0x01; // -> 0x00; Z=1, H=0

        cpu.step(&mut bus); // DEC C
        assert_eq!(cpu.regs.c, 0xff);
        assert_eq!(cpu.regs.f.zero, false);
        assert_eq!(cpu.regs.f.subtract, true);
        assert_eq!(cpu.regs.f.half_carry, true);
        assert_eq!(cpu.regs.f.carry, false); // unchanged

        cpu.step(&mut bus); // DEC A
        assert_eq!(cpu.regs.a, 0x00);
        assert_eq!(cpu.regs.f.zero, true);
        assert_eq!(cpu.regs.f.subtract, true);
        assert_eq!(cpu.regs.f.half_carry, false);
        assert_eq!(cpu.regs.f.carry, false); // unchanged
    }

    #[test]
    fn inc_dec_hl_memory() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();

        // 34 = INC (HL), 35 = DEC (HL)
        let prog = [0x34, 0x35];
        bus.load_rom(&prog);

        cpu.pc = 0x0000;
        cpu.regs.set_hl(0x8000);
        bus.write_byte(0x8000, 0xff);

        cpu.regs.f.carry = true; // must remain unchanged

        cpu.step(&mut bus); // INC (HL): 0xFF -> 0x00 (Z=1, H=1)
        assert_eq!(bus.read_byte(0x8000), 0x00);
        assert_eq!(cpu.regs.f.zero, true);
        assert_eq!(cpu.regs.f.subtract, false);
        assert_eq!(cpu.regs.f.half_carry, true);
        assert_eq!(cpu.regs.f.carry, true);

        cpu.step(&mut bus); // DEC (HL): 0x00 -> 0xFF (N=1, H=1, Z=0)
        assert_eq!(bus.read_byte(0x8000), 0xff);
        assert_eq!(cpu.regs.f.zero, false);
        assert_eq!(cpu.regs.f.subtract, true);
        assert_eq!(cpu.regs.f.half_carry, true);
        assert_eq!(cpu.regs.f.carry, true);
    }

    #[test]
    fn jr_unconditional_forward() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();

        // 18 05 (JR +5)
        let rom = [0x18, 0x05];
        bus.load_rom(&rom);
        cpu.pc = 0x0000;

        // Before: PC=0x0000
        cpu.step(&mut bus);
        // After fetch of op+imm, PC was 0x0002, then +5 => 0x0007
        assert_eq!(cpu.pc, 0x0007);
    }

    #[test]
    fn jr_unconditional_backward() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();

        // Put code at 0x0003 so the backward jump is obvious:
        // 0003: 18 FB  (JR -5) => from 0x0005 back to 0x0000
        let mut rom = [0u8; 6];
        rom[3] = 0x18; // JR
        rom[4] = 0xfb; // -5 in two's complement
        bus.load_rom(&rom);

        cpu.pc = 0x0003;
        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x0000);
    }

    #[test]
    fn jr_nz_taken_and_not_taken() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();

        // 20 05 (JR NZ,+5)
        let rom = [0x20, 0x05];
        bus.load_rom(&rom);
        cpu.pc = 0x0000;

        // Case 1: Z=0 -> taken: 0x0002 + 5 = 0x0007
        cpu.regs.f.zero = false;
        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x0007);

        // Reset for not-taken case
        cpu.pc = 0x0000;
        cpu.regs.f.zero = true;

        // Case 2: Z=1 -> not taken: PC becomes 0x0002 after consuming imm
        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x0002);
    }

    #[test]
    fn jr_c_and_jr_nc() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();

        // Sequence: 38 02 | 30 FE
        // 38 02 = JR C,+2    (taken when C=1)   => PC: 0x0002 + 2 -> 0x0004
        // 30 FE = JR NC,-2   (taken when C=0)   => from 0x0005 -> 0x0003
        let rom = [0x38, 0x02, 0x00, 0x00, 0x30, 0xfe];
        bus.load_rom(&rom);
        cpu.pc = 0x0000;

        // C=1, take JR C
        cpu.regs.f.carry = true;
        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x0004);

        // Now at 0x0004; execute JR NC,-2 (0x30, 0xFE).
        // For JR NC to be taken, set C=0:
        cpu.regs.f.carry = false;
        cpu.step(&mut bus);
        // 0x0004 + 2 = 0x0006 (PC after immediate), then -2 => 0x0004
        assert_eq!(cpu.pc, 0x0004);
    }
}

#[cfg(test)]
mod tests_ld_hl_plus {
    use super::*;

    #[test]
    fn ld_hli_a_writes_then_increments_hl() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();

        // 22 = LD (HL+),A
        let rom = [0x22];
        bus.load_rom(&rom);

        cpu.pc = 0x0000;
        cpu.regs.set_hl(0x8000);
        cpu.regs.a = 0x5a;
        bus.write_byte(0x8000, 0x00);

        cpu.step(&mut bus);

        assert_eq!(bus.read_byte(0x8000), 0x5a);
        assert_eq!(cpu.regs.get_hl(), 0x8001);
    }

    #[test]
    fn ld_a_hli_reads_then_increments_hl() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();

        // 2A = LD A,(HL+)
        let rom = [0x2a];
        bus.load_rom(&rom);

        cpu.pc = 0x0000;
        cpu.regs.set_hl(0x8000);
        bus.write_byte(0x8000, 0xab);

        cpu.step(&mut bus);

        assert_eq!(cpu.regs.a, 0xab);
        assert_eq!(cpu.regs.get_hl(), 0x8001);
    }
}

#[cfg(test)]
mod tests_ld_hl_minus {
    use super::*;

    #[test]
    fn ld_hld_a_writes_then_decrements_hl() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();

        // 32 = LD (HL-),A
        let rom = [0x32];
        bus.load_rom(&rom);

        cpu.pc = 0x0000;
        cpu.regs.set_hl(0x8001);
        cpu.regs.a = 0x6c;
        bus.write_byte(0x8001, 0x00);

        cpu.step(&mut bus);

        assert_eq!(bus.read_byte(0x8001), 0x6c);
        assert_eq!(cpu.regs.get_hl(), 0x8000);
    }

    #[test]
    fn ld_a_hld_reads_then_decrements_hl() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();

        // 3A = LD A,(HL-)
        let rom = [0x3a];
        bus.load_rom(&rom);

        cpu.pc = 0x0000;
        cpu.regs.set_hl(0x8001);
        bus.write_byte(0x8001, 0xb7);

        cpu.step(&mut bus);

        assert_eq!(cpu.regs.a, 0xb7);
        assert_eq!(cpu.regs.get_hl(), 0x8000);
    }
}

#[cfg(test)]
mod tests_ldh {
    use super::*;

    #[test]
    fn ldh_a8_a_writes_to_ff00_plus_imm() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();

        // E0 10  => LDH (0xFF10),A
        let rom = [0xe0, 0x10];
        bus.load_rom(&rom);

        cpu.pc = 0x0000;
        cpu.regs.a = 0x5a;

        cpu.step(&mut bus);
        assert_eq!(bus.read_byte(0xff10), 0x5a);
    }

    #[test]
    fn ldh_a_imm_reads_from_ff00_plus_imm() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();

        // F0 10  => LDH A,(0xFF10)
        let rom = [0xf0, 0x10];
        bus.load_rom(&rom);

        cpu.pc = 0x0000;
        bus.write_byte(0xff10, 0xab);

        cpu.step(&mut bus);
        assert_eq!(cpu.regs.a, 0xab);
    }

    #[test]
    fn ld_c_indexed_store_and_load() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();

        // E2 ; F2  => LD (C),A ; LD A,(C)
        let rom = [0xe2, 0xf2];
        bus.load_rom(&rom);

        cpu.pc = 0x0000;
        cpu.regs.c = 0x34;

        // Store A at FF34
        cpu.regs.a = 0x66;
        cpu.step(&mut bus);
        assert_eq!(bus.read_byte(0xff34), 0x66);

        // Overwrite memory, then read back into A
        bus.write_byte(0xff34, 0x99);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.a, 0x99);
    }
}

#[cfg(test)]
mod tests_ld_a16_sp {
    use super::*;

    #[test]
    fn ld_a16_sp_writes_sp_little_endian() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();

        // Program:
        // 31 FE FF        LD SP,0xFFFE
        // 08 00 80        LD (0x8000),SP
        let rom = [0x31, 0xfe, 0xff, 0x08, 0x00, 0x80];
        bus.load_rom(&rom);
        cpu.pc = 0x0000;

        // Step 1: LD SP,0xFFFE
        cpu.step(&mut bus);
        assert_eq!(cpu.sp, 0xfffe);

        // Step 2: LD (0x8000),SP
        cpu.step(&mut bus);
        assert_eq!(bus.read_byte(0x8000), 0xfe); // low byte
        assert_eq!(bus.read_byte(0x8001), 0xff); // high byte
    }
}

// src/cpu.rs (append under #[cfg(test)])
#[cfg(test)]
mod tests_ld_sp_hl {
    use super::*;

    #[test]
    fn ld_sp_hl_copies_hl_into_sp() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();

        // Program:
        // 21 34 12    LD HL,0x1234
        // F9          LD SP,HL
        let rom = [0x21, 0x34, 0x12, 0xf9];
        bus.load_rom(&rom);

        cpu.pc = 0x0000;

        cpu.step(&mut bus); // LD HL,0x1234
        assert_eq!(cpu.regs.get_hl(), 0x1234);

        cpu.step(&mut bus); // LD SP,HL
        assert_eq!(cpu.sp, 0x1234);
    }
}
