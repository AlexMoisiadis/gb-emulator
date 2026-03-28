// src/cpu.rs

// --- Tiny macros to reduce repetition for register-only cases ---

/// INC r: apply alu_inc8 to a register and return 4 cycles.
macro_rules! inc8_reg {
    ($self:ident, $reg:ident) => {
        {
        $self.regs.$reg = $self.alu_inc8($self.regs.$reg);
        4
        }
    };
}

/// DEC r: apply alu_dec8 to a register and return 4 cycles.
macro_rules! dec8_reg {
    ($self:ident, $reg:ident) => {
        {
        $self.regs.$reg = $self.alu_dec8($self.regs.$reg);
        4
        }
    };
}

/// LD r,#imm8: fetch next byte into a register and return 8 cycles.
macro_rules! ld_d8 {
    ($self:ident, $bus:ident, $reg:ident) => {
        {
        $self.regs.$reg = $self.fetch8($bus);
        8
        }
    };
}
use crate::bus::MemoryBus;
use crate::instruction::{ DecodeError, Instruction };
use crate::registers::Registers;
use crate::trace::{ self, Category as TraceCategory };
use crate::types::{
    IncDecTarget,
    JumpTest,
    LoadByteSource,
    LoadByteTarget,
    LoadType,
    PrefixTarget,
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
    halt_bug_armed: bool,
    ei_delay: u8, // NEW: counts down instructions after EI
    step_index: u64,
    trace_irq_last_ime: bool,
    trace_irq_last_ie: u8,
    trace_irq_last_if: u8,
    trace_irq_last_pending: u8,
    trace: bool,
    last_pc: u16,
    last_op: u8,
    timer_consumed: u32,
    gpu_consumed: u32,

    // --- NEW: low-noise debug snapshots (only used when debug_timing is on) ---
    #[cfg(feature = "debug_timing")]
    last_ime: bool,
    #[cfg(feature = "debug_timing")]
    last_ie: u8,
    #[cfg(feature = "debug_timing")]
    last_if: u8,
}

impl CPU {
    pub fn new() -> Self {
        Self {
            regs: Registers::default(),
            pc: 0x0000,
            sp: 0xfffe,
            ime: false,
            halted: false,
            halt_bug_armed: false,
            ei_delay: 0, // NEW
            step_index: 0,
            trace_irq_last_ime: false,
            trace_irq_last_ie: 0,
            trace_irq_last_if: 0,
            trace_irq_last_pending: 0,
            trace: false,
            last_pc: 0,
            last_op: 0,
            timer_consumed: 0,
            gpu_consumed: 0,

            // --- NEW ---
            #[cfg(feature = "debug_timing")]
            last_ime: false,
            #[cfg(feature = "debug_timing")]
            last_ie: 0,
            #[cfg(feature = "debug_timing")]
            last_if: 0,
        }
    }

    #[inline]
    pub fn set_trace(&mut self, enabled: bool) {
        self.trace = enabled;
    }

    // NEW:
    #[inline]
    fn trace_insn(&self, pc: u16, op: u8, cb_extra: Option<u8>) {
        if !self.trace {
            return;
        }
        let af = self.regs.get_af();
        let bc = self.regs.get_bc();
        let de = self.regs.get_de();
        let hl = self.regs.get_hl();

        match cb_extra {
            Some(cb) => {
                println!(
                    "PC={:04X} OP={:02X} CB={:02X} | AF={:04X} BC={:04X} DE={:04X} HL={:04X} SP={:04X} IME={} HALT={}",
                    pc,
                    op,
                    cb,
                    af,
                    bc,
                    de,
                    hl,
                    self.sp,
                    self.ime as u8,
                    self.halted as u8
                );
            }
            None => {
                println!(
                    "PC={:04X} OP={:02X}      | AF={:04X} BC={:04X} DE={:04X} HL={:04X} SP={:04X} IME={} HALT={}",
                    pc,
                    op,
                    af,
                    bc,
                    de,
                    hl,
                    self.sp,
                    self.ime as u8,
                    self.halted as u8
                );
            }
        }
    }

    #[inline]
    fn log_regs(&self, context: &str) {
        let af = self.regs.get_af();
        let bc = self.regs.get_bc();
        let de = self.regs.get_de();
        let hl = self.regs.get_hl();
        println!("{}  AF={:04X}  BC={:04X}  DE={:04X}  HL={:04X}", context, af, bc, de, hl);
    }

    #[inline]
    fn write_byte_with_ff50_log(&self, bus: &mut MemoryBus, addr: u16, val: u8) {
        if addr == 0xff50 {
            // Reuse your existing logger
            self.log_regs("FF50 write:");
        }
        bus.write_byte(addr, val);
    }

    #[inline]
    fn alu_cycles(src: &LoadByteSource) -> u32 {
        // Known variants defaulted intentionally; update this when adding new variants.
        match src {
            LoadByteSource::D8 | LoadByteSource::MemReg16(Reg16::HL) => 8,
            | LoadByteSource::A
            | LoadByteSource::B
            | LoadByteSource::C
            | LoadByteSource::D
            | LoadByteSource::E
            | LoadByteSource::H
            | LoadByteSource::L => 4,
            // These should never come through ALU in your current decoder; if they do, you’ll want to decide cycles.
            other => {
                debug_assert!(
                    matches!(
                        other,
                        LoadByteSource::D8 |
                            LoadByteSource::MemReg16(Reg16::HL) |
                            LoadByteSource::A |
                            LoadByteSource::B |
                            LoadByteSource::C |
                            LoadByteSource::D |
                            LoadByteSource::E |
                            LoadByteSource::H |
                            LoadByteSource::L
                    ),
                    "Unexpected ALU source {:?}; confirm cycles!",
                    other
                );
                4
            }
        }
    }

    /// Run a CB operation that reads a target, produces a new value, writes it back,
    /// and returns the correct cycles (8 for r, 16 for (HL)).
    #[inline]
    fn cb_rw<F>(&mut self, bus: &mut MemoryBus, t: PrefixTarget, f: F) -> u32
        where F: FnOnce(&mut Self, u8) -> u8
    {
        let v = self.cb_read(bus, t);
        let r = f(self, v);
        let is_hl = matches!(t, PrefixTarget::HL);
        self.cb_write(bus, t, r);
        if is_hl {
            16
        } else {
            8
        }
    }

    /// Run a CB operation that only reads (e.g., BIT), sets flags, and returns correct cycles
    /// (8 for r, 12 for (HL)).
    #[inline]
    fn cb_read_only<F>(&mut self, bus: &mut MemoryBus, t: PrefixTarget, f: F) -> u32
        where F: FnOnce(&mut Self, u8)
    {
        let v = self.cb_read(bus, t);
        f(self, v);
        if matches!(t, PrefixTarget::HL) {
            12
        } else {
            8
        }
    }

    #[inline]
    fn cb_read(&mut self, bus: &mut MemoryBus, t: PrefixTarget) -> u8 {
        match t {
            PrefixTarget::B => self.regs.b,
            PrefixTarget::C => self.regs.c,
            PrefixTarget::D => self.regs.d,
            PrefixTarget::E => self.regs.e,
            PrefixTarget::H => self.regs.h,
            PrefixTarget::L => self.regs.l,
            PrefixTarget::A => self.regs.a,
            PrefixTarget::HL => {
                let addr = self.regs.get_hl();
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                let v = bus.read_byte(addr);
                bus.service_gpu(4); self.gpu_consumed += 4;
                v
            }
        }
    }

    #[inline]
    fn cb_write(&mut self, bus: &mut MemoryBus, t: PrefixTarget, val: u8) {
        match t {
            PrefixTarget::B => {
                self.regs.b = val;
            }
            PrefixTarget::C => {
                self.regs.c = val;
            }
            PrefixTarget::D => {
                self.regs.d = val;
            }
            PrefixTarget::E => {
                self.regs.e = val;
            }
            PrefixTarget::H => {
                self.regs.h = val;
            }
            PrefixTarget::L => {
                self.regs.l = val;
            }
            PrefixTarget::A => {
                self.regs.a = val;
            }
            PrefixTarget::HL => {
                let addr = self.regs.get_hl();
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                bus.write_byte(addr, val);
                bus.service_gpu(4); self.gpu_consumed += 4;
            }
        }
    }

    // ========== CB bit helpers (BIT/RES/SET) ==========
    #[inline]
    fn cb_bit_test_flags(&mut self, bit: u8, val: u8) {
        // Z = !(val has bit), N=0, H=1, C preserved
        self.regs.f.zero = (val & (1 << bit)) == 0;
        self.regs.f.subtract = false;
        self.regs.f.half_carry = true;
        // carry unchanged
    }

    #[inline]
    fn cb_bit_res(val: u8, bit: u8) -> u8 {
        val & !(1 << bit)
    }

    #[inline]
    fn cb_bit_set(val: u8, bit: u8) -> u8 {
        val | (1 << bit)
    }

    // ========== CB rotate/shift ops ==========
    // Flags (CB family): Z set by result, N=0, H=0, C as noted per op.

    #[inline]
    fn op_rlc(&mut self, v: u8) -> u8 {
        let c = (v >> 7) & 1;
        let res = v.rotate_left(1);
        self.regs.f.zero = res == 0;
        self.regs.f.subtract = false;
        self.regs.f.half_carry = false;
        self.regs.f.carry = c != 0;
        res
    }

    #[inline]
    fn op_rrc(&mut self, v: u8) -> u8 {
        let c = v & 1;
        let res = v.rotate_right(1);
        self.regs.f.zero = res == 0;
        self.regs.f.subtract = false;
        self.regs.f.half_carry = false;
        self.regs.f.carry = c != 0;
        res
    }

    #[inline]
    fn op_rl(&mut self, v: u8) -> u8 {
        let carry_in = if self.regs.f.carry { 1 } else { 0 };
        let new_carry = (v >> 7) & 1;
        let res = (v << 1) | carry_in;
        self.regs.f.zero = res == 0;
        self.regs.f.subtract = false;
        self.regs.f.half_carry = false;
        self.regs.f.carry = new_carry != 0;
        res
    }

    #[inline]
    fn op_rr(&mut self, v: u8) -> u8 {
        let carry_in = if self.regs.f.carry { 1 } else { 0 };
        let new_carry = v & 1;
        let res = (v >> 1) | ((carry_in as u8) << 7);
        self.regs.f.zero = res == 0;
        self.regs.f.subtract = false;
        self.regs.f.half_carry = false;
        self.regs.f.carry = new_carry != 0;
        res
    }

    #[inline]
    fn op_sla(&mut self, v: u8) -> u8 {
        let new_carry = (v >> 7) & 1;
        let res = v << 1;
        self.regs.f.zero = res == 0;
        self.regs.f.subtract = false;
        self.regs.f.half_carry = false;
        self.regs.f.carry = new_carry != 0;
        res
    }

    #[inline]
    fn op_sra(&mut self, v: u8) -> u8 {
        let new_carry = v & 1;
        let msb = v & 0x80;
        let res = (v >> 1) | msb; // arithmetic (preserve bit7)
        self.regs.f.zero = res == 0;
        self.regs.f.subtract = false;
        self.regs.f.half_carry = false;
        self.regs.f.carry = new_carry != 0;
        res
    }

    #[inline]
    fn op_srl(&mut self, v: u8) -> u8 {
        let new_carry = v & 1;
        let res = v >> 1; // logical (bit7 becomes 0)
        self.regs.f.zero = res == 0;
        self.regs.f.subtract = false;
        self.regs.f.half_carry = false;
        self.regs.f.carry = new_carry != 0;
        res
    }

    #[inline]
    fn op_swap(&mut self, v: u8) -> u8 {
        let res = (v << 4) | (v >> 4);
        self.regs.f.zero = res == 0;
        self.regs.f.subtract = false;
        self.regs.f.half_carry = false;
        self.regs.f.carry = false;
        res
    }

    #[inline]
    fn read_ie_if(bus: &mut MemoryBus) -> (u8, u8) {
        let ie = bus.read_byte(0xffff);
        let iflag = bus.read_byte(0xff0f);
        (ie, iflag)
    }

    fn any_pending_interrupt(&self, bus: &mut MemoryBus) -> bool {
        let (ie, iflag) = Self::read_ie_if(bus);
        (ie & iflag) != 0
    }

    #[inline]
    fn trace_cpu_halt_state(&self, bus: &mut MemoryBus) {
        if !trace::enabled(TraceCategory::CpuHalt) {
            return;
        }
        let (ie, iflag) = Self::read_ie_if(bus);
        let pending = ie & iflag;
        eprintln!(
            "event=cpu_halt step={} halted={} halt_bug_armed={} pending={:02X} ime={}",
            self.step_index,
            self.halted as u8,
            self.halt_bug_armed as u8,
            pending,
            self.ime as u8
        );
    }

    /// Execute a single instruction and return consumed cycles.

    pub fn step(&mut self, bus: &mut MemoryBus) -> u32 {
        self.step_index = self.step_index.saturating_add(1);
        trace::set_step(self.step_index);
        self.timer_consumed = 0;
        self.gpu_consumed = 0;
        bus.instruction_tcycles = 0;

        // --- EI delayed-IME semantics ----------------------------------------
        // If EI was executed previously, IME must become 1 *after* the next
        // instruction completes (i.e., between instructions, not immediately).
        // We snapshot the condition at the start of this step and apply IME at
        // the end of this step if we're on that "next" instruction.
        let enable_ime_after_this_instruction = self.ei_delay == 1;
        if self.ei_delay > 0 {
            self.ei_delay = self.ei_delay.saturating_sub(1);
        }

        // 1) Execute one instruction and get the cycles it consumed
        let mut total_tcycles: u32 = self.execute_one(bus);

        // 2) Advance PPU by the same amount and surface its events into IF now.
        // Timer was already partially advanced per-M-cycle during execute_one();
        // only service the remaining T-cycles here to avoid double-counting.
        bus.service_gpu(total_tcycles.saturating_sub(self.gpu_consumed));
        bus.service_timer(total_tcycles.saturating_sub(self.timer_consumed));
        bus.service_apu(total_tcycles);
        bus.service_input();

        // 3) Apply the deferred IME enable (EI delay completes *after* this instr)

        if enable_ime_after_this_instruction {
            self.ime = true;
            #[cfg(feature = "debug_timing")]
            if !trace::structured_enabled() {
                eprintln!("[CPU] IME enabled (EI delay complete)");
            }
            if trace::enabled(TraceCategory::CpuCtrl) {
                eprintln!(
                    "event=cpu_ctrl step={} action=ei_delay_complete ime={}",
                    self.step_index,
                    self.ime as u8
                );
            }
        }

        #[cfg(feature = "debug_timing")]
        {
            // Read IE/IF once (these are the exact values that will gate ISR entry)
            let ie_now = bus.read_byte(0xffff);
            let if_now = bus.read_byte(0xff0f);
            let ime_now = self.ime;
            let pend = ie_now & if_now;

            // Only print when one of the values changes (prevents console flood)
            if
                !trace::structured_enabled() &&
                (ime_now != self.last_ime || ie_now != self.last_ie || if_now != self.last_if)
            {
                eprintln!(
                    "[CPU] IME={} IE={:02X} IF={:02X} pending={:02X}",
                    ime_now as u8,
                    ie_now,
                    if_now,
                    pend
                );
                self.last_ime = ime_now;
                self.last_ie = ie_now;
                self.last_if = if_now;
            }
        }

        // 4) Interrupt entry: if IME is set and any interrupt is pending, service it now.
        let ime_now = self.ime;
        let (ie_now, if_now) = Self::read_ie_if(bus);
        let pending_now = ie_now & if_now;
        let mut irq_action = "none";
        if ime_now && pending_now != 0 {
            if self.service_one_interrupt(bus) {
                irq_action = "service";
                // ISR entry consumes 20 t-cycles on DMG.
                total_tcycles = total_tcycles.saturating_add(20);
                bus.service_gpu(20);
                bus.service_timer(20);
                bus.service_apu(20);
                bus.service_input();
            }
        } else if self.halted && pending_now != 0 {
            // Wake from HALT even if IME is 0 (matches DMG behavior)
            self.halted = false;
            irq_action = "wake_halt";
            self.trace_cpu_halt_state(bus);
        }

        let cpu_irq_changed =
            ime_now != self.trace_irq_last_ime ||
            ie_now != self.trace_irq_last_ie ||
            if_now != self.trace_irq_last_if ||
            pending_now != self.trace_irq_last_pending;
        if trace::enabled(TraceCategory::CpuIrq) && (irq_action != "none" || cpu_irq_changed) {
            eprintln!(
                "event=cpu_irq step={} ime={} ie={:02X} if={:02X} pending={:02X} action={}",
                self.step_index,
                ime_now as u8,
                ie_now,
                if_now,
                pending_now,
                irq_action
            );
        }
        self.trace_irq_last_ime = ime_now;
        self.trace_irq_last_ie = ie_now;
        self.trace_irq_last_if = if_now;
        self.trace_irq_last_pending = pending_now;

        total_tcycles
    }

    /// Service a single interrupt by priority (VBlank -> LCD STAT -> Timer -> Serial -> Joypad).
    /// Clears IF bit, clears IME, pushes PC, and jumps to vector.
    fn service_one_interrupt(&mut self, bus: &mut MemoryBus) -> bool {
        const IE_ADDR: u16 = 0xffff;
        const IF_ADDR: u16 = 0xff0f;

        let ie = bus.read_byte(IE_ADDR);
        bus.instruction_tcycles += 4;
        let mut iflag = bus.read_byte(IF_ADDR);
        bus.instruction_tcycles += 4;
        let pending = ie & iflag;
        if pending == 0 {
            return false;
        }

        // Interrupt table (bit, vector)
        //  VBlank: bit0 -> 0x0040
        //  LCDSTAT: bit1 -> 0x0048
        //  Timer: bit2 -> 0x0050
        //  Serial: bit3 -> 0x0058
        //  Joypad: bit4 -> 0x0060
        const VBLANK: (u8, u16) = (0, 0x0040);
        const LCDSTAT: (u8, u16) = (1, 0x0048);
        const TIMER: (u8, u16) = (2, 0x0050);
        const SERIAL: (u8, u16) = (3, 0x0058);
        const JOYPAD: (u8, u16) = (4, 0x0060);

        // Choose highest priority set bit
        let (bit, vector) = if (pending & (1 << VBLANK.0)) != 0 {
            VBLANK
        } else if (pending & (1 << LCDSTAT.0)) != 0 {
            LCDSTAT
        } else if (pending & (1 << TIMER.0)) != 0 {
            TIMER
        } else if (pending & (1 << SERIAL.0)) != 0 {
            SERIAL
        } else {
            JOYPAD
        };

        // Clear the IF bit we are servicing
        iflag &= !(1 << bit);
        bus.write_byte(IF_ADDR, iflag);
        bus.instruction_tcycles += 4;

        // Disable IME
        self.ime = false;
        self.halted = false;
        self.halt_bug_armed = false;

        // Push PC to stack (little endian).
        // Each SP decrement is an IDU operation — triggers OAM corruption if in range.
        let hi = ((self.pc >> 8) & 0xff) as u8;
        let lo = (self.pc & 0xff) as u8;
        bus.idu_oam_corrupt(self.sp);
        self.sp = self.sp.wrapping_sub(1);
        bus.write_byte(self.sp, hi);
        bus.instruction_tcycles += 4;
        bus.idu_oam_corrupt(self.sp);
        self.sp = self.sp.wrapping_sub(1);
        bus.write_byte(self.sp, lo);
        bus.instruction_tcycles += 4;

        // Jump to vector
        self.pc = vector;

        #[cfg(feature = "debug_timing")]
        if !trace::structured_enabled() {
            eprintln!("[CPU] ISR vector={:#06X} (bit {}) IME=OFF", vector, bit);
        }

        true
    }

    #[inline]
    fn execute_one(&mut self, bus: &mut MemoryBus) -> u32 {
        // HALT handling: burn 4 T-cycles. Wake detection is handled in step()
        // after peripheral advance, matching hardware M-cycle-boundary timing.
        if self.halted {
            return 4;
        }

        // Fetch opcode (fetch8 handles HALT bug automatically)
        let pc_before = self.pc;
        let op = self.fetch8(bus);
        self.last_pc = pc_before;
        self.last_op = op;

        if op == 0xcb {
            let cb = self.fetch8(bus);
            self.trace_insn(pc_before, op, Some(cb));

            match Instruction::from_byte(cb, true) {
                Ok(insn) => {
                    if self.trace {
                        println!("    => {}", insn.mnemonic());
                    }
                    self.exec(insn, bus)
                }
                Err(e) => self.trap_unknown(e),
            }
        } else {
            self.trace_insn(pc_before, op, None);

            match Instruction::from_byte(op, false) {
                Ok(insn) => {
                    if self.trace {
                        println!("    => {}", insn.mnemonic());
                    }
                    self.exec(insn, bus)
                }
                Err(e) => self.trap_unknown(e),
            }
        }
    }

    fn fetch8(&mut self, bus: &mut MemoryBus) -> u8 {
        let addr = self.pc;
        // PC increment is an IDU operation; if PC is in OAM range during mode 2,
        // the combined read+IDU produces Read-During-Inc/Dec corruption.
        let value = bus.oam_read_during_inc(addr);

        // HALT bug: suppress PC increment exactly once.
        if self.halt_bug_armed {
            self.halt_bug_armed = false;
        } else {
            self.pc = self.pc.wrapping_add(1);
        }

        // Each fetch8 is one M-cycle (4 T-cycles); advance the timer and GPU now so
        // mid-instruction TIMA reads and VBlank/STAT timing are accurate.
        bus.service_timer(4);
        bus.service_gpu(4);
        self.timer_consumed += 4; self.gpu_consumed += 4; bus.instruction_tcycles += 4;

        value
    }

    #[inline]
    fn fetch16(&mut self, bus: &mut MemoryBus) -> u16 {
        // Little-endian: lo then hi
        let lo = self.fetch8(bus) as u16;
        let hi = self.fetch8(bus) as u16;
        (hi << 8) | lo
    }

    fn push16(&mut self, bus: &mut MemoryBus, value: u16) {
        // DMG push: pre-decrement then write hi, pre-decrement then write lo.
        // Each SP decrement is an IDU operation; if SP is in OAM range, it
        // triggers IDU Write Corruption before the actual write.
        bus.idu_oam_corrupt(self.sp);
        self.sp = self.sp.wrapping_sub(1);
        bus.write_byte(self.sp, (value >> 8) as u8);
        bus.service_gpu(4); self.gpu_consumed += 4;
        bus.instruction_tcycles += 4;
        bus.idu_oam_corrupt(self.sp);
        self.sp = self.sp.wrapping_sub(1);
        bus.write_byte(self.sp, (value & 0x00ff) as u8);
        bus.service_gpu(4); self.gpu_consumed += 4;
        bus.instruction_tcycles += 4;
    }

    fn pop16(&mut self, bus: &mut MemoryBus) -> u16 {
        // Pop: read lo then hi, post-increment SP each read.
        // Hardware quirk: POP triggers only 3 times instead of the expected 4:
        // "one read, one glitched write, and another read without a glitched write."
        // The first SP++ fires IDU Write Corruption; the second SP++ does NOT.
        let lo = bus.read_byte(self.sp) as u16;
        bus.service_gpu(4); self.gpu_consumed += 4;
        bus.idu_oam_corrupt(self.sp);
        bus.instruction_tcycles += 4;
        self.sp = self.sp.wrapping_add(1);
        let hi = bus.read_byte(self.sp) as u16;
        bus.service_gpu(4); self.gpu_consumed += 4;
        bus.instruction_tcycles += 4;
        // Second SP++ IDU is suppressed per hardware behaviour.
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
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                bus.write_byte(addr, val);
                bus.service_gpu(4); self.gpu_consumed += 4;
            }
            LoadByteTarget::MemReg16(Reg16::BC) => {
                let addr = self.regs.get_bc();
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                bus.write_byte(addr, val);
                bus.service_gpu(4); self.gpu_consumed += 4;
            }
            LoadByteTarget::MemReg16(Reg16::DE) => {
                let addr = self.regs.get_de();
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                bus.write_byte(addr, val);
                bus.service_gpu(4); self.gpu_consumed += 4;
            }
            LoadByteTarget::MemImm16 => {
                let addr = self.fetch16(bus);
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                self.write_byte_with_ff50_log(bus, addr, val);
                bus.service_gpu(4); self.gpu_consumed += 4;
            }
            LoadByteTarget::MemImm8 => {
                let lo = self.fetch8(bus) as u16;
                let addr = 0xff00 | lo;
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                self.write_byte_with_ff50_log(bus, addr, val);
                bus.service_gpu(4); self.gpu_consumed += 4;
            }
            LoadByteTarget::MemHighC => {
                let addr = 0xff00 | (self.regs.c as u16);
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                self.write_byte_with_ff50_log(bus, addr, val);
                bus.service_gpu(4); self.gpu_consumed += 4;
            }
            LoadByteTarget::HLI => {
                let addr = self.regs.get_hl();
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                bus.write_byte(addr, val);
                bus.service_gpu(4); self.gpu_consumed += 4;
            }
            // AF, SP, PC not valid byte targets via this enum

            LoadByteTarget::MemReg16(_) => {
                // NEW: show which one before trapping
                eprintln!("write_to_target: UNHANDLED tgt = {:?}", tgt);
                self.trap_unknown(DecodeError::UnknownOpcode(0x00, false));
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
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                let v = bus.read_byte(addr);
                bus.service_gpu(4); self.gpu_consumed += 4;
                v
            }
            LoadByteSource::MemReg16(Reg16::BC) => {
                let addr = self.regs.get_bc();
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                let v = bus.read_byte(addr);
                bus.service_gpu(4); self.gpu_consumed += 4;
                v
            }
            LoadByteSource::MemReg16(Reg16::DE) => {
                let addr = self.regs.get_de();
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                let v = bus.read_byte(addr);
                bus.service_gpu(4); self.gpu_consumed += 4;
                v
            }
            LoadByteSource::MemImm16 => {
                let addr = self.fetch16(bus);
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                let v = bus.read_byte(addr);
                bus.service_gpu(4); self.gpu_consumed += 4;
                v
            }
            LoadByteSource::MemImm8 => {
                let lo = self.fetch8(bus) as u16;
                let addr = 0xff00 | lo;
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                let v = bus.read_byte(addr);
                bus.service_gpu(4); self.gpu_consumed += 4;
                v
            }
            LoadByteSource::MemHighC => {
                let addr = 0xff00 | (self.regs.c as u16);
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                let v = bus.read_byte(addr);
                bus.service_gpu(4); self.gpu_consumed += 4;
                v
            }
            LoadByteSource::D8 => self.fetch8(bus),
            LoadByteSource::HLI => {
                let addr = self.regs.get_hl();
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                let v = bus.read_byte(addr);
                bus.service_gpu(4); self.gpu_consumed += 4;
                v
            }
            LoadByteSource::MemReg16(_) => {
                // NEW: show which one before trapping
                eprintln!("read_from_source: UNHANDLED src = {:?}", src);
                self.trap_unknown(DecodeError::UnknownOpcode(0x00, false));
            }
        }
    }

    fn exec(&mut self, insn: Instruction, bus: &mut MemoryBus) -> u32 {
        match insn {
            // ---------- Misc ----------
            Instruction::NOP => 4,
            Instruction::HALT => {
                // If IME is enabled or no interrupts are pending, normal HALT
                if self.ime || !self.any_pending_interrupt(bus) {
                    self.halted = true;
                    self.halt_bug_armed = false;
                } else {
                    // HALT bug: IME=0 and interrupt pending.
                    self.halted = false;
                    self.halt_bug_armed = true;
                }
                self.trace_cpu_halt_state(bus);
                4
            }

            Instruction::STOP => {
                self.pc = self.pc.wrapping_add(1);
                4
            }

            Instruction::RLCA => {
                let old_bit7 = (self.regs.a & 0x80) != 0;
                self.regs.a = (self.regs.a << 1) | (if old_bit7 { 1 } else { 0 });

                // Flags
                self.regs.f.zero = false; // Z always cleared
                self.regs.f.subtract = false; // N cleared
                self.regs.f.half_carry = false; // H cleared
                self.regs.f.carry = old_bit7; // C = old bit 7

                4 // cycles
            }

            Instruction::RLA => {
                let old_carry = if self.regs.f.carry { 1 } else { 0 };
                let new_carry = (self.regs.a & 0x80) != 0;

                self.regs.a = (self.regs.a << 1) | old_carry;

                // Flags
                self.regs.f.zero = false; // Z cleared
                self.regs.f.subtract = false; // N cleared
                self.regs.f.half_carry = false; // H cleared
                self.regs.f.carry = new_carry; // C = old bit 7

                4
            }

            Instruction::RRCA => {
                let old_bit0 = (self.regs.a & 0x01) != 0;
                self.regs.a = (self.regs.a >> 1) | (if old_bit0 { 0x80 } else { 0 });

                // Flags
                self.regs.f.zero = false; // Z cleared
                self.regs.f.subtract = false; // N cleared
                self.regs.f.half_carry = false; // H cleared
                self.regs.f.carry = old_bit0; // C = old bit 0

                4
            }

            Instruction::RRA => {
                let old_carry = if self.regs.f.carry { 0x80 } else { 0 };
                let new_carry = (self.regs.a & 0x01) != 0;

                self.regs.a = (self.regs.a >> 1) | old_carry;

                // Flags
                self.regs.f.zero = false; // Z cleared
                self.regs.f.subtract = false; // N cleared
                self.regs.f.half_carry = false; // H cleared
                self.regs.f.carry = new_carry; // C = old bit 0

                4
            }

            Instruction::AddHL(reg) => {
                let hl = self.regs.get_hl();
                let val = match reg {
                    Reg16::BC => self.regs.get_bc(),
                    Reg16::DE => self.regs.get_de(),
                    Reg16::HL => self.regs.get_hl(),
                    Reg16::SP => self.sp,
                    _ => unreachable!(),
                };

                let result = hl.wrapping_add(val);

                // Flags:
                // Z unaffected
                // N = 0
                // H = carry from bit 11
                // C = carry from bit 15
                self.regs.f.subtract = false;
                self.regs.f.half_carry = (hl & 0x0fff) + (val & 0x0fff) > 0x0fff;
                self.regs.f.carry = (hl as u32) + (val as u32) > 0xffff;

                self.regs.set_hl(result);
                8
            }

            // ---------- ADD SP, r8 ----------
            Instruction::AddSpR8 => {
                let offset = self.fetch8(bus) as i8 as i16;
                let sp = self.sp;
                let result = sp.wrapping_add(offset as u16);

                // Flags according to Mooneye test:
                self.regs.f.zero = false; // Z = 0
                self.regs.f.subtract = false; // N = 0
                self.regs.f.half_carry = ((sp ^ (offset as u16) ^ result) & 0x10) != 0; // H = carry from bit 3
                self.regs.f.carry = ((sp ^ (offset as u16) ^ result) & 0x100) != 0; // C = carry from bit 7

                self.sp = result;
                16
            }

            // ---------- LD HL, SP+r8 ----------
            Instruction::LdHlSpR8 => {
                let offset = self.fetch8(bus) as i8 as i16;
                let sp = self.sp;
                let result = sp.wrapping_add(offset as u16);

                // Flags according to Mooneye test:
                self.regs.f.zero = false; // Z = 0
                self.regs.f.subtract = false; // N = 0
                self.regs.f.half_carry = ((sp ^ (offset as u16) ^ result) & 0x10) != 0; // H = carry from bit 3
                self.regs.f.carry = ((sp ^ (offset as u16) ^ result) & 0x100) != 0; // C = carry from bit 7

                self.regs.set_hl(result); // HL = SP + r8
                12
            }

            // ---------- DAA ----------
            Instruction::DAA => {
                let mut a = self.regs.a;
                let mut adjust = 0;
                let mut carry = self.regs.f.carry;

                if !self.regs.f.subtract {
                    if self.regs.f.half_carry || a & 0x0f > 9 {
                        adjust |= 0x06;
                    }
                    if self.regs.f.carry || a > 0x99 {
                        adjust |= 0x60;
                        carry = true;
                    }
                    a = a.wrapping_add(adjust);
                } else {
                    if self.regs.f.half_carry {
                        adjust |= 0x06;
                    }
                    if self.regs.f.carry {
                        adjust |= 0x60;
                    }
                    a = a.wrapping_sub(adjust);
                }

                self.regs.a = a;
                self.regs.f.zero = a == 0;
                self.regs.f.half_carry = false;
                self.regs.f.carry = carry;
                4
            }
            Instruction::CPL => {
                self.regs.a = !self.regs.a;
                self.regs.f.subtract = true;
                self.regs.f.half_carry = true;
                4
            }

            Instruction::SCF => {
                self.regs.f.subtract = false;
                self.regs.f.half_carry = false;
                self.regs.f.carry = true;
                4
            }

            Instruction::CCF => {
                self.regs.f.subtract = false;
                self.regs.f.half_carry = false;
                self.regs.f.carry = !self.regs.f.carry;
                4
            }

            // ---------- ALU group ----------
            Instruction::AddA(src) => self.exec_alu_op(bus, src, CPU::alu_add8),
            Instruction::AdcA(src) => self.exec_alu_op(bus, src, CPU::alu_adc8),
            Instruction::SubA(src) => self.exec_alu_op(bus, src, CPU::alu_sub8),
            Instruction::SbcA(src) => self.exec_alu_op(bus, src, CPU::alu_sbc8),
            Instruction::AndA(src) => self.exec_alu_op(bus, src, CPU::alu_and8),
            Instruction::XorA(src) => self.exec_alu_op(bus, src, CPU::alu_xor8),
            Instruction::OrA(src)  => self.exec_alu_op(bus, src, CPU::alu_or8),
            Instruction::CpA(src) => {
                let b = self.read_from_source(bus, src);
                let a = self.regs.a;
                self.alu_cp8(a, b); // A unchanged, only flags set
                CPU::alu_cycles(&src)
            }

            // ---------- HL auto-increment/decrement ----------
            Instruction::LdHliA => {
                let addr = self.regs.get_hl();
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                // Write and IDU increment in the same M-cycle → single Write Corruption.
                bus.oam_write_during_inc(addr, self.regs.a);
                bus.service_gpu(4); self.gpu_consumed += 4;
                self.regs.set_hl(addr.wrapping_add(1));
                8
            }
            Instruction::LdAHli => {
                let addr = self.regs.get_hl();
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                // Read and IDU increment in the same M-cycle → Read-During-Inc/Dec Corruption.
                self.regs.a = bus.oam_read_during_inc(addr);
                bus.service_gpu(4); self.gpu_consumed += 4;
                self.regs.set_hl(addr.wrapping_add(1));
                8
            }
            Instruction::LdHldA => {
                let addr = self.regs.get_hl();
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                // Write and IDU decrement in the same M-cycle → single Write Corruption.
                bus.oam_write_during_inc(addr, self.regs.a);
                bus.service_gpu(4); self.gpu_consumed += 4;
                self.regs.set_hl(addr.wrapping_sub(1));
                8
            }
            Instruction::LdAHld => {
                let addr = self.regs.get_hl();
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                // Read and IDU decrement in the same M-cycle → Read-During-Inc/Dec Corruption.
                self.regs.a = bus.oam_read_during_inc(addr);
                bus.service_gpu(4); self.gpu_consumed += 4;
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
            Instruction::INC8(LoadByteTarget::A) => inc8_reg!(self, a),
            Instruction::INC8(LoadByteTarget::B) => inc8_reg!(self, b),
            Instruction::INC8(LoadByteTarget::C) => inc8_reg!(self, c),
            Instruction::INC8(LoadByteTarget::D) => inc8_reg!(self, d),
            Instruction::INC8(LoadByteTarget::E) => inc8_reg!(self, e),
            Instruction::INC8(LoadByteTarget::H) => inc8_reg!(self, h),
            Instruction::INC8(LoadByteTarget::L) => inc8_reg!(self, l),
            Instruction::INC8(LoadByteTarget::MemReg16(Reg16::HL)) => {
                let addr = self.regs.get_hl();
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                let v = bus.read_byte(addr);
                bus.service_gpu(4); self.gpu_consumed += 4;
                let r = self.alu_inc8(v);
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                bus.write_byte(addr, r);
                bus.service_gpu(4); self.gpu_consumed += 4;
                12
            }

            // ---------- 8-bit DEC ----------
            Instruction::DEC8(LoadByteTarget::A) => dec8_reg!(self, a),
            Instruction::DEC8(LoadByteTarget::B) => dec8_reg!(self, b),
            Instruction::DEC8(LoadByteTarget::C) => dec8_reg!(self, c),
            Instruction::DEC8(LoadByteTarget::D) => dec8_reg!(self, d),
            Instruction::DEC8(LoadByteTarget::E) => dec8_reg!(self, e),
            Instruction::DEC8(LoadByteTarget::H) => dec8_reg!(self, h),
            Instruction::DEC8(LoadByteTarget::L) => dec8_reg!(self, l),
            Instruction::DEC8(LoadByteTarget::MemReg16(Reg16::HL)) => {
                let addr = self.regs.get_hl();
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                let v = bus.read_byte(addr);
                bus.service_gpu(4); self.gpu_consumed += 4;
                let r = self.alu_dec8(v);
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                bus.write_byte(addr, r);
                bus.service_gpu(4); self.gpu_consumed += 4;
                12
            }

            // ---------- LD r,d8 (explicit to ensure 8 cycles) ----------
            Instruction::LD(LoadType::Byte(LoadByteTarget::A, LoadByteSource::D8)) => {
                ld_d8!(self, bus, a)
            }
            Instruction::LD(LoadType::Byte(LoadByteTarget::B, LoadByteSource::D8)) => {
                ld_d8!(self, bus, b)
            }
            Instruction::LD(LoadType::Byte(LoadByteTarget::C, LoadByteSource::D8)) => {
                ld_d8!(self, bus, c)
            }
            Instruction::LD(LoadType::Byte(LoadByteTarget::D, LoadByteSource::D8)) => {
                ld_d8!(self, bus, d)
            }
            Instruction::LD(LoadType::Byte(LoadByteTarget::E, LoadByteSource::D8)) => {
                ld_d8!(self, bus, e)
            }
            Instruction::LD(LoadType::Byte(LoadByteTarget::H, LoadByteSource::D8)) => {
                ld_d8!(self, bus, h)
            }
            Instruction::LD(LoadType::Byte(LoadByteTarget::L, LoadByteSource::D8)) => {
                ld_d8!(self, bus, l)
            }
            // 0x36: LD (HL), n — fetch immediate + write to memory = 12 cycles
            Instruction::LD(LoadType::Byte(LoadByteTarget::MemReg16(Reg16::HL), LoadByteSource::D8)) => {
                let n = self.fetch8(bus);
                let addr = self.regs.get_hl();
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                bus.write_byte(addr, n);
                bus.service_gpu(4); self.gpu_consumed += 4;
                12
            }

            // ---------- Absolute addressing ----------
            Instruction::LD(LoadType::Byte(LoadByteTarget::MemImm16, LoadByteSource::A)) => {
                let addr = self.fetch16(bus);
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                self.write_byte_with_ff50_log(bus, addr, self.regs.a);
                bus.service_gpu(4); self.gpu_consumed += 4;
                16
            }
            Instruction::LD(LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemImm16)) => {
                let addr = self.fetch16(bus);
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                self.regs.a = bus.read_byte(addr);
                bus.service_gpu(4); self.gpu_consumed += 4;
                16
            }

            // ---------- High-RAM I/O: (FF00+a8), (FF00+C) ----------
            Instruction::LD(LoadType::Byte(LoadByteTarget::MemImm8, LoadByteSource::A)) => {
                // This path calls write_to_target() which we will patch below to log when addr==FF50
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
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                self.write_byte_with_ff50_log(bus, addr, self.regs.a);
                bus.service_gpu(4); self.gpu_consumed += 4;
                8
            }
            Instruction::LD(LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemHighC)) => {
                let addr = 0xff00 | (self.regs.c as u16);
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                self.regs.a = bus.read_byte(addr);
                bus.service_gpu(4); self.gpu_consumed += 4;
                8
            }

            // ---------- (BC)/(DE) ----------
            Instruction::LD(
                LoadType::Byte(LoadByteTarget::MemReg16(Reg16::BC), LoadByteSource::A),
            ) => {
                let addr = self.regs.get_bc();
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                bus.write_byte(addr, self.regs.a);
                bus.service_gpu(4); self.gpu_consumed += 4;
                8
            }
            Instruction::LD(
                LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemReg16(Reg16::BC)),
            ) => {
                let addr = self.regs.get_bc();
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                self.regs.a = bus.read_byte(addr);
                bus.service_gpu(4); self.gpu_consumed += 4;
                8
            }
            Instruction::LD(
                LoadType::Byte(LoadByteTarget::MemReg16(Reg16::DE), LoadByteSource::A),
            ) => {
                let addr = self.regs.get_de();
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                bus.write_byte(addr, self.regs.a);
                bus.service_gpu(4); self.gpu_consumed += 4;
                8
            }
            Instruction::LD(
                LoadType::Byte(LoadByteTarget::A, LoadByteSource::MemReg16(Reg16::DE)),
            ) => {
                let addr = self.regs.get_de();
                bus.service_timer(4); self.timer_consumed += 4; bus.instruction_tcycles += 4;
                self.regs.a = bus.read_byte(addr);
                bus.service_gpu(4); self.gpu_consumed += 4;
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

            Instruction::JPHL => {
                self.pc = self.regs.get_hl();
                4
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

            Instruction::RLC(t) => self.cb_rw(bus, t, CPU::op_rlc),
            Instruction::RRC(t) => self.cb_rw(bus, t, CPU::op_rrc),
            Instruction::RL(t) => self.cb_rw(bus, t, CPU::op_rl),
            Instruction::RR(t) => self.cb_rw(bus, t, CPU::op_rr),
            Instruction::SLA(t) => self.cb_rw(bus, t, CPU::op_sla),
            Instruction::SRA(t) => self.cb_rw(bus, t, CPU::op_sra),
            Instruction::SRL(t) => self.cb_rw(bus, t, CPU::op_srl),
            Instruction::SWAP(t) => self.cb_rw(bus, t, CPU::op_swap),

            Instruction::BIT(bit, t) => {
                let cycles = self.cb_read_only(bus, t, |cpu, v| cpu.cb_bit_test_flags(bit, v));

                if self.trace && bit == 7 && matches!(t, PrefixTarget::H) {
                    if self.regs.f.zero {
                        println!(
                            "Boot clear-down loop complete: HL crossed below 0x8000 (HL={:04X})",
                            self.regs.get_hl()
                        );
                    }
                }

                cycles
            }
            Instruction::RES(bit, t) => self.cb_rw(bus, t, |_, v| CPU::cb_bit_res(v, bit)),
            Instruction::SET(bit, t) => self.cb_rw(bus, t, |_, v| CPU::cb_bit_set(v, bit)),

            // --- EI / DI / RETI ---
            Instruction::EI => {
                // Schedule IME enabling after the *next* instruction completes.
                // (One tick because we decrement once right after EI, then once after the next instruction.)
                self.ei_delay = 1;
                4
            }
            Instruction::DI => {
                self.ime = false;
                self.ei_delay = 0;
                self.halt_bug_armed = false;
                4
            }
            Instruction::RETI => {
                let addr = self.pop16(bus);
                self.pc = addr;
                self.ime = true;
                if trace::enabled(TraceCategory::CpuCtrl) {
                    eprintln!(
                        "event=cpu_ctrl step={} action=reti ime={} pc={:04X}",
                        self.step_index,
                        self.ime as u8,
                        self.pc
                    );
                }
                16
            }

            // ---------- 16-bit INC ----------
            Instruction::INC(IncDecTarget::BC) => {
                let old = self.regs.get_bc();
                bus.idu_oam_corrupt(old);
                self.regs.set_bc(old.wrapping_add(1));
                8
            }
            Instruction::INC(IncDecTarget::DE) => {
                let old = self.regs.get_de();
                bus.idu_oam_corrupt(old);
                self.regs.set_de(old.wrapping_add(1));
                8
            }
            Instruction::INC(IncDecTarget::HL) => {
                let old = self.regs.get_hl();
                bus.idu_oam_corrupt(old);
                self.regs.set_hl(old.wrapping_add(1));
                8
            }
            Instruction::INC(IncDecTarget::SP) => {
                bus.idu_oam_corrupt(self.sp);
                self.sp = self.sp.wrapping_add(1);
                8
            }

            // ---------- 16-bit DEC ----------
            Instruction::DEC(IncDecTarget::BC) => {
                let old = self.regs.get_bc();
                bus.idu_oam_corrupt(old);
                self.regs.set_bc(old.wrapping_sub(1));
                8
            }
            Instruction::DEC(IncDecTarget::DE) => {
                let old = self.regs.get_de();
                bus.idu_oam_corrupt(old);
                self.regs.set_de(old.wrapping_sub(1));
                8
            }
            Instruction::DEC(IncDecTarget::HL) => {
                let old = self.regs.get_hl();
                bus.idu_oam_corrupt(old);
                self.regs.set_hl(old.wrapping_sub(1));
                8
            }
            Instruction::DEC(IncDecTarget::SP) => {
                bus.idu_oam_corrupt(self.sp);
                self.sp = self.sp.wrapping_sub(1);
                8
            }

            // ---------- Fallback ----------
            _ => {
                eprintln!("UNHANDLED INSTRUCTION: {:?} at PC={:04X}", insn, self.last_pc);
                self.trap_unknown(DecodeError::UnknownOpcode(0x00, false))
            }
        }
    }

    /// Run one of the 8 binary ALU ops that read src, apply op, and write back to A.
    #[inline]
    fn exec_alu_op(
        &mut self,
        bus: &mut MemoryBus,
        src: LoadByteSource,
        op: fn(&mut CPU, u8, u8) -> u8,
    ) -> u32 {
        let b = self.read_from_source(bus, src);
        let a = self.regs.a;
        let result = op(self, a, b);
        self.regs.a = result;
        CPU::alu_cycles(&src)
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
        panic!("TRAP: pc={:04X} op={:02X} err={:?}", self.last_pc, self.last_op, e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_cpu_bus(code: &[u8], start: u16) -> (CPU, MemoryBus) {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();
        cpu.pc = start;
        for (i, b) in code.iter().enumerate() {
            bus.write_byte(start.wrapping_add(i as u16), *b);
        }
        (cpu, bus)
    }

    #[test]
    fn halt_bug_suppresses_next_pc_increment_once() {
        let start = 0xc000;
        let (mut cpu, mut bus) = setup_cpu_bus(&[0x76, 0x00, 0x00], start); // HALT, NOP, NOP
        cpu.ime = false;
        bus.write_byte(0xffff, 0x01); // IE: VBlank
        bus.write_byte(0xff0f, 0x01); // IF: VBlank pending

        let c1 = cpu.step(&mut bus);
        assert_eq!(c1, 4);
        assert_eq!(cpu.pc, start.wrapping_add(1));
        assert!(cpu.halt_bug_armed);
        assert!(!cpu.halted);

        let c2 = cpu.step(&mut bus);
        assert_eq!(c2, 4);
        assert_eq!(cpu.pc, start.wrapping_add(1)); // no increment once
        assert!(!cpu.halt_bug_armed);

        let c3 = cpu.step(&mut bus);
        assert_eq!(c3, 4);
        assert_eq!(cpu.pc, start.wrapping_add(2));
    }
}
