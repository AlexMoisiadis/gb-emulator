// src/cpu.rs
use crate::bus::MemoryBus;
use crate::instruction::{DecodeError, Instruction};
use crate::registers::Registers;
use crate::types::ArithmeticTarget;

#[derive(Debug)]
pub struct CPU {
    pub registers: Registers,
    pub pc: u16,
    pub bus: MemoryBus,
}

impl CPU {
    pub fn new(bus: MemoryBus) -> Self {
        Self {
            registers: Registers::default(),
            pc: 0,
            bus,
        }
    }

    pub fn step(&mut self) {
        let opcode = self.bus.read_byte(self.pc);
        match Instruction::from_byte(opcode) {
            Ok(instruction) => {
                let next_pc = self.execute(instruction);
                self.pc = next_pc;
            }
            Err(DecodeError::UnknownOpcode(_)) => {
                panic!("Unknown instruction found for: 0x{:02X}", opcode);
            }
        }
    }

    fn execute(&mut self, instruction: Instruction) -> u16 {
        match instruction {
            Instruction::ADD(target) => match target {
                ArithmeticTarget::C => {
                    let value = self.registers.c;
                    let new_value = self.add(value);
                    self.registers.a = new_value;
                    self.pc.wrapping_add(1)
                }
                // Add other ArithmeticTarget variants as you implement them
                _ => self.pc,
            },
            Instruction::JP(_test) => {
                // Implement Jump tests and PC changes here
                self.pc
            }
        }
    }

    fn add(&mut self, value: u8) -> u8 {
        let (new_value, did_overflow) = self.registers.a.overflowing_add(value);
        self.registers.f.zero = new_value == 0;
        self.registers.f.subtract = false;
        self.registers.f.carry = did_overflow;
        self.registers.f.half_carry = (self.registers.a & 0x0F) + (value & 0x0F) > 0x0F;
        new_value
    }
}