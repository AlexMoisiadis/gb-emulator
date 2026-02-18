use gb_emulator::{ CPU, MemoryBus };

fn main() {
    let mut cpu = CPU::new();
    let mut bus = MemoryBus::new();

    // 0000: 06 17        LD B,0x17
    // 0002: 3E 42        LD A,0x42
    // 0004: EA 00 80     LD (0x8000),A
    // 0007: CD 0B 00     CALL 0x000B
    // 000A: C3 0A 00     JP 0x000A
    // 000B: 03           INC BC
    // 000C: C9           RET

    let rom: [u8; 16] = [
        0x06,
        0x17, // 0000: LD B,0x17
        0x3e,
        0x42, // 0002: LD A,0x42
        0xea,
        0x00,
        0x80, // 0004: LD (0x8000),A
        0xcd,
        0x0b,
        0x00, // 0007: CALL 0x000B
        0x03, // 000A: (unused; safe filler)
        0x03, // 000B: INC BC
        0xc9, // 000C: RET
        0xc3,
        0x0d,
        0x00, // 000D: JP 0x000D
    ];
    bus.load_rom(&rom);

    cpu.pc = 0x0000;
    cpu.regs.set_bc(0x0000);

    // Print first 7 bytes so we also see the 0x80 from the a16 operand
    println!("mem[0000..0007] = {:02X?}", (0..7).map(|i| bus.read_byte(i)).collect::<Vec<_>>());

    // Step 1: LD B,0x17
    cpu.step(&mut bus);
    assert_eq!(cpu.regs.b, 0x17);
    assert_eq!(cpu.regs.get_bc(), 0x1700);

    // Step 2: LD A,0x42
    cpu.step(&mut bus);
    assert_eq!(cpu.regs.a, 0x42);

    // Step 3: LD (0x8000),A
    cpu.step(&mut bus);
    assert_eq!(bus.read_byte(0x8000), 0x42);

    // Step 4: CALL 0x000B
    cpu.step(&mut bus);

    // Step 5: INC BC (in subroutine)
    cpu.step(&mut bus);

    // Step 6: RET (back to 0x000A)
    cpu.step(&mut bus);

    // Now BC should be 0x1701
    assert_eq!(cpu.regs.get_bc(), 0x1701);

    // Step 7: JP 0x000A (self-loop)
    cpu.step(&mut bus);

    println!("All assertions passed.");
}
