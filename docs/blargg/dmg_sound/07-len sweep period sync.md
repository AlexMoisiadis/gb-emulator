# 07-len sweep period sync

**Suite:** dmg_sound
**Source:** [07-len sweep period sync.s](../../../src/roms/test/blargg-test-roms/dmg_sound/source/07-len sweep period sync.s)
**ROM:** [07-len sweep period sync.gb](../../../src/roms/test/blargg-test-roms/dmg_sound/rom_singles/07-len sweep period sync.gb)

## Summary
Measures the *period* of internal APU clocks against the CPU, and asserts the synchronisation rules tying them to the DIV-driven 512 Hz frame sequencer. Three independent assertions: (1) the length-counter clock fires every 16384 T-cycles (256 Hz), (2) the sweep clock fires every 32768 T-cycles (128 Hz), (3) the length and sweep clocks are phase-aligned with one another (the sweep clock always coincides with a length clock). Additional sub-tests verify what happens when the APU is power-cycled (NR52 bit 7 toggled): on power-up, the FS must restart at a known step, and the first FS clock after power-up must occur at `(elapsed_div_cycles) mod 8192` from the power-up moment (i.e. the FS counter is *aligned to the DIV bit-12 falling edge*, not to NR52=$80). Each sub-test uses `test_timing` (lines 5–21) which counts CPU iterations of a tight `(NR52);AND $01;jr nz` loop and asserts the resulting `DE` value lands in the range `[expected, expected+4]`.

## Hardware behaviour exercised
- **256 Hz length clock** (sub-test 2): length counter for ch1 with length=1 should expire exactly 16384 T-cycles after the previous length clock.
- **128 Hz sweep clock** (sub-test 3): a ch1 trigger with max freq ($7FF), shift=1, period=1 should overflow at the second sweep clock — measured as 32768 T-cycles per overflow cycle.
- **Length/sweep phase alignment** (sub-test 4): length and sweep are clocked from the same FS — length on steps 0,2,4,6 and sweep on steps 2,6. Sub-test 4 sets up length=1 and measures the time to first length clock against the same yardstick as the sweep test on test 3 — they must come out to compatible numbers.
- **Power-up FS phase alignment** (sub-test 5): writing NR52=$80 does **not** reset the global FS phase. The FS is driven by a falling edge of DIV bit 12 (= every 8192 T-cycles of DIV-counter advance), and powering up the APU mid-FS-cycle means the *next* FS step fires `(8192 - (DIV % 8192))` T-cycles after power-up. The sub-test verifies this by running `test_power` (power-cycle + trigger + measure) at five different DIV offsets and asserting the timing changes accordingly.
- **Power-up resets the 128 Hz sweep divider** (sub-test 6): unlike the FS itself (which is DIV-locked), the sweep-clock divider INSIDE the APU *does* reset on power-up. After power-up, the first sweep clock is one full sweep period (8192 T-cycles × 4 = 32768) later — measured against the FS, which itself may not be at step 0.

## DMG vs CGB
No `check_crc_dmg_cgb`. The hardcoded expected DE values (`-$170`, `-$2E4`, `-$170`, `-$16F`, `-$B5`, `-$229`) match DMG cycle counts. CGB-02 has different sweep timing in a few places (the cgb version of this file has different constants).

## Sub-tests
| ID | Description | Setup | Expected `DE` initial |
|----|-------------|-------|-----------------------|
| 2  | Length period (256 Hz) is correct | sync_apu, NR14=$40 (no extra clock), NR11=$3F (length=1), NR12=$08, NR14=$C0 (trigger+len) | `-$170` = -368; loop adds 1 per iteration, channel goes off in ~4096 T-cycles = ~368 iterations of an 11-cycle loop |
| 3  | Sweep period (128 Hz) is correct | sync_sweep, NR10=$10 (per=1,sh=0), NR12=$08, NR13=$FF, NR14=$87 (trigger freq=$7FF) — but shift=0 means trigger doesn't calc | `-$2E4` = -740 — about 2x sub-test 2's count, confirming 128 Hz cadence |
| 4  | Sweep clock is synced with length | sync_sweep then NR14=$40, length=1, NR12=$08, NR14=$C0 — same as sub-test 2 but after sync_sweep instead of sync_apu | `-$170` — equal to sub-test 2's value, proving length and sweep are phase-aligned to the same FS |
| 5  | Powering up APU MODs next FS time with 8192 | 5 variants — `test_power` & `test_power_off` at different DIV offsets | `-$16F`, `-$B5`, `-$B5`, `-$B5`, `-$B5` — the first variant has slightly different timing because no extra `delay_clocks 8192` was run; the rest show the 8192-mod behaviour |
| 6  | Powering up APU resets 128 Hz sweep divider | 2 variants of `test_power2` at different sync_sweep phases | `-$229` — about half of sub-test 3's value, meaning the first sweep clock after power-up is one sweep-period (4 FS steps) away regardless of pre-power-up phase |

## Test sequence
1. [07-len sweep period sync.s:5-21](../../../src/roms/test/blargg-test-roms/dmg_sound/source/07-len sweep period sync.s) — `test_timing`: zero DE in caller, increment until NR52 bit 0 clears. After exit, assert `D==0` and `E<5`. The caller pre-loads DE with a negative number so DE *wraps to 0..4* if timing is exact.
2. Sub-test 2 ([line 25-32](../../../src/roms/test/blargg-test-roms/dmg_sound/source/07-len sweep period sync.s)) — length period measurement.
3. Sub-test 3 ([line 34-41](../../../src/roms/test/blargg-test-roms/dmg_sound/source/07-len sweep period sync.s)) — sweep period measurement.
4. Sub-test 4 ([line 43-50](../../../src/roms/test/blargg-test-roms/dmg_sound/source/07-len sweep period sync.s)) — phase alignment check.
5. Sub-test 5 ([line 52-75](../../../src/roms/test/blargg-test-roms/dmg_sound/source/07-len sweep period sync.s)) — five variants of power-cycle timing.
6. Sub-test 6 ([line 77-86](../../../src/roms/test/blargg-test-roms/dmg_sound/source/07-len sweep period sync.s)) — two variants of post-power-up sweep timing.
7. `test_power` (line 91): NR52=$80; NR14=$40; NR11=-1 (length=1); NR12=8; NR14=$C0; jp test_timing.
8. `test_power2` (line 99): NR52=$00; NR52=$80; NR10=$11 (per=1,sh=1); NR12=8; NR13=$00; NR14=$84 (trigger, freq=$400); jp test_timing.

## Expected output
Pass prints "Passed". Failure prints the sub-test's string + "Failed #N". No CRC check.

## Emulator pitfalls
This test is the most direct check that your *frame sequencer is DIV-locked*. The hang you're seeing strongly suggests the test enters `test_timing`'s tight loop and never exits — i.e. NR52 bit 0 never clears within the watchdog timeout. Concrete hypotheses, in priority order:
1. **FS not clocked by DIV bit 12 falling edge**: The DMG APU does not have an independent timer for the FS — it watches DIV bit 12 (i.e. the bit that toggles every 8192 T-cycles when DIV runs normally) and clocks the FS on the *falling edge*. If your emulator runs the FS off an internal 8192-cycle counter that resets on NR52=$80, sub-test 5 will fail and *test_timing* will measure the wrong number of cycles. Worse: if NR52=$80 doesn't reset that counter but it's not synced to DIV either, the channel might never expire.
2. **Length clock period off by a small constant**: A length-counter clocked every 16384 T-cycles (256 Hz) is the DMG spec; if you clock it every 8192 (512 Hz) or every 32768 (128 Hz), test_timing's measured DE value falls outside [0,4]. A common emulator bug is clocking length on FS steps 0,2,4,6,8 (8 clocks per FS revolution) instead of 4.
3. **Hang from never-disabling channel**: If your length counter doesn't tick down (e.g. you only decrement when `length_enabled` was already 1 *before* the trigger, missing the 0→1 transition's clock), then writing NR11=$3F (length=1) followed by NR14=$C0 leaves the channel with length=1 and it never expires → infinite hang in `test_timing`. *This is the most likely cause of "07 hangs"*.
4. **Power-up does not reset the sweep divider** (sub-test 6): If you reset the FS step on NR52=$80 but leave the channel's internal sweep timer at its previous value, the first sweep clock fires too soon or too late.
5. **Power-up clears NR10's negate-used latch** but you might be re-using a global FS step that is now misaligned: between the two `test_power2` calls, the first uses sync_sweep; the second uses sync_sweep + `delay_apu 1`. If your power-up DOES reset the FS step, both variants will produce the same timing (test passes); if it doesn't, the offsets differ and timing will vary by 8192 — both expectations are `-$229` so both must match.
6. **Tight `test_timing` loop cycles**: the loop is `inc de; lda NR52; and 1; jr nz,-` = 1+3+2+3 = 9 (taken) or 8 (untaken) T-cycles ÷ 4 = … actually iterations are ~11 M-cycles each. If your CPU cycle counts are off by a small constant per-instruction, the measured DE will drift by N*small per N iterations and a sub-test may pass at ~16k iterations and another fail at ~32k. Sub-test 3 (~2x as long) is more sensitive.
7. **DIV-write side effects**: writing to DIV is supposed to (a) zero the DIV register and (b) trigger an FS clock if DIV bit 12 was high (i.e. the "DIV-write APU clock" quirk). Sub-test 5's `test_power` flow does NOT write DIV — it writes NR52. But your `sync_apu` may rely on a DIV write somewhere. If your APU clocks the FS on DIV writes when it shouldn't, sync_apu may drop you in the wrong FS phase and all sub-tests measure off.

The reporter notes a "hang" on test 7 — the prime suspect is hypothesis #3 (channel never expires because length is never decremented) or hypothesis #1 (FS not running because NR52=$80 didn't kick-start it, and your APU treats DIV-bit-12 falling edge as the only FS source and DIV happens to be quiescent). To diagnose: trace the FS step counter and the length counter for ch1 between the `NR14,$C0` write at sub-test 2 line 30 and the read of NR52 in `test_timing`. The length counter must go 1 → 0 within 16384 T-cycles of the trigger.

## Notes
- `sync_apu` (in `apu.s`): writes NR24=$00, NR21=$3E (length=2), NR22=$08, NR24=$C0 (trigger), spins on NR52 bit 1 until ch2 goes off. Leaves FS aligned so the *next* length clock is ~16384 T-cycles away.
- `sync_sweep`: writes NR10=$11, NR12=$08, NR13=$FF, NR14=$83 (trigger), waits for ch1 to go off via sweep overflow. Leaves FS aligned so the *next* sweep clock is ~32768 T-cycles away.
- The `-$170`, `-$2E4` etc. constants are calibrated against the cycle count of the `test_timing` inner loop on real DMG hardware. They have a ±4 tolerance built in.
- This test depends on EVERY previous APU test working: if your length counter is wrong, 02 fails first; if your sweep is wrong, 04/05 fail first. The fact that 07 hangs while earlier tests fail with sub-codes indicates the hang is specifically about *length-clock period* or *FS phase after sync_apu*, not the trigger algorithm itself.
