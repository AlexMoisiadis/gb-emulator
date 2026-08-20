# 09-wave read while on

**Suite:** dmg_sound
**Source:** [09-wave read while on.s](../../../src/roms/test/blargg-test-roms/dmg_sound/source/09-wave read while on.s)
**ROM:** [09-wave read while on.gb](../../../src/roms/test/blargg-test-roms/dmg_sound/rom_singles/09-wave read while on.gb)
**Expected CRC (DMG):** `0x118A3620`
**Expected CRC (CGB):** `0x270DA9A3`

> Part of the CH3 wave-RAM-while-on cluster: **09** (CPU *reads*), [10](10-wave trigger while on.md) (re-*trigger* corruption), [12](12-wave write while on.md) (CPU *writes*). All three share the same skeleton and the same probe offset of **+208 T-cycles after the trigger write**; only the operation performed at the probe differs.

## Summary
Verifies the DMG rule that while CH3 is enabled, wave RAM (`$FF30`–`$FF3F`) is **not** normally CPU-readable: a read returns `$FF` unless it happens in the ~2-T-cycle window in which the wave channel itself is fetching a sample byte, in which case it returns **the byte at the channel's current position** (regardless of which of the 16 addresses was read). The test runs 69 iterations; each iteration triggers CH3 with a slightly higher frequency so the channel's first sample fetch happens **2 T-cycles earlier than the previous iteration**, then immediately drops the period to 4 T-cycles and reads `$FF30` at a *fixed* time. Sweeping the fetch grid past a fixed probe in 2-T steps makes the probe land alternately on and off the access window, so the expected output alternates between `$FF` and an ascending wave-RAM byte. It is simultaneously a test of the CH3 frequency-timer reload period, the trigger→first-sample delay, and the fact that the channel keeps fetching samples while muted (`NR32=0`, `NR51=0`).

## Hardware behaviour exercised
- **Wave RAM read gate (DMG):** with CH3 active (`NR30` bit 7 = 1 *and* the channel enabled), `read($FF30..$FF3F)` returns `$FF` except during the brief window when the channel latches a sample byte, when it returns `wave_ram[current_sample_index >> 1]`. The window is roughly one 2 MHz APU tick ≈ **2 T-cycles**; it must be **≥2 T and <4 T** wide or this test cannot produce an alternating pattern (see *Emulator pitfalls*).
- **CH3 frequency timer period** = `(2048 − freq) × 2` T-cycles per *nibble* advance, and a wave-RAM byte is latched on every advance. `NR33 = $FE`, `NR34` low bits `= 7` ⇒ `freq = $7FE` ⇒ **4 T-cycles per fetch**, the fastest useful rate for this experiment.
- **NR33 writes do not reload the running timer.** The test writes `NR33 = $FE` 20 T-cycles *after* the trigger; the first fetch must still occur at the period loaded by the *trigger* (from `freq = $799 + n`), and only subsequent reloads use the new 4-T period. If your emulator reloads the counter on an `NRx3` write, the whole phase sweep collapses.
- **Trigger→first-fetch delay:** the trigger loads the period plus a small extra constant Δ (the "wave channel trigger delay"; SameBoy's `(len ^ 0x7FF) + 3` at 2 T/tick works out to Δ ≈ +4 T, other emulators use +6 T). This test is effectively a **calibration of Δ mod 4** — see the phase equation below.
- **On trigger the sample index resets to 0**, and the first fetch after the trigger reads index 1 (advance-then-read), i.e. byte 0. Fetch *j* reads nibble index `j mod 32` ⇒ byte `(j mod 32) >> 1`.
- **The channel keeps running while inaudible.** `NR51 = 0` ([:10](../../../src/roms/test/blargg-test-roms/dmg_sound/source/09-wave read while on.s)) and `NR32 = $00` ([:25](../../../src/roms/test/blargg-test-roms/dmg_sound/source/09-wave read while on.s)) mute the output but must not stop the frequency timer or the wave fetch.
- **Length counter is irrelevant:** `NR34 = $87` has bit 6 clear, so length is disabled; `NR31` is never written.
- **Wave RAM is freely writable while the channel is off:** `load_wave` writes `NR30 = $00` first (DAC off ⇒ channel off) before filling `$FF30..$FF3F`.

## DMG vs CGB
- Both `REQUIRE_DMG` and `REQUIRE_CGB` are commented out in the source ([:4-5](../../../src/roms/test/blargg-test-roms/dmg_sound/source/09-wave read while on.s)); the define is supplied on the wla command line (`wla -DREQUIRE_DMG …`). The shipped `dmg_sound` build therefore checks **`$118A3620`**. The byte-identical source in `cgb_sound/source/09-wave read while on.s` is built with `-DREQUIRE_CGB` and checks `$270DA9A3`.
- **CGB has no access window.** On CGB, reading any wave address while CH3 is on always returns the byte at the current position (this is asserted directly by `cgb_sound/source/12-wave.s:22` — *"Current byte readable at any wave addr"*). So the CGB output contains no `$FF`-from-blocked-read at all, and the CGB CRC differs.

## Sub-tests
**There are none.** This ROM never calls `set_test`, so `test_code` stays at the `$FF` set by `init_testing` ([common/testing.s:49](../../../src/roms/test/blargg-test-roms/dmg_sound/source/common/testing.s)). There is exactly one assertion: the single `check_crc_dmg_cgb` at [:12](../../../src/roms/test/blargg-test-roms/dmg_sound/source/09-wave read while on.s), covering all 69 iterations.

On mismatch, `check_crc_` prints the *computed* CRC and falls into `test_failed`, which maps `test_code == $FF` to **A = 1** ([common/testing.s:91-93](../../../src/roms/test/blargg-test-roms/dmg_sound/source/common/testing.s)) → `exit(1)` prints `"Failed"` and stores **`$01` at `$A000`**.

> **Correction to a common misreading:** result code `0x01` here does *not* mean "sub-test #1 failed". It means "the one and only CRC check failed" — i.e. at least one of the 69 printed bytes is wrong, anywhere in the run. The `09:fail`-style numbering only exists in the combined `dmg_sound.gb` (produced by `build_multi.s`, which is not shipped with the sources).

## Test sequence
1. [:10](../../../src/roms/test/blargg-test-roms/dmg_sound/source/09-wave read while on.s) — `NR51 = 0`: mute both output terminals for the whole run.
2. [:11](../../../src/roms/test/blargg-test-roms/dmg_sound/source/09-wave read while on.s) — `loop_n_times test,69` expands to `for_loop test,0,68,+1` ([common/macros.inc:47-65](../../../src/roms/test/blargg-test-roms/dmg_sound/source/common/macros.inc)): calls `test` 69 times with **A = n = 0…68**.
3. [:16-17](../../../src/roms/test/blargg-test-roms/dmg_sound/source/09-wave read while on.s) — `add $99` ⇒ `B = $99 + n` (`$99`…`$DD`, no wrap). This becomes `NR33`.
4. [:22-23](../../../src/roms/test/blargg-test-roms/dmg_sound/source/09-wave read while on.s) — `load_wave` ([cpu_instrs/source/common/apu.s:49-59](../../../src/roms/test/blargg-test-roms/cpu_instrs/source/common/apu.s)) writes `NR30 = $00` (channel off ⇒ wave RAM unlocked) then copies the 16-byte ramp `$00,$11,…,$FF` from [:40-41](../../../src/roms/test/blargg-test-roms/dmg_sound/source/09-wave read while on.s) into `$FF30..$FF3F`.
5. [:24-27](../../../src/roms/test/blargg-test-roms/dmg_sound/source/09-wave read while on.s) — `NR30 = $80` (DAC on), `NR32 = $00` (output level = mute), `NR33 = B`.
6. [:28](../../../src/roms/test/blargg-test-roms/dmg_sound/source/09-wave read while on.s) — `NR34 = $87`: trigger, length disabled, `freq_hi = 7` ⇒ **`freq = $700 | ($99+n) = 1945 + n`**, initial period `= 2 × (2048 − freq) = 206 − 2n` T-cycles (n = 0 → 206 T, n = 68 → 70 T).
7. [:31](../../../src/roms/test/blargg-test-roms/dmg_sound/source/09-wave read while on.s) — `NR33 = -2 = $FE` ⇒ `freq = $7FE` ⇒ every subsequent reload is **4 T-cycles**.
8. [:32](../../../src/roms/test/blargg-test-roms/dmg_sound/source/09-wave read while on.s) — `delay_clocks 176` = **44 M-cycles** exactly (`delay_ 44,0` → `push af`(4) + `ld a,15`(2) + `call delay_a_20_cycles`(35) + `pop af`(3) = 44 M; see [common/delay.s:105-128](../../../src/roms/test/blargg-test-roms/dmg_sound/source/common/delay.s)).
9. [:33](../../../src/roms/test/blargg-test-roms/dmg_sound/source/09-wave read while on.s) — `lda WAVE` = `ldh a,($30)`, 3 M-cycles, **bus read in the 3rd M-cycle**.
10. [:35](../../../src/roms/test/blargg-test-roms/dmg_sound/source/09-wave read while on.s) — `print_a` CRCs the raw byte via `update_crc` then prints 2 hex digits + a space (the digits and the space are printed with `print_char_nocrc`, so **only the raw byte value enters the CRC**; see [common/numbers.s:16-23, 94-107](../../../src/roms/test/blargg-test-roms/dmg_sound/source/common/numbers.s)).

### Exact probe timing
Take `t = 0` at the M-cycle in which the `NR34` trigger write commits ([:28](../../../src/roms/test/blargg-test-roms/dmg_sound/source/09-wave read while on.s)):

| M-cycle | Instruction | Event |
|---|---|---|
| 0 | `ldh (NR34),a` M3 | **trigger** |
| +1,+2 | `ld a,$FE` | |
| +3,+4,+5 | `ldh (NR33),a` | `NR33 = $FE` commits at **+5 M = +20 T** |
| +6 … +49 | `delay_clocks 176` | 44 M |
| +50,+51,+52 | `ldh a,(WAVE)` | **read commits at +52 M = +208 T** |

So the probe is always at **trigger + 208 T**, while the first fetch is at **trigger + (206 − 2n) + Δ**. Therefore

```
d(n) = probe − first_fetch = 2n + 2 − Δ        (T-cycles)
hit  ⇔  d(n) ≥ 0  and  d(n) mod 4 == 0
j(n) = d(n)/4 + 1                              (index of the fetch that is hit)
byte = wave[(j mod 32) >> 1] = ((j mod 32) >> 1) × $11
```

`Δ mod 4` selects the phase: Δ ≡ 2 (mod 4) ⇒ **even n hit**; Δ ≡ 0 (mod 4) ⇒ **odd n hit**. A 2-T error in your trigger delay inverts the whole pattern and produces a completely different CRC while looking structurally correct.

## Expected output
69 bytes printed as `XX ` (hex pair + space), all on one line, then `"Passed"`.

Derived sequence assuming Δ = 6 T (i.e. even iterations hit, first hit at n = 2) — **this is derived from the model above, not from hardware capture**; the alternation and the value progression are certain, the exact starting phase depends on Δ:

```
FF FF 00 FF 11 FF 11 FF 22 FF 22 FF 33 FF 33 FF
44 FF 44 FF 55 FF 55 FF 66 FF 66 FF 77 FF 77 FF
88 FF 88 FF 99 FF 99 FF AA FF AA FF BB FF BB FF
CC FF CC FF DD FF DD FF EE FF EE FF FF FF FF FF
00 FF 00 FF 11
```

Structure to check even if the phase is off:
- 34 "hit" iterations and 35 "miss" iterations.
- Miss ⇒ `$FF`. Hit ⇒ the wave byte, progressing `00, 11, 11, 22, 22, 33, 33, …, EE, EE, FF, FF`, then **wrapping** (fetch j = 32 wraps the 32-nibble position back to 0) to `00, 00, 11`.
- The two leading `$FF`s (n = 0, 1) are because the probe at +208 T arrives *before* the first fetch (period 206 T + Δ > 208 T).
- Note the trap at j = 30, 31 (n = 60, 62 under Δ = 6): the wave byte there genuinely *is* `$FF`, indistinguishable from a blocked read.

The printed text is also mirrored into cartridge RAM starting at `$A004` as a zero-terminated string ([common/shell.s:194-225](../../../src/roms/test/blargg-test-roms/dmg_sound/source/common/shell.s)) — dump it from your headless harness and diff it against the pattern above to find the *first* divergent iteration instead of guessing from the CRC.

## Emulator pitfalls
- **No access window implemented at all.** This repo's `Ch3::read_wave_ram` ([src/apu/ch3.rs:92-99](../../../src/apu/ch3.rs)) returns `wave_ram[wave_pos/2]` whenever the channel is active, with no `wave_just_read` gate. That is **CGB** behaviour. Result: every one of the 69 iterations prints a data byte and never `$FF` ⇒ guaranteed CRC failure. You need a flag set at the moment the sample byte is latched and cleared ~2 T-cycles later, and reads outside it must return `$FF` on DMG.
- **Window too wide or too narrow.** Period is 4 T and the phase shifts 2 T per iteration. A window ≥ 4 T ⇒ every iteration hits (no `$FF`s). A window of 0/exact-cycle-equality against a coarser APU step ⇒ every iteration misses (all `$FF`s). Only `2 ≤ window < 4` T produces the alternation.
- **APU stepped in whole-instruction chunks.** This test needs the CPU read to be observed by the APU at the correct T-cycle *inside* `ldh a,(n8)` — the read is in the 3rd M-cycle, i.e. 4 T before the instruction retires. If your bus advances the APU by the full instruction length *after* executing it (or before), your effective probe time is off by up to ±12 T, which is ±3 phases on a 4-T grid. Sub-instruction (per-M-cycle at minimum) APU catch-up is mandatory for 09/10/12.
- **Reloading the frequency timer on an `NR33`/`NR34` frequency write.** The `NR33 = $FE` at +20 T must *not* restart the counter; if it does, the first fetch lands at a fixed +24 T for every iteration, the phase never sweeps, and the output is constant.
- **Stopping the wave fetch when output is muted.** `NR32 = 0` (level 0) and `NR51 = 0` must not gate the frequency timer. If you skip ticking CH3 when its mixed contribution is zero, `wave_pos` never advances.
- **Wrong trigger delay Δ.** Off by 2 T ⇒ the `$FF`/data phase inverts. Off by 4 T ⇒ the data values shift by one fetch (`00 11 11 22 …` becomes `11 11 22 22 …`) and the number of leading `$FF`s changes.
- **`wave_pos` reset semantics.** Trigger must set the nibble index to 0 with the *first* post-trigger fetch reading index 1 (byte 0). Reading index 0 on the first fetch shifts every printed value by one byte for half the run.
- **Position wrap.** The index is 32 nibbles, not 16 bytes — fetch j = 32 must wrap to byte 0 (visible at the tail of the expected output).

## Notes
- CRC is standard reflected CRC-32 (poly `$EDB88320`, init `$FFFFFFFF`, final complement) over the raw byte values only ([common/crc.s](../../../src/roms/test/blargg-test-roms/dmg_sound/source/common/crc.s)); spaces and newlines are excluded. So a single wrong byte anywhere in the 69 fails the test with no locality information — use the `$A004` text buffer.
- No frame-sequencer/DIV synchronisation is needed anywhere in this test: every measurement is relative to the trigger, which reloads the CH3 period counter. That is why `sync_apu` is never called.
- Interrupts are disabled by the shell (`di` at [common/shell.s:88](../../../src/roms/test/blargg-test-roms/dmg_sound/source/common/shell.s)), so nothing can perturb the 44-M-cycle delay.
- The value of Δ is the one genuinely ambiguous parameter in this analysis. Everything else (208 T probe, 4 T period, 2 T per-iteration shift, the byte progression) follows directly from the assembly.
