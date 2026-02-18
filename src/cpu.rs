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
            // ---- LD variants we’ve mapped so far ----
            Instruction::LD(LoadType::Byte(tgt, src)) => {
                match (tgt, src) {
                    // 0x02: LD (BC),A  [handled by decoder mapping]
                    (LoadByteTarget::MemReg16(Reg16::BC), LoadByteSource::A) => {
                        let addr = self.regs.get_bc();
                        bus.write_byte(addr, self.regs.a);
                        8
                    }
                    // 0xEA: LD (a16),A
                    (LoadByteTarget::MemImm16, LoadByteSource::A) => {
                        let addr = self.fetch16(bus);
                        bus.write_byte(addr, self.regs.a);
                        16
                    }
                    // 0xFA: LD A,(a16)
                    (LoadByteTarget::A, LoadByteSource::MemImm16) => {
                        let addr = self.fetch16(bus);
                        self.regs.a = bus.read_byte(addr);
                        16
                    }

                    // src/cpu.rs  (inside exec match for Instruction::LD(LoadType::Byte(..)))
                    (LoadByteTarget::A, LoadByteSource::D8) => {
                        let imm = self.fetch8(bus); // read the immediate byte after opcode
                        self.regs.a = imm;
                        8 // placeholder cycles; we'll refine timing later
                    }
                    (LoadByteTarget::B, LoadByteSource::D8) => {
                        let imm = self.fetch8(bus); // read immediate after opcode
                        self.regs.b = imm;
                        8 // placeholder cycles for now
                    }

                    // LD C,d8 (0x0E)
                    (LoadByteTarget::C, LoadByteSource::D8) => {
                        let imm = self.fetch8(bus);
                        self.regs.c = imm;
                        8
                    }

                    // LD D,d8 (0x16)
                    (LoadByteTarget::D, LoadByteSource::D8) => {
                        let imm = self.fetch8(bus);
                        self.regs.d = imm;
                        8
                    }

                    // LD E,d8 (0x1E)
                    (LoadByteTarget::E, LoadByteSource::D8) => {
                        let imm = self.fetch8(bus);
                        self.regs.e = imm;
                        8
                    }

                    // LD H,d8 (0x26)
                    (LoadByteTarget::H, LoadByteSource::D8) => {
                        let imm = self.fetch8(bus);
                        self.regs.h = imm;
                        8
                    }

                    // LD L,d8 (0x2E)
                    (LoadByteTarget::L, LoadByteSource::D8) => {
                        let imm = self.fetch8(bus);
                        self.regs.l = imm;
                        8
                    }
                    // LD A,(BC)  [0x0A]
                    (LoadByteTarget::A, LoadByteSource::MemReg16(Reg16::BC)) => {
                        let addr = self.regs.get_bc();
                        self.regs.a = bus.read_byte(addr);
                        8
                    }

                    // LD (DE),A  [0x12]
                    (LoadByteTarget::MemReg16(Reg16::DE), LoadByteSource::A) => {
                        let addr = self.regs.get_de();
                        bus.write_byte(addr, self.regs.a);
                        8
                    }

                    // LD A,(DE)  [0x1A]
                    (LoadByteTarget::A, LoadByteSource::MemReg16(Reg16::DE)) => {
                        let addr = self.regs.get_de();
                        self.regs.a = bus.read_byte(addr);
                        8
                    }
                    _ => self.trap_unknown(DecodeError::UnknownOpcode(0x00, false)),
                }
            }

            // ---- 16-bit INC ----
            Instruction::INC(IncDecTarget::BC) => {
                let bc = self.regs.get_bc().wrapping_add(1);
                self.regs.set_bc(bc);
                8
            }

            // ---- Control flow ----
            // 0xC3 JP a16
            Instruction::JP(JumpTest::Always) => {
                let addr = self.fetch16(bus);
                self.pc = addr;
                16
            }
            // 0xCD CALL a16
            Instruction::CALL(JumpTest::Always) => {
                let addr = self.fetch16(bus);
                self.push16(bus, self.pc);
                self.pc = addr;
                24
            }
            // 0xC9 RET
            Instruction::RET(JumpTest::Always) => {
                let addr = self.pop16(bus);
                self.pc = addr;
                16
            }

            Instruction::NOP => { 4 }

            _ => self.trap_unknown(DecodeError::UnknownOpcode(0x00, false)),
        }
    }

    fn trap_unknown(&self, e: DecodeError) -> u32 {
        panic!("Unknown or unhandled instruction: {:?}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_ld_a_d8() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();

        // Place: 3E 99  (LD A,0x99)
        bus.write_byte(0x0000, 0x3e);
        bus.write_byte(0x0001, 0x99);

        cpu.step(&mut bus);
        assert_eq!(cpu.regs.a, 0x99);
    }

    #[test]
    fn exec_ld_r_d8_group() {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();

        // Program at 0x0000:
        // 06 11 | 0E 22 | 16 33 | 1E 44 | 26 55 | 2E 66 | 3E 77
        //  LD B,11   LD C,22   LD D,33   LD E,44   LD H,55   LD L,66   LD A,77
        let prog = [
            0x06, 0x11, 0x0e, 0x22, 0x16, 0x33, 0x1e, 0x44, 0x26, 0x55, 0x2e, 0x66, 0x3e, 0x77,
        ];
        for (i, b) in prog.iter().enumerate() {
            bus.write_byte(i as u16, *b);
        }

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

        // Memory setup: put distinct bytes at 0x8001 and 0x8002
        bus.write_byte(0x8001, 0xaa);
        bus.write_byte(0x8002, 0xbb);

        // Program:
        // 00: 01 01 80   LD BC,0x8001   (not implemented yet, so we’ll set BC directly)
        // 03: 0A         LD A,(BC)      => A=0xAA
        // 04: 12         LD (DE),A      (after setting DE)
        // 05: 1A         LD A,(DE)      => A=0xBB
        //
        // For now, poke registers instead of LD rr,d16.
        cpu.pc = 0x0000;
        cpu.regs.set_bc(0x8001);
        cpu.regs.set_de(0x8002);

        // Encode the three bytes we do execute: 0A 12 1A
        bus.write_byte(0x0000, 0x0a); // LD A,(BC)
        bus.write_byte(0x0001, 0x12); // LD (DE),A  -> writes 0xAA into 0x8002
        bus.write_byte(0x0002, 0x1a); // LD A,(DE)  -> reads  0xAA from 0x8002 (we overwrote BB)

        cpu.step(&mut bus); // 0x0A
        assert_eq!(cpu.regs.a, 0xaa);

        cpu.step(&mut bus); // 0x12
        assert_eq!(bus.read_byte(0x8002), 0xaa);

        cpu.step(&mut bus); // 0x1A
        assert_eq!(cpu.regs.a, 0xaa);
    }
}
