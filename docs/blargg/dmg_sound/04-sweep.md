# 04-sweep

**Suite:** dmg_sound
**Source:** [04-sweep.s](../../../src/roms/test/blargg-test-roms/dmg_sound/source/04-sweep.s)
**ROM:** [04-sweep.gb](../../../src/roms/test/blargg-test-roms/dmg_sound/rom_singles/04-sweep.gb)

## Summary
Exercises the Channel 1 frequency sweep unit (NR10) and proves that the *overflow check* and *frequency-update* operations happen at the right points: on trigger (when shift > 0), on every sweep clock (when period > 0), with the correct interaction between `shift==0` and `period==0`, and with the right sequencing for "enables on trigger" vs "disables on trigger". Every sub-test is built so that the channel must turn off because of a sweep overflow (or stay on because no overflow happened) — never because the length counter ran out — within a tight `delay_apu` window. If the sweep logic is off by one calc, fires too early, fires when it shouldn't, or skips the update, the channel is either silenced too soon (`should_be_off` fails with `nz` → "channel still on") or stays on too long (`should_be_almost_off` fails with `z` → "channel already off").

## Hardware behaviour exercised
- NR10 layout: bit6:4 = sweep period, bit3 = negate, bit2:0 = shift.
- Trigger (NR14 bit7) with `shift > 0` performs the initial *calc* and overflow check immediately, even when sweep period > 0 (the timer has not ticked yet).
- Trigger with `shift == 0` does **not** perform the initial calc; channel stays enabled regardless of what `(freq >> 0)` would have produced.
- A sweep clock with `period == 0` is a no-op; the internal sweep timer is reloaded with 8 when the NR10 period field is 0 (this `period 0 → 8` quirk is tested in detail by 05-sweep details, but 04 implicitly assumes the inert behaviour).
- The sweep unit performs *two* calcs per period when `shift > 0`: a first one that *also* writes back the new frequency (the "update"), and a second one whose result is **only used for the overflow check** and is *not* written. Test 5 ("After updating frequency, calculates a second time") depends on this.
- Overflow check: if `(freq ± freq>>shift)` produces a value > $7FF, the channel is disabled. This must happen for **both** calcs in a period.
- After the first update, subsequent overflow checks use the *updated* shadow frequency, not the original.
- The internal sweep enable flag (set on trigger to `period != 0 OR shift != 0`) gates whether the timer runs at all.
- "Trigger always re-enables if DAC on" — even when the trigger itself immediately disables via overflow, the channel must briefly be enabled, then disabled (test 6 wants `should_be_off` instantly: i.e. the disable wins the same M-cycle).

## DMG vs CGB
No `check_crc_dmg_cgb` in this file; the test relies on `set_test ##` + `test_failed` to print a numeric sub-code rather than a CRC. The dmg_sound ROM expects identical behaviour on DMG-CPU-B / DMG-CPU-C. The CGB-02 variant has an extra-length-clock corner case that this test largely avoids by using `NR14=$40` (length-disabled write before trigger).

## Sub-tests
The `begin` routine at [04-sweep.s:6](../../../src/roms/test/blargg-test-roms/dmg_sound/source/04-sweep.s) calls `sync_sweep` (aligns FS so a sweep clock just fired and the FS is at a known phase), then writes `NR14=$40` (clear trigger, disable length), `NR11=$DF` (length-load = 31 → length counter = 33), `NR12=$08` (DAC on, envelope 0, channel silent). Each sub-test then writes the specific NR10/13/14 under test.

| ID | Description | Setup | Expectation |
|----|-------------|-------|-------------|
| 2  | If shift>0, calculates on trigger | NR10=$01 or $11, freq=$7FF, trigger+len | `should_be_off` immediately — initial calc overflows |
| 3  | If shift=0, doesn't calculate on trigger | NR10=$10 (period=1,shift=0), freq=$7FF | After `delay_apu 1`: `should_be_almost_off` — still on, period 1 clock kills it next |
| 4  | If period=0, doesn't calculate | NR10=$00, freq=$7FF, wait `delay_apu $20` | Still on (until length expires); proves period-0 timer never fires the calc |
| 5  | After update, calculates a second time | NR10=$11 (per=1,sh=1), freq=$1FF, wait 1 | After update freq=$3FE; second calc=$3FE+$1FF=$5FD ≤$7FF; channel stays on. But on next clock, $5FD+$2FE>$7FF disables — used to drive `should_be_almost_off` |
| 6  | If calculation>$7FF, disables | NR10=$02 (per=0,sh=2), freq=$667, trigger | $667 + $667>>2 = $7E8 (ok), but with NR10=$02 period=0 means sweep is disabled at the timer level — BUT shift>0 still does the trigger calc; that calc is `$667 + $199 = $800 > $7FF` → `should_be_off` |
| 7  | If calculation<=$7FF, doesn't disable | NR10=$01, freq=$555+ shifted, sweep keeps incrementing slowly | Channel stays on until length expires |
| 8  | shift=0 and period>0, trigger enables | NR10=$10 then later set NR10=$11 | Trigger does not calc (shift=0); channel stays on; later NR10=$11 makes shift>0 and next sweep clock kills it |
| 9  | shift>0 and period=0, trigger enables | NR10=$01 (per=0,sh=1); then NR10=$11 after delay | Initial calc with per=0,sh=1: per=0 → sweep timer wouldn't run, but trigger still calcs; freq=$7FF + $7FF>>1=$BFE>$7FF → would disable. (See "Emulator pitfalls".) |
| 10 | shift=0 and period=0, trigger disables | NR10=$08 (per=0,sh=0,neg=1); then NR10=$11 | With per=0 AND sh=0, sweep enable flag is off; trigger doesn't calc. Stays on via length |
| 11 | shift=0 doesn't update | NR10=$10 (per=1,sh=0), trigger | Sweep clocks fire but `shift==0` means no frequency write; channel stays on |
| 12 | period=0 doesn't update | NR10=$01 (per=0,sh=1) | Trigger calc happens (and may disable if freq big enough); but in this case freq=$500: $500 + $500>>1 = $780 ≤ $7FF, no disable. No sweep timer running so no further writes |

## Test sequence
1. [04-sweep.s:6-11](../../../src/roms/test/blargg-test-roms/dmg_sound/source/04-sweep.s) — `begin` aligns FS, clears trigger/length, loads a long length-counter (33), sets DAC alive but silent envelope.
2. Each sub-test calls `begin`, writes its specific NR10/13/14, and immediately verifies via `should_be_off` or `should_be_almost_off`.
3. `should_be_off` (line 18) — read NR52, mask bit 0 (ch1), branch to `test_failed` if non-zero.
4. `should_be_almost_off` (line 13) — read NR52, mask bit 0, branch to `test_failed` if zero (channel should still be active), then `delay_apu 1` (one FS step = 8192 T-cycles ≈ 2 ms), then `should_be_off`.

## Expected output
There is no `check_crc` in this test; failure prints `set_test`'s string label and the numeric code (2…12). Pass prints "Passed" via `tests_passed`. The text printed during pass is just the final "Passed" line.

## Emulator pitfalls
- **Trigger overflow check gated on shift>0**: If your trigger always runs the calc (including when `shift==0`), test 3 fails immediately — the channel will be silenced too aggressively. Conversely, if the trigger never calcs even with `shift>0`, test 2 fails (channel stays on when it should die).
- **`shift==0` skips the *update* but not the *trigger calc***: The internal logic is "trigger calc runs iff shift>0", AND "sweep-clock calc runs every period iff sweep enable flag is on". The frequency write-back only happens on the first calc of a period when `shift>0` (the second calc is overflow-only). Common bug: writing back from the second calc, or writing back when `shift==0`.
- **Sweep enable flag**: the internal "sweep on" latch is computed at trigger as `period != 0 OR shift != 0`. If you compute it as `period != 0 AND shift != 0`, tests 8 and 9 will misbehave (channel either enabled when it should be inert, or disabled when it should remain on).
- **Period 0 timer reload to 8**: this is needed for 05 but if your timer reloads to 0 instead of 8, the sweep clock fires every step at period=0, which will disable test 4 (channel dies from extra calcs instead of staying on for length).
- **Negate-flag latch and exit-from-negate disable**: Cleared in test 6; touched only lightly in 04 (test 10 uses neg=1 with shift=0/period=0). If your implementation forces a calc when neg bit is set even with shift=0/period=0, test 10 fails.
- **`should_be_off` immediately after trigger**: requires that the trigger overflow path silences the channel *in the same M-cycle as the NR14 write*. If your APU defers trigger side-effects to the next tick, the NR52 read 5 cycles later still shows ch1=1 and test 2 fails.
- **Order of operations relative to length clock**: `begin` writes `NR14=$40` then later `NR14=$Cx`. The 0→1 length-enable transition can extra-clock the length counter during the first half of a length period. `sync_sweep` happens to leave FS in a known phase to dodge that, but if the FS phase is wrong (e.g. emulator's `power_off_reset` doesn't actually zero step on a DIV write), the extra clock fires at the wrong time and the length-counter walks become misaligned, killing all "almost_off" checks.

## Notes
- `delay_apu N` (defined in `apu.s` — not in tree but referenced from the cpu_instrs copy) delays N frame-sequencer cycles ≈ N * 8192 T-cycles.
- The sub-code reported on failure is the `set_test` number, displayed as e.g. `04:6` on screen.
- The reporter for this repo flagged a "sub-code 4" failure on test 4: that points at "If period=0, doesn't calculate" — the most likely culprit is the period-0 timer reload not being 8 (so the sweep clock erroneously fires under period=0 and the calc disables the channel before the `delay_apu $20` window completes).
