// src/cpu.rs
use crate::bus::MemoryBus;
use crate::instruction::{ DecodeError, Instruction };
use crate::registers::Registers;
use crate::types::{
    IncDecTarget,
    JumpTest,
    LoadByteSource,
    LoadByteTarget,
    LoadType,
    Reg16,
    StackTarget,
};

// In struct CPU (add one field)
pub struct CPU {
    pub regs: Registers,
    pub pc: u16,
    pub sp: u16,
    pub ime: bool,
    pub halted: bool,
    ei_delay: u8, // NEW: counts down instructions after EI
}

impl CPU {
    pub fn new() -> Self {
        Self {
            regs: Registers::default(),
            pc: 0x0000,
            sp: 0xfffe,
            ime: false,
            halted: false,
            ei_delay: 0, // NEW
        }
    }

    #[inline]
    fn read_ie_if(bus: &mut MemoryBus) -> (u8, u8) {
        let ie = bus.read_byte(0xffff);
        let iflag = bus.read_byte(0xff0f);
        (ie, iflag)
    }

    #[inline]
    fn write_if(bus: &mut MemoryBus, val: u8) {
        bus.write_byte(0xff0f, val);
    }

    #[inline]
    fn pending_interrupt_mask(bus: &mut MemoryBus) -> u8 {
        let (ie, iflag) = Self::read_ie_if(bus);
        ie & iflag
    }

    fn service_interrupt(&mut self, bus: &mut MemoryBus) -> u32 {
        // Priority: bit0..bit4 => vectors 0x40,0x48,0x50,0x58,0x60
        let (ie, iflag) = Self::read_ie_if(bus);
        let pending = ie & iflag;
        if pending == 0 {
            return 0;
        }

        let (bit, vector) = if (pending & 0x01) != 0 {
            (0, 0x0040)
        } else if (pending & 0x02) != 0 {
            (1, 0x0048)
        } else if (pending & 0x04) != 0 {
            (2, 0x0050)
        } else if (pending & 0x08) != 0 {
            (3, 0x0058)
        } else {
            (4, 0x0060)
        };

        // Clear IF bit
        let new_if = iflag & !(1 << bit);
        Self::write_if(bus, new_if);

        // Enter ISR
        self.ime = false;
        self.push16(bus, self.pc);
        self.pc = vector;
        20 // cycles for ISR entry
    }

    /// Execute a single instruction and return consumed cycles.
    pub fn step(&mut self, bus: &mut MemoryBus) -> u32 {
        // 1) Service interrupt if IME=1 and any pending
        if self.ime {
            let pending = Self::pending_interrupt_mask(bus);
            if pending != 0 {
                let c = self.service_interrupt(bus);
                bus.tick(c); // <-- tick timer
                return c;
            }
        }

        // 2) HALT handling
        if self.halted {
            let pending = Self::pending_interrupt_mask(bus);
            if pending != 0 {
                self.halted = false;
            } else {
                bus.tick(4); // <-- burn time while halted
                return 4;
            }
        }

        // 3) Fetch/decode/execute
        let op = self.fetch8(bus);
        let cycles = if op == 0xcb {
            let cb = self.fetch8(bus);
            match Instruction::from_byte(cb, true) {
                Ok(insn) => self.exec(insn, bus),
                Err(e) => self.trap_unknown(e),
            }
        } else {
            match Instruction::from_byte(op, false) {
                Ok(insn) => self.exec(insn, bus),
                Err(e) => self.trap_unknown(e),
            }
        };

        // 4) EI delayed enabling
        if self.ei_delay > 0 {
            self.ei_delay = self.ei_delay.saturating_sub(1);
            if self.ei_delay == 0 {
                self.ime = true;
            }
        }

        // 5) Tick on-SoC components by consumed cycles
        bus.tick(cycles);

        cycles
    }

    #[inline]
    fn fetch8(&mut self, bus: &mut MemoryBus) -> u8 {
        let b = bus.read_byte(self.pc);
        self.pc = self.pc.wrapping_add(1);
        b
    }

    #[inline]
    fn fetch16(&mut self, bus: &mut MemoryBus) -> u16 {
        // Little-endian: lo then hi
        let lo = self.fetch8(bus) as u16;
        let hi = self.fetch8(bus) as u16;
        (hi << 8) | lo
    }

    fn push16(&mut self, bus: &mut MemoryBus, value: u16) {
        // DMG push: pre-decrement then write hi, pre-decrement then write lo
        self.sp = self.sp.wrapping_sub(1);
        bus.write_byte(self.sp, (value >> 8) as u8);
        self.sp = self.sp.wrapping_sub(1);
        bus.write_byte(self.sp, (value & 0x00ff) as u8);
    }

    fn pop16(&mut self, bus: &mut MemoryBus) -> u16 {
        // Pop: read lo then hi, post-increment SP each read
        let lo = bus.read_byte(self.sp) as u16;
        self.sp = self.sp.wrapping_add(1);
        let hi = bus.read_byte(self.sp) as u16;
        self.sp = self.sp.wrapping_add(1);
        (hi << 8) | lo
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

    // ---------- ALU helpers (set flags, return result) ----------
    #[inline]
    fn alu_add8(&mut self, a: u8, b: u8) -> u8 {
        let (res, c) = a.overflowing_add(b);
        let h = (a & 0x0f) + (b & 0x0f) > 0x0f;
        self.regs.f.zero = res == 0;
        self.regs.f.subtract = false;
        self.regs.f.half_carry = h;
        self.regs.f.carry = c;
        res
    }

    #[inline]
    fn alu_adc8(&mut self, a: u8, b: u8) -> u8 {
        let c_in = if self.regs.f.carry { 1 } else { 0 };
        let (t, c1) = a.overflowing_add(b);
        let (res, c2) = t.overflowing_add(c_in);
        let h = (a & 0x0f) + (b & 0x0f) + c_in > 0x0f;
        self.regs.f.zero = res == 0;
        self.regs.f.subtract = false;
        self.regs.f.half_carry = h;
        self.regs.f.carry = c1 || c2;
        res
    }

    #[inline]
    fn alu_sub8(&mut self, a: u8, b: u8) -> u8 {
        let (res, borrow) = a.overflowing_sub(b);
        let h = a & 0x0f < b & 0x0f;
        self.regs.f.zero = res == 0;
        self.regs.f.subtract = true;
        self.regs.f.half_carry = h;
        self.regs.f.carry = borrow;
        res
    }

    #[inline]
    fn alu_sbc8(&mut self, a: u8, b: u8) -> u8 {
        let c_in = if self.regs.f.carry { 1 } else { 0 };
        let (t, b1) = a.overflowing_sub(b);
        let (res, b2) = t.overflowing_sub(c_in);
        // half-borrow if low nibble of a < low nibble of b + carry
        let h = a & 0x0f < (b & 0x0f).wrapping_add(c_in as u8);
        self.regs.f.zero = res == 0;
        self.regs.f.subtract = true;
        self.regs.f.half_carry = h;
        self.regs.f.carry = b1 || b2;
        res
    }

    #[inline]
    fn alu_and8(&mut self, a: u8, b: u8) -> u8 {
        let res = a & b;
        self.regs.f.zero = res == 0;
        self.regs.f.subtract = false;
        self.regs.f.half_carry = true;
        self.regs.f.carry = false;
        res
    }

    #[inline]
    fn alu_xor8(&mut self, a: u8, b: u8) -> u8 {
        let res = a ^ b;
        self.regs.f.zero = res == 0;
        self.regs.f.subtract = false;
        self.regs.f.half_carry = false;
        self.regs.f.carry = false;
        res
    }

    #[inline]
    fn alu_or8(&mut self, a: u8, b: u8) -> u8 {
        let res = a | b;
        self.regs.f.zero = res == 0;
        self.regs.f.subtract = false;
        self.regs.f.half_carry = false;
        self.regs.f.carry = false;
        res
    }

    #[inline]
    fn alu_cp8(&mut self, a: u8, b: u8) {
        let (res, borrow) = a.overflowing_sub(b);
        let h = a & 0x0f < b & 0x0f;
        self.regs.f.zero = res == 0;
        self.regs.f.subtract = true;
        self.regs.f.half_carry = h;
        self.regs.f.carry = borrow;
    }

    // ---------- LD helpers ----------
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
                let addr = 0xff00 | (self.regs.c as u16);
                bus.write_byte(addr, val);
            }
            LoadByteTarget::HLI => {
                let addr = self.regs.get_hl();
                bus.write_byte(addr, val);
            }
            // AF, SP, PC not valid byte targets via this enum
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
                let lo = self.fetch8(bus) as u16;
                let addr = 0xff00 | lo;
                bus.read_byte(addr)
            }
            LoadByteSource::MemHighC => {
                let addr = 0xff00 | (self.regs.c as u16);
                bus.read_byte(addr)
            }
            LoadByteSource::D8 => self.fetch8(bus),
            LoadByteSource::HLI => {
                let addr = self.regs.get_hl();
                bus.read_byte(addr)
            }
            LoadByteSource::MemReg16(_) => {
                self.trap_unknown(DecodeError::UnknownOpcode(0x00, false))
            }
        }
    }

    #[inline]
    fn src_involves_hl_mem(src: &LoadByteSource) -> bool {
        matches!(src, LoadByteSource::MemReg16(Reg16::HL))
    }

    fn exec(&mut self, insn: Instruction, bus: &mut MemoryBus) -> u32 {
        match insn {
            // ---------- Misc ----------
            Instruction::NOP => 4,
            Instruction::HALT => {
                self.halted = true;
                4
            }

            // ---------- HL auto-increment/decrement ----------
            Instruction::LdHliA => {
                let addr = self.regs.get_hl();
                bus.write_byte(addr, self.regs.a);
                self.regs.set_hl(addr.wrapping_add(1));
                8
            }
            Instruction::LdAHli => {
                let addr = self.regs.get_hl();
                self.regs.a = bus.read_byte(addr);
                self.regs.set_hl(addr.wrapping_add(1));
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

            // ---------- Addressed pointer ops ----------
            // LD (a16),SP
            Instruction::LdA16Sp => {
                let addr = self.fetch16(bus);
                let lo = (self.sp & 0x00ff) as u8;
                let hi = (self.sp >> 8) as u8;
                bus.write_byte(addr, lo);
                bus.write_byte(addr.wrapping_add(1), hi);
                20
            }
            // LD SP,HL
            Instruction::LdSpHl => {
                self.sp = self.regs.get_hl();
                8
            }

            // ---------- 16-bit immediate loads ----------
            Instruction::LD16Imm(Reg16::BC) => {
                let v = self.fetch16(bus);
                self.regs.set_bc(v);
                12
            }
            Instruction::LD16Imm(Reg16::DE) => {
                let v = self.fetch16(bus);
                self.regs.set_de(v);
                12
            }
            Instruction::LD16Imm(Reg16::HL) => {
                let v = self.fetch16(bus);
                self.regs.set_hl(v);
                12
            }
            Instruction::LD16Imm(Reg16::SP) => {
                let v = self.fetch16(bus);
                self.sp = v;
                12
            }

            // ---------- 8-bit INC ----------
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

            // ---------- 8-bit DEC ----------
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

            // Keep your 16-bit INC (BC) case
            Instruction::INC(IncDecTarget::BC) => {
                let bc = self.regs.get_bc().wrapping_add(1);
                self.regs.set_bc(bc);
                8
            }

            // ---------- LD r,d8 (explicit to ensure 8 cycles) ----------
            Instruction::LD(LoadType::Byte(LoadByteTarget::A, LoadByteSource::D8)) => {
                self.regs.a = self.fetch8(bus);
                8
            }
            Instruction::LD(LoadType::Byte(LoadByteTarget::B, LoadByteSource::D8)) => {
                self.regs.b = self.fetch8(bus);
                8
            }
            Instruction::LD(LoadType::Byte(LoadByteTarget::C, LoadByteSource::D8)) => {
                self.regs.c = self.fetch8(bus);
                8
            }
            Instruction::LD(LoadType::Byte(LoadByteTarget::D, LoadByteSource::D8)) => {
                self.regs.d = self.fetch8(bus);
                8
            }
            Instruction::LD(LoadType::Byte(LoadByteTarget::E, LoadByteSource::D8)) => {
                self.regs.e = self.fetch8(bus);
                8
            }
            Instruction::LD(LoadType::Byte(LoadByteTarget::H, LoadByteSource::D8)) => {
                self.regs.h = self.fetch8(bus);
                8
            }
            Instruction::LD(LoadType::Byte(LoadByteTarget::L, LoadByteSource::D8)) => {
                self.regs.l = self.fetch8(bus);
                8
            }

            // ---------- Absolute addressing ----------
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

            // ---------- High-RAM I/O: (FF00+a8), (FF00+C) ----------
            Instruction::LD(LoadType::Byte(LoadByteTarget::MemImm8, LoadByteSource::A)) => {
                self.write_to_target(bus, LoadByteTarget::MemImm8, self.regs.a);
                12
            }
            Instruction::LD(LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemImm8)) => {
                let v = self.read_from_source(bus, LoadByteSource::MemImm8);
                self.regs.a = v;
                12
            }
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

            // ---------- (BC)/(DE) ----------
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

            // ---------- Generic LD r,r' and (HL) paths ----------
            Instruction::LD(LoadType::Byte(tgt, src)) => {
                let involves_hl_mem =
                    matches!(tgt, LoadByteTarget::MemReg16(Reg16::HL)) ||
                    matches!(src, LoadByteSource::MemReg16(Reg16::HL));

                // D8 / immediate or other special forms are handled by explicit arms above.
                // This generic arm covers register <-> register and (HL) forms.
                let v = self.read_from_source(bus, src);
                self.write_to_target(bus, tgt, v);
                if involves_hl_mem {
                    8
                } else {
                    4
                }
            }

            // ---------- ALU group ----------
            Instruction::AddA(src) => {
                let involves_hl = Self::src_involves_hl_mem(&src);
                let b = self.read_from_source(bus, src);
                let a = self.regs.a;
                self.regs.a = self.alu_add8(a, b);
                if matches!(src, LoadByteSource::D8) {
                    8
                } else if involves_hl {
                    8
                } else {
                    4
                }
            }
            Instruction::AdcA(src) => {
                let involves_hl = Self::src_involves_hl_mem(&src);
                let b = self.read_from_source(bus, src);
                let a = self.regs.a;
                self.regs.a = self.alu_adc8(a, b);
                if matches!(src, LoadByteSource::D8) {
                    8
                } else if involves_hl {
                    8
                } else {
                    4
                }
            }
            Instruction::SubA(src) => {
                let involves_hl = Self::src_involves_hl_mem(&src);
                let b = self.read_from_source(bus, src);
                let a = self.regs.a;
                self.regs.a = self.alu_sub8(a, b);
                if matches!(src, LoadByteSource::D8) {
                    8
                } else if involves_hl {
                    8
                } else {
                    4
                }
            }
            Instruction::SbcA(src) => {
                let involves_hl = Self::src_involves_hl_mem(&src);
                let b = self.read_from_source(bus, src);
                let a = self.regs.a;
                self.regs.a = self.alu_sbc8(a, b);
                if matches!(src, LoadByteSource::D8) {
                    8
                } else if involves_hl {
                    8
                } else {
                    4
                }
            }
            Instruction::AndA(src) => {
                let involves_hl = Self::src_involves_hl_mem(&src);
                let b = self.read_from_source(bus, src);
                let a = self.regs.a;
                self.regs.a = self.alu_and8(a, b);
                if matches!(src, LoadByteSource::D8) {
                    8
                } else if involves_hl {
                    8
                } else {
                    4
                }
            }
            Instruction::XorA(src) => {
                let involves_hl = Self::src_involves_hl_mem(&src);
                let b = self.read_from_source(bus, src);
                let a = self.regs.a;
                self.regs.a = self.alu_xor8(a, b);
                if matches!(src, LoadByteSource::D8) {
                    8
                } else if involves_hl {
                    8
                } else {
                    4
                }
            }
            Instruction::OrA(src) => {
                let involves_hl = Self::src_involves_hl_mem(&src);
                let b = self.read_from_source(bus, src);
                let a = self.regs.a;
                self.regs.a = self.alu_or8(a, b);
                if matches!(src, LoadByteSource::D8) {
                    8
                } else if involves_hl {
                    8
                } else {
                    4
                }
            }
            Instruction::CpA(src) => {
                let involves_hl = Self::src_involves_hl_mem(&src);
                let b = self.read_from_source(bus, src);
                let a = self.regs.a;
                self.alu_cp8(a, b); // A unchanged
                if matches!(src, LoadByteSource::D8) {
                    8
                } else if involves_hl {
                    8
                } else {
                    4
                }
            }

            // ---------- Relative jumps ----------
            Instruction::JR(cond) => {
                let disp = self.fetch8(bus) as i8 as i32; // always consume disp
                if self.cond_met(cond) {
                    let new_pc = ((self.pc as i32) + disp).rem_euclid(0x1_0000) as u16;
                    self.pc = new_pc;
                    12
                } else {
                    8
                }
            }

            // ---------- JP a16 / JP cc,a16 ----------
            Instruction::JP(JumpTest::Always) => {
                let addr = self.fetch16(bus);
                self.pc = addr;
                16
            }
            Instruction::JP(cond) => {
                let addr = self.fetch16(bus); // always fetch immediate
                if self.cond_met(cond) {
                    self.pc = addr;
                    16
                } else {
                    12
                }
            }

            // ---------- CALL a16 / CALL cc,a16 ----------
            Instruction::CALL(JumpTest::Always) => {
                let addr = self.fetch16(bus);
                self.push16(bus, self.pc);
                self.pc = addr;
                24
            }
            Instruction::CALL(cond) => {
                let addr = self.fetch16(bus);
                if self.cond_met(cond) {
                    self.push16(bus, self.pc);
                    self.pc = addr;
                    24
                } else {
                    12
                }
            }

            // ---------- RET / RET cc ----------
            Instruction::RET(JumpTest::Always) => {
                let addr = self.pop16(bus);
                self.pc = addr;
                16
            }
            Instruction::RET(cond) => {
                if self.cond_met(cond) {
                    let addr = self.pop16(bus);
                    self.pc = addr;
                    20
                } else {
                    8
                }
            }

            // ---------- PUSH / POP ----------
            Instruction::POP(StackTarget::BC) => {
                let v = self.pop16(bus);
                self.regs.set_bc(v);
                12
            }
            Instruction::POP(StackTarget::DE) => {
                let v = self.pop16(bus);
                self.regs.set_de(v);
                12
            }
            Instruction::POP(StackTarget::HL) => {
                let v = self.pop16(bus);
                self.regs.set_hl(v);
                12
            }
            Instruction::POP(StackTarget::AF) => {
                let v = self.pop16(bus);
                self.regs.set_af(v);
                12
            }

            Instruction::PUSH(StackTarget::BC) => {
                self.push16(bus, self.regs.get_bc());
                16
            }
            Instruction::PUSH(StackTarget::DE) => {
                self.push16(bus, self.regs.get_de());
                16
            }
            Instruction::PUSH(StackTarget::HL) => {
                self.push16(bus, self.regs.get_hl());
                16
            }
            Instruction::PUSH(StackTarget::AF) => {
                self.push16(bus, self.regs.get_af());
                16
            }

            Instruction::Rst(vec) => {
                // PC currently points at the next instruction (fetch already advanced it).
                self.push16(bus, self.pc);
                self.pc = vec;
                16
            }

            // --- EI / DI / RETI ---
            Instruction::EI => {
                // Schedule IME enabling after the *next* instruction completes.
                // (Two ticks because we decrement once right after EI, then once after the next instruction.)
                self.ei_delay = 2;
                4
            }
            Instruction::DI => {
                self.ime = false;
                self.ei_delay = 0;
                4
            }
            Instruction::RETI => {
                let addr = self.pop16(bus);
                self.pc = addr;
                self.ime = true;
                16
            }

            // ---------- Fallback ----------
            _ => self.trap_unknown(DecodeError::UnknownOpcode(0x00, false)),
        }
    }

    // INC helpers use the same flag rules as 8-bit INC/DEC instructions
    #[inline]
    fn alu_inc8(&mut self, v: u8) -> u8 {
        let res = v.wrapping_add(1);
        let c = self.regs.f.carry; // preserve C
        self.regs.f.zero = res == 0;
        self.regs.f.subtract = false;
        self.regs.f.half_carry = (v & 0x0f) == 0x0f;
        self.regs.f.carry = c;
        res
    }

    #[inline]
    fn alu_dec8(&mut self, v: u8) -> u8 {
        let res = v.wrapping_sub(1);
        let c = self.regs.f.carry; // preserve C
        self.regs.f.zero = res == 0;
        self.regs.f.subtract = true;
        self.regs.f.half_carry = (v & 0x0f) == 0x00;
        self.regs.f.carry = c;
        res
    }

    fn trap_unknown(&self, e: DecodeError) -> ! {
        panic!("Unknown or unhandled instruction: {:?}", e);
    }
}

#[cfg(test)]
mod cpu_tests {
    use super::*;

    // -----------------------
    // Helpers (optional)
    // -----------------------
    fn rom(bytes: &[u8]) -> Vec<u8> {
        bytes.to_vec()
    }

    #[test]
    fn services_vblank_interrupt_and_clears_if() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();

        // Enable VBLANK only; request it.
        bus.write_byte(0xffff, 0b0000_0001); // IE
        bus.write_byte(0xff0f, 0b0000_0001); // IF
        cpu.ime = true;
        cpu.pc = 0x0100;

        // Should service immediately: push 0x0100, jump 0x0040
        let taken = cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x0040);
        assert_eq!(taken, 20);
        // IF bit cleared
        assert_eq!(bus.read_byte(0xff0f) & 0b0000_0001, 0);
    }

    #[test]
    fn interrupt_priority_vblank_over_timer() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();
        // IE: VBLANK + TIMER ; IF: both
        bus.write_byte(0xffff, 0b0000_0101);
        bus.write_byte(0xff0f, 0b0000_0101);
        cpu.ime = true;

        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x0040); // VBLANK first
        // Next service would be 0x0050 if we called step again with IME=1 and IF still had TIMER set.
    }

    #[test]
    fn ei_is_delayed_until_one_instruction_after_next() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();

        // Program: FB (EI) ; 00 (NOP) ; 00 (NOP)
        bus.load_rom(&[0xfb, 0x00, 0x00]);

        // Prepare an interrupt pending from the start
        bus.write_byte(0xffff, 0b0000_0001); // IE VBLANK
        bus.write_byte(0xff0f, 0b0000_0001); // IF VBLANK

        cpu.ime = false;
        cpu.pc = 0x0000;

        // Step 1: EI executes; IME still false; no service
        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x0001);
        assert_eq!(cpu.ime, false);

        // Step 2: first NOP executes; only *after* this completes should IME become true
        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x0002);
        assert_eq!(cpu.ime, true);

        // Step 3: now IME=true and interrupt pending => service to 0x0040
        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x0040);
    }

    #[test]
    fn halt_wakes_on_pending_interrupt_even_with_ime_zero() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();

        // Program: 76 (HALT) ; 00 (NOP)
        bus.load_rom(&[0x76, 0x00]);
        cpu.ime = false;
        cpu.pc = 0x0000;

        // No interrupt yet: step once → enter HALT and burn cycles
        let _c = cpu.step(&mut bus);
        assert!(cpu.halted);

        // Now request+enable an interrupt while IME=0
        bus.write_byte(0xffff, 0b0000_0001); // IE VBLANK
        bus.write_byte(0xff0f, 0b0000_0001); // IF VBLANK

        // Next step should *wake* from HALT and execute the next instruction (not service yet)
        let _c2 = cpu.step(&mut bus);
        assert!(!cpu.halted);
        assert_eq!(cpu.pc, 0x0002); // executed the NOP at 0x0001
        // Interrupt can be serviced later once IME becomes 1 (e.g., after DI/EI flow)
    }

    #[test]
    fn reti_enables_ime_and_returns() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();

        // Put RETI at 0x0040 (simulate an ISR)
        bus.write_byte(0x0040, 0xd9); // RETI

        // Simulate we are in ISR: PC=0x0040; return address 0x1234 on stack; IME=0
        cpu.pc = 0x0040;
        cpu.sp = 0xfffc;
        bus.write_byte(0xfffc, 0x34); // lo
        bus.write_byte(0xfffd, 0x12); // hi
        cpu.ime = false;

        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x1234);
        assert!(cpu.ime);
    }

    #[test]
    fn rst_38_pushes_return_address_and_jumps() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();

        // FF = RST 38h
        bus.load_rom(&[0xff]);
        cpu.pc = 0x0000;
        cpu.sp = 0xfffe;

        cpu.step(&mut bus);

        // After fetch, PC was 0x0001 and should be pushed
        assert_eq!(cpu.sp, 0xfffc);
        let lo = bus.read_byte(0xfffc) as u16;
        let hi = bus.read_byte(0xfffd) as u16;
        assert_eq!((hi << 8) | lo, 0x0001);

        // PC must jump to 0x0038
        assert_eq!(cpu.pc, 0x0038);
    }

    #[test]
    fn rst_00_pushes_and_jumps_to_zero() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();

        // C7 = RST 00h ; place a NOP at 0x0000 just for sanity
        bus.load_rom(&[0xc7]);
        cpu.pc = 0x0000;
        cpu.sp = 0xfff0;

        cpu.step(&mut bus);

        // Pushed return address 0x0001
        assert_eq!(cpu.sp, 0xffee);
        let lo = bus.read_byte(0xffee) as u16;
        let hi = bus.read_byte(0xffef) as u16;
        assert_eq!((hi << 8) | lo, 0x0001);

        // Jump target
        assert_eq!(cpu.pc, 0x0000);
    }

    // -----------------------
    // LD immediates (8-bit)
    // -----------------------
    #[test]
    fn exec_ld_a_d8() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();
        bus.load_rom(&rom(&[0x3e, 0x99])); // LD A,0x99
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.a, 0x99);
    }

    #[test]
    fn exec_ld_r_d8_group() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();
        // 06 11 | 0E 22 | 16 33 | 1E 44 | 26 55 | 2E 66 | 3E 77
        let bytes = [
            0x06, 0x11, 0x0e, 0x22, 0x16, 0x33, 0x1e, 0x44, 0x26, 0x55, 0x2e, 0x66, 0x3e, 0x77,
        ];
        bus.load_rom(&bytes);
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

    // -----------------------
    // LD indirect via BC/DE
    // -----------------------
    #[test]
    fn ld_a_from_bc_and_de_then_store_back() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();
        // data in VRAM area (writable)
        bus.write_byte(0x8001, 0xaa);
        bus.write_byte(0x8002, 0xbb);

        // 0A (LD A,(BC)), 12 (LD (DE),A), 1A (LD A,(DE))
        bus.load_rom(&rom(&[0x0a, 0x12, 0x1a]));
        cpu.regs.set_bc(0x8001);
        cpu.regs.set_de(0x8002);

        cpu.step(&mut bus); // A = [BC] = 0xAA
        assert_eq!(cpu.regs.a, 0xaa);

        cpu.step(&mut bus); // [DE] = A -> [0x8002] = 0xAA
        assert_eq!(bus.read_byte(0x8002), 0xaa);

        cpu.step(&mut bus); // A = [DE] = 0xAA
        assert_eq!(cpu.regs.a, 0xaa);
    }

    // -----------------------
    // INC/DEC (8-bit) flags
    // -----------------------
    #[test]
    fn inc_sets_h_on_low_nibble_overflow_and_zero_on_00() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();
        // INC B; INC A
        bus.load_rom(&rom(&[0x04, 0x3c]));
        cpu.regs.f.carry = true; // carry must be preserved
        cpu.regs.b = 0x0f; // 0x0F -> 0x10 (H=1)
        cpu.regs.a = 0xff; // 0xFF -> 0x00 (Z=1, H=1)

        cpu.step(&mut bus);
        assert_eq!(cpu.regs.b, 0x10);
        assert_eq!(cpu.regs.f.zero, false);
        assert_eq!(cpu.regs.f.subtract, false);
        assert_eq!(cpu.regs.f.half_carry, true);
        assert_eq!(cpu.regs.f.carry, true);

        cpu.step(&mut bus);
        assert_eq!(cpu.regs.a, 0x00);
        assert_eq!(cpu.regs.f.zero, true);
        assert_eq!(cpu.regs.f.subtract, false);
        assert_eq!(cpu.regs.f.half_carry, true);
        assert_eq!(cpu.regs.f.carry, true);
    }

    #[test]
    fn dec_sets_h_on_borrow_from_bit4_and_zero_on_00() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();
        // DEC C; DEC A
        bus.load_rom(&rom(&[0x0d, 0x3d]));
        cpu.regs.f.carry = false; // carry must be preserved
        cpu.regs.c = 0x00; // -> 0xFF (H=1, Z=0)
        cpu.regs.a = 0x01; // -> 0x00 (Z=1, H=0)

        cpu.step(&mut bus);
        assert_eq!(cpu.regs.c, 0xff);
        assert_eq!(cpu.regs.f.zero, false);
        assert_eq!(cpu.regs.f.subtract, true);
        assert_eq!(cpu.regs.f.half_carry, true);
        assert_eq!(cpu.regs.f.carry, false);

        cpu.step(&mut bus);
        assert_eq!(cpu.regs.a, 0x00);
        assert_eq!(cpu.regs.f.zero, true);
        assert_eq!(cpu.regs.f.subtract, true);
        assert_eq!(cpu.regs.f.half_carry, false);
        assert_eq!(cpu.regs.f.carry, false);
    }

    #[test]
    fn inc_dec_hl_memory() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();
        // INC (HL); DEC (HL)
        bus.load_rom(&rom(&[0x34, 0x35]));
        cpu.regs.set_hl(0x8000);
        bus.write_byte(0x8000, 0xff);
        cpu.regs.f.carry = true;

        cpu.step(&mut bus); // 0xFF -> 0x00
        assert_eq!(bus.read_byte(0x8000), 0x00);
        assert!(cpu.regs.f.zero);
        assert!(!cpu.regs.f.subtract);
        assert!(cpu.regs.f.half_carry);
        assert!(cpu.regs.f.carry);

        cpu.step(&mut bus); // 0x00 -> 0xFF
        assert_eq!(bus.read_byte(0x8000), 0xff);
        assert!(!cpu.regs.f.zero);
        assert!(cpu.regs.f.subtract);
        assert!(cpu.regs.f.half_carry);
        assert!(cpu.regs.f.carry);
    }

    // -----------------------
    // HL auto-inc/dec
    // -----------------------
    #[test]
    fn ld_hli_a_and_ld_a_hli() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();
        // 22 ; 2A
        bus.load_rom(&rom(&[0x22, 0x2a]));
        cpu.regs.set_hl(0x8000);
        cpu.regs.a = 0x5a;
        cpu.step(&mut bus); // LD (HL+),A
        assert_eq!(bus.read_byte(0x8000), 0x5a);
        assert_eq!(cpu.regs.get_hl(), 0x8001);

        bus.write_byte(0x8001, 0xab);
        cpu.step(&mut bus); // LD A,(HL+)
        assert_eq!(cpu.regs.a, 0xab);
        assert_eq!(cpu.regs.get_hl(), 0x8002);
    }

    #[test]
    fn ld_hld_a_and_ld_a_hld() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();
        // 32 ; 3A
        bus.load_rom(&rom(&[0x32, 0x3a]));
        cpu.regs.set_hl(0x8001);
        cpu.regs.a = 0x6c;
        cpu.step(&mut bus); // LD (HL-),A
        assert_eq!(bus.read_byte(0x8001), 0x6c);
        assert_eq!(cpu.regs.get_hl(), 0x8000);

        bus.write_byte(0x8000, 0xb7);
        cpu.step(&mut bus); // LD A,(HL-)
        assert_eq!(cpu.regs.a, 0xb7);
        assert_eq!(cpu.regs.get_hl(), 0x7fff);
    }

    // -----------------------
    // High-RAM I/O (LDH and (FF00+C))
    // -----------------------
    #[test]
    fn ldh_a8_a_and_a_a8() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();
        // E0 10 ; F0 10
        bus.load_rom(&rom(&[0xe0, 0x10, 0xf0, 0x10]));
        cpu.regs.a = 0x5a;
        cpu.step(&mut bus); // LDH (0xFF10),A
        assert_eq!(bus.read_byte(0xff10), 0x5a);

        bus.write_byte(0xff10, 0xab);
        cpu.regs.a = 0x00;
        cpu.step(&mut bus); // LDH A,(0xFF10)
        assert_eq!(cpu.regs.a, 0xab);
    }

    #[test]
    fn ld_c_indexed_store_and_load() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();
        // E2 ; F2
        bus.load_rom(&rom(&[0xe2, 0xf2]));
        cpu.regs.c = 0x34;
        cpu.regs.a = 0x66;

        cpu.step(&mut bus); // (FF00+C) <- A
        assert_eq!(bus.read_byte(0xff34), 0x66);

        bus.write_byte(0xff34, 0x99);
        cpu.step(&mut bus); // A <- (FF00+C)
        assert_eq!(cpu.regs.a, 0x99);
    }

    // -----------------------
    // LD (a16),SP and LD SP,HL
    // -----------------------
    #[test]
    fn ld_a16_sp_writes_sp_little_endian() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();
        // 31 FE FF ; 08 00 80
        bus.load_rom(&rom(&[0x31, 0xfe, 0xff, 0x08, 0x00, 0x80]));
        cpu.step(&mut bus); // LD SP,0xFFFE
        assert_eq!(cpu.sp, 0xfffe);
        cpu.step(&mut bus); // LD (0x8000),SP
        assert_eq!(bus.read_byte(0x8000), 0xfe);
        assert_eq!(bus.read_byte(0x8001), 0xff);
    }

    #[test]
    fn ld_sp_hl_copies_hl_into_sp() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();
        // 21 34 12 ; F9
        bus.load_rom(&rom(&[0x21, 0x34, 0x12, 0xf9]));
        cpu.step(&mut bus); // HL=0x1234
        assert_eq!(cpu.regs.get_hl(), 0x1234);
        cpu.step(&mut bus); // SP=HL
        assert_eq!(cpu.sp, 0x1234);
    }

    // -----------------------
    // PUSH / POP
    // -----------------------
    #[test]
    fn push_pop_bc_and_af_and_de_hl() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();
        // C5 C1 F5 F1 D5 D1 E5 E1
        bus.load_rom(&rom(&[0xc5, 0xc1, 0xf5, 0xf1, 0xd5, 0xd1, 0xe5, 0xe1]));
        cpu.sp = 0xfffe;

        // BC
        cpu.regs.set_bc(0x1234);
        cpu.step(&mut bus); // PUSH BC
        assert_eq!(cpu.sp, 0xfffc);
        assert_eq!(bus.read_byte(0xfffd), 0x12);
        assert_eq!(bus.read_byte(0xfffc), 0x34);
        cpu.regs.set_bc(0x0000);
        cpu.step(&mut bus); // POP BC
        assert_eq!(cpu.sp, 0xfffe);
        assert_eq!(cpu.regs.get_bc(), 0x1234);

        // AF
        cpu.regs.a = 0x9a;
        cpu.regs.f.zero = true;
        cpu.regs.f.half_carry = true;
        cpu.regs.f.carry = true;
        cpu.step(&mut bus); // PUSH AF
        let f_byte = bus.read_byte(0xfffc);
        let a_byte = bus.read_byte(0xfffd);
        assert_eq!(a_byte, 0x9a);
        assert_eq!(f_byte & 0x0f, 0x00); // low nibble of F is 0
        cpu.regs.a = 0x00;
        cpu.regs.f.zero = false;
        cpu.regs.f.half_carry = false;
        cpu.regs.f.carry = false;
        cpu.step(&mut bus); // POP AF
        assert_eq!(cpu.sp, 0xfffe);
        assert_eq!(cpu.regs.a, 0x9a);
        assert!(cpu.regs.f.zero);
        assert!(cpu.regs.f.half_carry);
        assert!(cpu.regs.f.carry);

        // DE / HL
        cpu.regs.set_de(0xbeef);
        cpu.step(&mut bus); // PUSH DE
        assert_eq!(cpu.sp, 0xfffc);
        cpu.regs.set_de(0x0000);
        cpu.step(&mut bus); // POP DE
        assert_eq!(cpu.sp, 0xfffe);
        assert_eq!(cpu.regs.get_de(), 0xbeef);

        cpu.regs.set_hl(0x0f01);
        cpu.step(&mut bus); // PUSH HL
        assert_eq!(cpu.sp, 0xfffc);
        cpu.regs.set_hl(0x0000);
        cpu.step(&mut bus); // POP HL
        assert_eq!(cpu.sp, 0xfffe);
        assert_eq!(cpu.regs.get_hl(), 0x0f01);
    }

    // -----------------------
    // JR (relative)
    // -----------------------
    #[test]
    fn jr_unconditional_forward_and_backward() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();
        // Place JR +5 at 0x0000 and JR -5 at 0x0003 (so we can jump around)
        let mut bytes = [0u8; 6];
        bytes[0] = 0x18;
        bytes[1] = 0x05; // JR +5 (from 0x0002 -> 0x0007)
        bytes[3] = 0x18;
        bytes[4] = 0xfb; // JR -5 (from 0x0005 -> 0x0000)
        bus.load_rom(&bytes);

        cpu.pc = 0x0000;
        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x0007);

        cpu.pc = 0x0003;
        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x0000);
    }

    #[test]
    fn jr_nz_taken_and_not_taken() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();
        // JR NZ,+5
        bus.load_rom(&rom(&[0x20, 0x05]));
        cpu.regs.f.zero = false; // taken
        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x0007);

        cpu.pc = 0x0000;
        cpu.regs.f.zero = true; // not taken
        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x0002);
    }

    // -----------------------
    // Conditional JP/CALL/RET
    // -----------------------
    #[test]
    fn jp_nz_taken_and_not_taken() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();
        // C2 34 12 (JP NZ,0x1234)
        bus.load_rom(&rom(&[0xc2, 0x34, 0x12]));
        cpu.regs.f.zero = false;
        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x1234);

        cpu.pc = 0x0000;
        cpu.regs.f.zero = true;
        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x0003); // fallthrough
    }

    #[test]
    fn call_c_taken_and_not_taken_with_stack() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();
        // DC 78 56 (CALL C,0x5678)
        bus.load_rom(&rom(&[0xdc, 0x78, 0x56]));
        cpu.sp = 0xfffe;

        cpu.regs.f.carry = true; // taken
        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x5678);
        assert_eq!(cpu.sp, 0xfffc);
        let lo = bus.read_byte(0xfffc) as u16;
        let hi = bus.read_byte(0xfffd) as u16;
        assert_eq!((hi << 8) | lo, 0x0003);

        cpu.pc = 0x0000;
        cpu.sp = 0xfffe;
        cpu.regs.f.carry = false; // not taken
        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x0003);
        assert_eq!(cpu.sp, 0xfffe);
    }

    #[test]
    fn ret_nc_taken_and_not_taken() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();
        // D0 (RET NC)
        bus.load_rom(&rom(&[0xd0]));
        // Prepare return addr 0x3456 on stack
        cpu.sp = 0xfffc;
        bus.write_byte(0xfffc, 0x56);
        bus.write_byte(0xfffd, 0x34);

        cpu.regs.f.carry = false; // taken
        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x3456);
        assert_eq!(cpu.sp, 0xfffe);

        // Not taken
        cpu.pc = 0x0000;
        cpu.sp = 0xfffc;
        cpu.regs.f.carry = true;
        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x0001);
        assert_eq!(cpu.sp, 0xfffc);
    }

    // -----------------------
    // ALU
    // -----------------------
    #[test]
    fn add_and_adc_with_flags() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();
        // C6 0F ; CE 01
        bus.load_rom(&rom(&[0xc6, 0x0f, 0xce, 0x01]));
        cpu.regs.a = 0x01;
        cpu.regs.f.carry = false;

        cpu.step(&mut bus); // A=0x10, H=1
        assert_eq!(cpu.regs.a, 0x10);
        assert_eq!(cpu.regs.f.zero, false);
        assert_eq!(cpu.regs.f.subtract, false);
        assert_eq!(cpu.regs.f.half_carry, true);
        assert_eq!(cpu.regs.f.carry, false);

        cpu.regs.f.carry = true; // ADC with carry-in
        cpu.step(&mut bus); // A=0x12
        assert_eq!(cpu.regs.a, 0x12);
        assert_eq!(cpu.regs.f.half_carry, false);
        assert_eq!(cpu.regs.f.carry, false);
    }

    #[test]
    fn sub_and_sbc_with_borrow_and_cp() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();
        // D6 01 ; DE 01 ; FE 10   (SUB 1 ; SBC 1 ; CP 0x10)
        bus.load_rom(&rom(&[0xd6, 0x01, 0xde, 0x01, 0xfe, 0x10]));
        cpu.regs.a = 0x10;
        cpu.regs.f.carry = false;

        cpu.step(&mut bus); // A=0x0F -> H=1, C=0, N=1
        assert_eq!(cpu.regs.a, 0x0f);
        assert!(cpu.regs.f.half_carry);
        assert!(!cpu.regs.f.carry);
        assert!(cpu.regs.f.subtract);

        cpu.regs.f.carry = true; // borrow-in
        cpu.step(&mut bus); // A=0x0D
        assert_eq!(cpu.regs.a, 0x0d);
        assert!(cpu.regs.f.subtract);
        assert!(!cpu.regs.f.carry);
        assert!(!cpu.regs.f.zero);

        // CP 0x10 vs A=0x0D: full borrow (C=1), low nibble 0xD !< 0x0 => H=0
        let a_before = cpu.regs.a;
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.a, a_before);
        assert!(cpu.regs.f.subtract);
        assert!(cpu.regs.f.carry);
        assert!(!cpu.regs.f.half_carry);
        assert!(!cpu.regs.f.zero);
    }

    #[test]
    fn logical_ops_and_xor_or() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();
        // E6 F0 ; EE 0F ; F6 01
        bus.load_rom(&rom(&[0xe6, 0xf0, 0xee, 0x0f, 0xf6, 0x01]));
        cpu.regs.a = 0x3c;

        cpu.step(&mut bus); // AND 0xF0 => 0x30 (H=1, C=0, N=0)
        assert_eq!(cpu.regs.a, 0x30);
        assert!(cpu.regs.f.half_carry);
        assert!(!cpu.regs.f.carry);
        assert!(!cpu.regs.f.subtract);

        cpu.step(&mut bus); // XOR 0x0F => 0x3F (H=0, C=0, N=0)
        assert_eq!(cpu.regs.a, 0x3f);
        assert!(!cpu.regs.f.half_carry);
        assert!(!cpu.regs.f.carry);
        assert!(!cpu.regs.f.subtract);

        cpu.step(&mut bus); // OR 0x01 => 0x3F
        assert_eq!(cpu.regs.a, 0x3f);
        assert_eq!(cpu.regs.f.zero, false);
        assert!(!cpu.regs.f.half_carry);
        assert!(!cpu.regs.f.carry);
    }

    #[test]
    fn alu_with_hl_memory_source() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();
        // 86 9E A6 AE B6 BE
        bus.load_rom(&rom(&[0x86, 0x9e, 0xa6, 0xae, 0xb6, 0xbe]));
        cpu.regs.set_hl(0x8000);
        bus.write_byte(0x8000, 0x10);

        cpu.regs.a = 0x0f;
        cpu.regs.f.carry = false;
        cpu.step(&mut bus); // ADD A,(HL) => 0x1F
        assert_eq!(cpu.regs.a, 0x1f);

        cpu.regs.f.carry = true;
        cpu.step(&mut bus); // SBC (carry-in) => 0x1F - 0x10 - 1 = 0x0E
        assert_eq!(cpu.regs.a, 0x0e);

        cpu.step(&mut bus); // AND (0x10) => 0x00
        assert_eq!(cpu.regs.a, 0x00);
        assert!(cpu.regs.f.zero);

        cpu.step(&mut bus); // XOR (0x10) => 0x10
        assert_eq!(cpu.regs.a, 0x10);

        cpu.step(&mut bus); // OR (0x10) => 0x10
        assert_eq!(cpu.regs.a, 0x10);

        let a_before = cpu.regs.a;
        cpu.step(&mut bus); // CP (HL) => compare with 0x10 -> Z=1
        assert_eq!(cpu.regs.a, a_before);
        assert!(cpu.regs.f.zero);
    }

    /// NOP = 4 cycles. DIV increments every 256 cycles => every 64 NOPs.
    #[test]
    fn div_increments_every_256_cycles() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();

        // 64 NOPs
        let rom = vec![0x00; 64];
        bus.load_rom(&rom);

        let prev_div = bus.read_byte(0xff04);
        for _ in 0..64 {
            cpu.step(&mut bus);
        }
        let div = bus.read_byte(0xff04);

        // DIV should have incremented by ~1 (wrap considered)
        assert_eq!(div, prev_div.wrapping_add(1));
    }

    /// TIMA increments at selected TAC frequency when enabled.
    /// For TAC=0b101 (enable, 262144Hz), the period is 16 cycles => 4 NOPs per increment.
    #[test]
    fn tima_increments_with_tac_262khz() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();

        // Program: write TAC, then a bunch of NOPs
        // We'll just plant NOPs and program TAC via bus writes before stepping.
        let rom = vec![0x00; 16];
        bus.load_rom(&rom);

        // Enable timer, select 262144Hz: TAC = 0b101
        bus.write_byte(0xff07, 0b0000_0101);

        let tima0 = bus.read_byte(0xff05);
        // 4 NOPs -> 16 cycles -> +1 TIMA
        for _ in 0..4 {
            cpu.step(&mut bus);
        }
        let tima1 = bus.read_byte(0xff05);
        assert_eq!(tima1, tima0.wrapping_add(1));

        // Another 4 NOPs -> +1 more
        for _ in 0..4 {
            cpu.step(&mut bus);
        }
        let tima2 = bus.read_byte(0xff05);
        assert_eq!(tima2, tima1.wrapping_add(1));
    }

    /// On overflow (FF->00), TIMA reloads from TMA and IF[TIMER] is requested.
    #[test]
    fn tima_overflow_reloads_tma_and_requests_interrupt() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();

        // ROM of NOPs
        bus.load_rom(&[0x00; 8]);

        // Set TMA=0x42; TIMA=0xFF; enable at 262kHz (16 cycles/step)
        bus.write_byte(0xff06, 0x42); // TMA
        // Set TIMA via direct write (allowed)
        bus.write_byte(0xff05, 0xff); // TIMA
        bus.write_byte(0xff07, 0b0000_0101); // TAC enable + 262kHz

        // Ensure IF is clear beforehand
        bus.write_byte(0xff0f, 0x00);

        // 4 NOPs => +1 TIMA => overflow; reload from TMA; IF[2] set
        for _ in 0..4 {
            cpu.step(&mut bus);
        }

        let tima = bus.read_byte(0xff05);
        let tma = bus.read_byte(0xff06);
        let iflag = bus.read_byte(0xff0f);

        assert_eq!(tima, tma); // reloaded
        assert_eq!(tma, 0x42);
        assert_ne!(iflag & 0b0000_0100, 0); // IF bit-2 (Timer) set
    }

    /// Quick smoke: changing TAC to a slower clock still causes increments, just less often.
    #[test]
    fn tima_increments_with_lower_rate() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();
        bus.load_rom(&[0x00; 256]);

        // Enable timer, 4096Hz (TAC=0b100). Period: 1024 cycles => 256 NOPs per increment.
        bus.write_byte(0xff07, 0b0000_0100);

        let t0 = bus.read_byte(0xff05);
        for _ in 0..256 {
            cpu.step(&mut bus);
        } // 256*4 = 1024 cycles
        let t1 = bus.read_byte(0xff05);
        assert_eq!(t1, t0.wrapping_add(1));
    }

    /// Writing DIV resets the divider (DIV becomes 0).
    #[test]
    fn writing_div_resets_divider() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();
        bus.load_rom(&[0x00; 4]);

        // Run a few cycles so DIV is non-zero
        for _ in 0..4 {
            cpu.step(&mut bus);
        }
        let div_before = bus.read_byte(0xff04);
        assert_ne!(div_before, 0);

        // Write any value to DIV to reset
        bus.write_byte(0xff04, 0xab);
        let div_after = bus.read_byte(0xff04);
        assert_eq!(div_after, 0);
    }
}
