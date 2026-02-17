use gb_emulator::bus::MemoryBus;
use gb_emulator::cpu::CPU;


fn main() {
    let bus = MemoryBus::default();
    let cpu = CPU::new(bus);
    println!("CPU ready. PC = 0x{:04X}", cpu.pc);

    // cpu.step(); // optional
}