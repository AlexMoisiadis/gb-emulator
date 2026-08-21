# gb-emulator

A cycle-accurate **DMG (original Game Boy)** emulator written in Rust.

## Features

- **CPU** — Full SM83 instruction set with correct M-cycle timing, HALT bug emulation
- **PPU** — 456-dot scanlines, 4 PPU modes, sprites, window layer, OAM DMA, OAM bug emulation
- **APU** — All 4 channels (pulse ×2, wave, noise), 512 Hz frame sequencer, real-time audio via `cpal`
- **Timer** — DIV/TIMA/TMA with edge-triggered overflow and TMA reload delay
- **Cartridge** — No-MBC and MBC1 mappers
- **Viewer** — `winit` + `pixels` GUI at 4× scale (~59.73 Hz)
- **Headless runner** — Scriptable frame runner for automated testing and visual regression

## Building

Requires Rust 1.85 or later.

```bash
cargo build --release
```

## Running

You must supply your own ROM file.

```bash
# Interactive viewer
cargo run --release --bin viewer path/to/rom.gb

# Headless (for scripting / CI)
cargo run --release --bin headless path/to/rom.gb
```

## Viewer Controls

| Key | Action |
|-----|--------|
| Space | Pause / resume |
| N | Step one frame (while paused) |
| M | Toggle audio mute |
| F9 | Save screenshot |
| G / W / A / S | Toggle debug overlays (grid, window, attributes, sprites) |

## Headless Environment Variables

```bash
GB_HEADLESS_FRAMES=300           # frames to run before stopping
GB_HEADLESS_SAVE_FINAL=1         # save final frame as PNG
GB_HEADLESS_FINAL_PATH=out.png   # output path for final frame
GB_HEADLESS_HASH=1               # print FNV-1a hash of final frame
GB_HEADLESS_TIMEOUT_MODE=lcd_aware
GB_HEADLESS_TIMEOUT_DOTS=702240  # per-frame dot budget; raise for LCD-off windows
```

The headless binary detects [Blargg test ROMs](https://gbdev.gg8.se/wiki/articles/Test_ROMs) on two channels — the `$A000` protocol and the serial port — and streams their text to stdout.

Every run ends with a `completion=` line on stderr naming why it stopped, and the exit code is meaningful:

| Code | Meaning |
|------|---------|
| `0` | Passed (`blargg_pass`, `serial_pass`), or ran to completion with no verdict channel |
| `1` | Failed (`blargg_fail`, `serial_fail`) |
| `2` | No verdict reached — `blargg_no_verdict` (stuck) or `frame_budget_exhausted` |

ROMs reporting only via an on-screen CRC (`halt_bug`) or the `ld b,b` register convention (mooneye) cannot be verdicted automatically; they report `no_blargg_signature` and exit `0` regardless of result. Use `GB_HEADLESS_SAVE_FINAL=1` and inspect the frame for those.

## Test Status

| Test suite | Status |
|-----------|--------|
| `cpu_instrs` | ✅ All 11 pass |
| `instr_timing` | ✅ Pass |
| `mem_timing` | ✅ All 3 pass |
| `mem_timing-2` | ✅ All 3 pass |
| `halt_bug` | ✅ Pass |
| `dmg_sound` | ✅ All 12 pass |
| `dmg-acid2` | ✅ Pixel-exact against the reference frame |
| `oam_bug` | 🔧 7 of 8 — `8-instr_effect` fails |
| `interrupt_time` | ❌ Fails — CRC `7F8F4AAF`, expected `B511F33D` |

### Obtaining the test ROMs

Test ROMs are **not** vendored here — supply your own. Every result above was
measured against [c-sp/game-boy-test-roms](https://github.com/c-sp/game-boy-test-roms)
release **v7.0**, which bundles blargg, mealybug-tearoom-tests, mooneye and
dmg-acid2 in one archive.

[`test-roms.lock`](test-roms.lock) pins exactly which files those were: it
records the archive's SHA-256, then one line per ROM giving its SHA-256, its
path inside the archive, and where this repo's harness expects it. Verifying
your copies against it means a mismatch shows up immediately rather than as a
mysterious test failure.

Note `src/roms/test/mooneye/` is misnamed: 31 of its 32 ROMs are
mealybug-tearoom-tests PPU tests.

## Debug / Trace Feature Flags

| Flag | Effect |
|------|--------|
| `trace_apu` | APU register writes, trigger/length events, frame sequencer steps → stderr |
| `trace_ppu` | PPU/GPU trace logging |
| `trace_input` | Input system trace logging |
| `debug_timing` | Extra CPU timing debug output |

```bash
cargo run --features trace_apu --bin viewer rom.gb 2>apu.log
```

## Architecture

```
src/
  cpu.rs            SM83 CPU — full instruction set, correct M-cycle counts
  gpu.rs            PPU — 456-dot scanlines, 4 modes, OAM DMA, OAM bug
  apu/              4-channel audio with 512 Hz frame sequencer
    mod.rs          Master APU: register dispatch, NR52, sample output
    pulse.rs        Shared pulse channel logic (Ch1 + Ch2)
    ch1.rs          Ch1 — pulse + frequency sweep
    ch2.rs          Ch2 — pulse
    ch3.rs          Ch3 — wave RAM playback
    ch4.rs          Ch4 — noise (LFSR)
    frame_seq.rs    Frame sequencer (length / sweep / envelope clocking)
  timer.rs          DIV/TIMA/TMA
  mmu.rs            64 KB address space mapping
  cart/             NoMBC and MBC1 cartridge mappers
  bus.rs            MemoryBus — ties all subsystems together per step
  bin/viewer.rs     winit + pixels interactive GUI
  bin/headless.rs   Scriptable test runner with Blargg ROM support
  bin/image_diff.rs Pixel-level PNG comparator for visual regression
```

The main loop is **CPU → Bus → Peripherals**: [`cpu.rs`](src/cpu.rs) drives the fetch-decode-execute cycle; after each instruction [`bus.rs`](src/bus.rs) advances the GPU, timer, and APU by the elapsed T-cycles.

## License

MIT — see [LICENSE](LICENSE).
