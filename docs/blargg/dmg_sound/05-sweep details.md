# 05-sweep details

**Suite:** dmg_sound
**Source:** [05-sweep details.s](../../../src/roms/test/blargg-test-roms/dmg_sound/source/05-sweep details.s)
**ROM:** [05-sweep details.gb](../../../src/roms/test/blargg-test-roms/dmg_sound/rom_singles/05-sweep details.gb)

## Summary
Exercises *internal* sweep-unit behaviours that aren't covered by 04: the period-0-treated-as-8 timer reload, the private shadow copy of the frequency taken at trigger, the "exiting negate mode after using it" disable rule (a known DMG sweep quirk: once a calc has run in negate mode, leaving negate mode forces an immediate disable), the two's-complement encoding of the negate calc, and the rule that NR13/NR14 writes don't synchronise to the sweep — the channel's frequency is only refreshed when the sweep timer reloads. Each sub-test sets up a precise NR10/NR13/NR14 sequence and uses `delay_apu N` to drive the FS forward by exactly N steps, then asserts that the channel is either silenced (`should_be_off`) or 1 length-tick from being silenced (`should_be_almost_off`).

## Hardware behaviour exercised
- **Period 0 → reload with 8** (test 2): NR10 period field = 0 must reload the internal sweep timer with 8, not 0. So writing NR10=$x1 then immediately changing to period=$1 once means the *next* sweep clock fires 8 FS steps later, not 1.
- **Shadow frequency** (test 3): The sweep unit makes a private copy of the channel frequency on trigger. Subsequent writes to NR13/NR14 (without re-triggering) change the channel's output frequency for the audio path, but the sweep unit keeps calculating using the *shadow* value. So the sweep can still overflow and disable the channel even if the user has changed NR13 to a much smaller value.
- **Exit-from-negate disables** (tests 4, 5, 6): Once a calc has been executed with the negate bit set, switching the negate bit back to add mode forces an immediate disable. Conditions:
  - The calc must actually have *run* (negate bit was set when a calc happened). Just setting negate without a calc is fine.
  - Test 6 explores edge cases: enabling negate, immediately disabling without a calc → no disable; using negate via trigger then leaving → disable.
- **Two's complement negate** (tests 7, 8): The negate calc is `freq - (freq >> shift)`, computed as `freq + (~(freq >> shift) + 1)` in 12-bit arithmetic. Boundary behaviour at `freq == 0` and `freq == 1` matters — test 8 picks values where two's complement should overflow if implemented wrong.
- **Sweep only updates on timer reload** (test 9): The channel's audible frequency register is only written from the shadow at the moment the sweep timer expires and the calc completes — *not* every NR13/NR14 user write, and not on every FS step.

## DMG vs CGB
No `check_crc_dmg_cgb` — failure is the numeric sub-code. Test 7 explicitly uses `delay 2048` to "avoid extra length clocking on CGB-02", indicating the same `.s` is shared between DMG and CGB ROMs and that CGB-02 has a sweep+length interaction not present on DMG; on DMG-CPU-B the `delay 2048` is harmless padding. Expected behaviour on DMG and CGB is otherwise identical.

## Sub-tests
| ID | Description | Key write | What it proves |
|----|-------------|-----------|---------------|
| 2 | Timer treats period 0 as 8 | NR10=$11 (per=1), then NR10=$01 (per=0,sh=1), then NR10=$11 again after `delay_apu 3` | After period=0, the timer must reload with 8 — so during 3 FS steps no clock fires; only after switching back to period=1 does the channel die from the next sweep clock with shift=1 calcs. The `delay_apu $11` (17 FS steps) then `almost_off` window asserts the right timing. |
| 3 | Makes private copy of frequency on trigger | NR10=$12 (per=1,sh=2), NR13=$04 NR14=$80 (trigger, freq=$004), then NR13=$00 (user changes freq to $000) | Sweep still uses shadow=$004 and overflows in `delay_apu $39` (57 FS steps). If you don't shadow, the calc uses freq=0 and never overflows. |
| 4 | Exit-from-negate after calc disables | NR10=$09 (per=0,sh=1,neg=1) at trigger → trigger calc runs in neg mode (shift>0). `delay_apu 2`. NR10=$10 (clear negate) | Immediate disable: `should_be_off`. |
| 5 | Exit-from-negate after sweep clock disables | NR10=$10 (per=1,sh=0,neg=0) trigger → no calc (shift=0). `delay_apu 2` (sweep clock fires, calc with neg=0 = no negate-mode-used yet). NR10=$18 (neg=1, per=1, sh=0) → no calc (shift=0). `delay_apu 2` (sweep clock fires, but shift=0 means no real calc still... wait — the bug here is that *any* sweep clock with neg=1 set "uses negate"). NR10=$10 (clear neg) | After a sweep clock saw neg=1, clearing it disables. |
| 6 | Exiting negate mode other ways doesn't disable | Sequence of NR10 toggles around triggers and FS steps; many cases | Verifies that just *seeing* neg=1 in the NR10 register without a calc actually using it does NOT arm the disable. Specifically: setting neg=1 then clearing it before a clock fires → no disable. |
| 7 | Subtract mode uses two's complement | NR10=$1C (per=1,sh=4,neg=1), NR13=$B0 NR14=$85 (trigger freq=$5B0) → trigger calc: $5B0 - $5B0>>4 = $5B0 - $5B = $555 OK. Then NR10=$01 NR14=$C5 (re-trigger pos with shift=1, freq=$5B0): $5B0 + $2D8 = $888 > $7FF, disables next clock | Tests that the negate calc subtracts correctly (no off-by-one or wrap). |
| 8 | Subtract upper bound | Same as 7 but NR13=$B1 (freq=$5B1) | $5B1+$2D8 = $889 > $7FF; channel disables sooner. Boundary value. |
| 9 | Update channel frequency only when period reloaded | NR10=$74 (per=7,sh=4), trigger at freq=$406, wait 14 FS steps (one sweep clock just fired), rewrite NR13=$06 (no effect on shadow), wait 13 more FS, then NR14=$85 just before next reload | The frequency only updates when the timer reloads — if your emulator writes the new frequency every FS step (or on every NR13 write), the result diverges. |

## Test sequence
1. [05-sweep details.s:6-11](../../../src/roms/test/blargg-test-roms/dmg_sound/source/05-sweep details.s) — `begin` (note: `NR11,-$20` here = $E0, length-load=$20=32 → counter=32; vs 04's 33).
2. Test 2 ([line 25-35](../../../src/roms/test/blargg-test-roms/dmg_sound/source/05-sweep details.s)) — exercises period-0 → 8 reload.
3. Test 3 ([line 37-44](../../../src/roms/test/blargg-test-roms/dmg_sound/source/05-sweep details.s)) — shadow frequency.
4. Test 4 ([line 46-53](../../../src/roms/test/blargg-test-roms/dmg_sound/source/05-sweep details.s)) — calc-in-negate then exit.
5. Test 5 ([line 55-64](../../../src/roms/test/blargg-test-roms/dmg_sound/source/05-sweep details.s)) — sweep-clock-in-negate then exit.
6. Test 6 ([line 66-85](../../../src/roms/test/blargg-test-roms/dmg_sound/source/05-sweep details.s)) — multi-stage NR10 sequence verifying the negate-disable rule's prerequisites.
7. Test 7 ([line 87-97](../../../src/roms/test/blargg-test-roms/dmg_sound/source/05-sweep details.s)) — two's complement basic.
8. Test 8 ([line 99-107](../../../src/roms/test/blargg-test-roms/dmg_sound/source/05-sweep details.s)) — two's complement boundary.
9. Test 9 ([line 109-119](../../../src/roms/test/blargg-test-roms/dmg_sound/source/05-sweep details.s)) — update-only-on-reload.

## Expected output
Pass prints "Passed". On fail, the `set_test` label string ("Timer treats period 0 as 8", "Makes private copy of frequency on trigger", etc.) is printed, then "Failed #N" where N matches the sub-test number.

## Emulator pitfalls
- **Period-0 timer reload (sub-test 2 — reporter saw 05:6, but 05:2 is also a common failure)**: when NR10 period field is 0, the *running* sweep timer (already loaded from a previous non-zero period) is allowed to expire, but on reload it must take the value 8, not 0. Common bug: reload with the period field as-is (=0), which makes the timer immediately fire again, in a tight loop, on every FS step. Effect: every period-0 write triggers extra calcs.
- **Negate-mode disable arming (sub-test 6 — most likely your "sub-code 6" failure)**: The disable is armed by *a calc that was performed while the negate bit was set*, not by setting the negate bit in NR10. If your emulator arms on the NR10 write (or fails to arm on the calc), test 6 fails. The disambiguation is subtle:
  - `NR10=$1F` (neg=1) but no calc happens because length doesn't trigger one → no disable when neg is later cleared.
  - `NR10=$18` (per=1, neg=1, sh=0) → sweep clock fires but shift=0 means no real frequency calc happens, BUT the calc *step* in the sweep clock still runs (it just doesn't write the result). The disable should arm here. If your emulator skips the calc entirely when shift=0, test 5 fails.
  - The correct mental model: the sweep clock's "calc step" runs whenever the timer expires AND sweep is internally enabled, regardless of shift. The *write* step is gated on shift>0. The negate-disable is armed by the *calc* step seeing neg=1, not the write step.
- **Shadow frequency (sub-test 3)**: must be captured on trigger only. If on each FS step you re-read NR13/14, the test 3 expectation breaks — user wrote NR13=$00 mid-test and the channel should still overflow.
- **Two's complement (tests 7, 8)**: implementation must do `freq - (freq >> shift)` in 12-bit arithmetic. Subtle bug: doing it in 16 bits with sign extension is fine; doing it in unsigned 11 bits with `if result < 0 then disable` would also produce the right channel disable but might mis-handle edge cases at freq=0.
- **Update gating (test 9)**: The shadow update writes to the *channel's* frequency register only when (a) the sweep timer reloads AND (b) shift > 0 AND (c) the calc didn't overflow. The user's NR13/NR14 writes between FS clocks must NOT touch the shadow.
- **NR10 writes that flip the negate bit mid-period**: the internal "negate used" latch must persist across NR10 writes that touch only period/shift, but should NOT be set by an NR10 write that merely sets the negate bit without a calc occurring.

## Notes
- Test 6 is the most intricate sub-test in the entire dmg_sound suite. The "Emulator pitfalls" section's first bullet about negate-disable arming is your strongest hypothesis for a "05:6" failure.
- A "05:fail with hang" outcome usually means the test_failed handler ran but the failure-print code looped because the APU is in an unexpected state — the original failure is whichever `set_test` was active last.
- `delay_apu 1` = 1 FS step = 8192 T-cycles. `sync_sweep` aligns FS so that step 2 (the next sweep clock) is `delay_apu 1` away.
