# 12-wave write while on

**Suite:** dmg_sound
**Source:** [12-wave write while on.s](../../../src/roms/test/blargg-test-roms/dmg_sound/source/12-wave write while on.s)
**ROM:** [12-wave write while on.gb](../../../src/roms/test/blargg-test-roms/dmg_sound/rom_singles/12-wave write while on.gb)
**Expected CRC (DMG):** `0x3B4538A9`
**Expected CRC (CGB):** `0x2B27544E` (present in the macro call but **not compiled** — see *DMG vs CGB*)

> Part of the CH3 wave-RAM-while-on cluster: [09](09-wave read while on.md) (CPU *reads*), [10](10-wave trigger while on.md) (re-*trigger* corruption), **12** (CPU *writes*). Same skeleton, same probe offset of **+208 T-cycles after the trigger**; only the operation at the probe differs.

## Summary
Verifies the write half of the DMG wave-RAM access gate: while CH3 is enabled, a CPU write to *any* address in `$FF30`–`$FF3F` is **dropped**, unless it happens in the ~2-T-cycle window in which the channel is latching a sample byte — in which case the write is **redirected to the byte at the channel's current position**, ignoring the address that was actually written. The test writes `$F7` to `$FF30` (always the same address) 69 times, each time 2 T-cycles earlier relative to the channel's 4-T fetch grid, then disables the channel and dumps all 16 wave bytes. On "miss" iterations the ramp comes back untouched; on "hit" iterations exactly one byte contains `$F7` and its index walks through all 16 positions.

## Hardware behaviour exercised
- **Wave RAM write gate (DMG):** with CH3 active, `write($FF30..$FF3F, v)` is ignored except inside the sample-latch window, where it performs `wave_ram[current_sample_index >> 1] = v`. The written *address* is irrelevant — the whole 16-byte region behaves as one port onto the current sample byte.
- **Window width:** ~one 2 MHz APU tick ≈ **2 T-cycles**; must be **≥2 T and <4 T** for this test to alternate (fetch period here is 4 T, phase shift is 2 T per iteration).
- **CH3 frequency timer** `= (2048 − freq) × 2` T-cycles per nibble advance, one wave-byte latch per advance. `NR33 = $FE` with `freq_hi = 7` ⇒ `freq = $7FE` ⇒ **4 T per fetch**.
- **`NR33` writes do not reload the running timer**: the first fetch after the trigger uses the period from `freq = $799 + n`, and only subsequent reloads use 4 T.
- **Trigger resets the sample index to 0**; fetch *j* after the trigger reads nibble `j mod 32`, i.e. byte `(j mod 32) >> 1`.
- **`NR30 = 0` (DAC off) disables the channel and unlocks wave RAM** for the readback.
- Channel is silent via `NR32 = $00` ([:27](../../../src/roms/test/blargg-test-roms/dmg_sound/source/12-wave write while on.s)) but must keep fetching. Note this test does **not** write `NR51 = 0` (unlike 09 and 10) — level 0 alone is what mutes it.
- Length disabled (`NR34 = $87`, bit 6 clear); `NR31` never written.

## DMG vs CGB
- **This test is DMG-only by construction:** `.define REQUIRE_DMG 1` is *uncommented* at [:3](../../../src/roms/test/blargg-test-roms/dmg_sound/source/12-wave write while on.s), so `check_crc_dmg_cgb` compiles to `check_crc $3B4538A9` ([common/testing.s:145-156](../../../src/roms/test/blargg-test-roms/dmg_sound/source/common/testing.s)) and the `$2B27544E` value is never assembled. Because `REQUIRE_DMG` is defined and `REQUIRE_CGB` is not, `build_rom.s` emits **nothing** at header offset `$143` ([common/build_rom.s:45-52](../../../src/roms/test/blargg-test-roms/dmg_sound/source/common/build_rom.s)), i.e. `$00` = DMG-only cartridge.
- The source states why: *"CGB behaves erratically, so DMG-only for now"* ([:11](../../../src/roms/test/blargg-test-roms/dmg_sound/source/12-wave write while on.s)).
- The `cgb_sound` suite has **no** counterpart; its `12-wave.s` is a different, `set_test`-based test that asserts CGB's *unconditional* access (`"Current byte readable at any wave addr"`, `"Write test"` — `cgb_sound/source/12-wave.s:22,40`). In other words: on CGB, writes while on are never dropped, they always redirect to the current byte.

## Sub-tests
**None.** No `set_test` call; `test_code` stays `$FF` from `init_testing` ([common/testing.s:49](../../../src/roms/test/blargg-test-roms/dmg_sound/source/common/testing.s)). One assertion: `check_crc_dmg_cgb` at [:12](../../../src/roms/test/blargg-test-roms/dmg_sound/source/12-wave write while on.s) over all 69 × 16 = **1104 printed bytes**.

On mismatch: computed CRC is printed, then `test_failed` maps `$FF` → **A = 1** ([common/testing.s:91-93](../../../src/roms/test/blargg-test-roms/dmg_sound/source/common/testing.s)) → `"Failed"` and **`$01` at `$A000`**.

> `0x01` here means "the single CRC check failed", not "sub-test #1 failed". Per-test numeric codes (`12:fail`) only exist in the combined `dmg_sound.gb` built with the unshipped `build_multi.s`.

## Test sequence
1. [:9](../../../src/roms/test/blargg-test-roms/dmg_sound/source/12-wave write while on.s) — `loop_n_times test,69` ⇒ `for_loop test,0,68,+1`, **n = 0…68**.
2. [:16-17](../../../src/roms/test/blargg-test-roms/dmg_sound/source/12-wave write while on.s) — `B = $99 + n` (`$99`…`$DD`).
3. [:22-23](../../../src/roms/test/blargg-test-roms/dmg_sound/source/12-wave write while on.s) — `load_wave`: `NR30 = 0` (unlock) then reload the ramp `$00,$11,…,$FF` from [:48-49](../../../src/roms/test/blargg-test-roms/dmg_sound/source/12-wave write while on.s). Fresh every iteration, so no accumulation.
4. [:24-27](../../../src/roms/test/blargg-test-roms/dmg_sound/source/12-wave write while on.s) — `NR30 = $80` (DAC on), `NR32 = $00` (mute), `NR33 = B`.
5. [:28](../../../src/roms/test/blargg-test-roms/dmg_sound/source/12-wave write while on.s) — `NR34 = $87`: **trigger**, `freq = $700 | ($99+n) = 1945 + n`, initial period `= 206 − 2n` T-cycles.
6. [:31](../../../src/roms/test/blargg-test-roms/dmg_sound/source/12-wave write while on.s) — `NR33 = -2 = $FE` ⇒ 4-T period from the second reload onwards.
7. [:32](../../../src/roms/test/blargg-test-roms/dmg_sound/source/12-wave write while on.s) — `delay_clocks 168` = **42 M-cycles** exactly ([common/delay.s:105-128](../../../src/roms/test/blargg-test-roms/dmg_sound/source/common/delay.s): `push af`(4) + `ld a,13`(2) + `call delay_a_20_cycles`(33) + `pop af`(3)).
8. [:33](../../../src/roms/test/blargg-test-roms/dmg_sound/source/12-wave write while on.s) — `wreg WAVE,$F7` = `ld a,$F7` (2 M) + `ldh ($30),a` (3 M, **write in M3**): the probe.
9. [:36](../../../src/roms/test/blargg-test-roms/dmg_sound/source/12-wave write while on.s) — `NR30 = 0`: channel off, wave RAM unlocked.
10. [:37](../../../src/roms/test/blargg-test-roms/dmg_sound/source/12-wave write while on.s) — `delay 1000` = **1000 M-cycles = 4000 T-cycles** of settling before readback (10 does not do this; treat it as a safety margin).
11. [:38-42](../../../src/roms/test/blargg-test-roms/dmg_sound/source/12-wave write while on.s) — read `$FF30`…`$FF3F` via `ld a,(hl+)` (note: **normal 16-bit addressing**, not `ldh`) and `print_a` each. `bit 6,l` ends the loop at `L = $40`.
12. [:43](../../../src/roms/test/blargg-test-roms/dmg_sound/source/12-wave write while on.s) — `print_newline` (not CRC'd).

### Exact probe timing
`t = 0` at the M-cycle in which the `NR34` trigger write commits ([:28](../../../src/roms/test/blargg-test-roms/dmg_sound/source/12-wave write while on.s)):

| M-cycle | Instruction | Event |
|---|---|---|
| 0 | `ldh (NR34),a` M3 | **trigger** |
| +1,+2 | `ld a,$FE` | |
| +3,+4,+5 | `ldh (NR33),a` | `NR33 = $FE` commits at **+5 M = +20 T** |
| +6 … +47 | `delay_clocks 168` | 42 M |
| +48,+49 | `ld a,$F7` | |
| +50,+51,+52 | `ldh (WAVE),a` | **write commits at +52 M = +208 T** |

Same +208 T as 09's read and 10's re-trigger (44 M + 3 M = 42 M + 5 M = 47 M after the `NR33` write). With the first fetch at `trigger + (206 − 2n) + Δ`:

```
d(n) = 208 − (206 − 2n) − Δ = 2n + 2 − Δ
hit  ⇔  d(n) ≥ 0  and  d(n) mod 4 == 0
j(n) = d(n)/4 + 1
target byte index = (j mod 32) >> 1      ← this byte becomes $F7
```

Δ is the wave trigger→first-fetch delay (SameBoy ≈ +4 T, other models +6 T). `Δ mod 4` decides whether even or odd `n` hit; a 2-T error inverts the pattern.

## Expected output
69 lines of `16 × "XX "` + newline, then `"Passed"`.

Derived expectation with Δ = 6 T (even `n` hit, first hit at `n = 2`) — **derived from the model, not a hardware capture**:

| iterations (n) | j | target index | line |
|---|---|---|---|
| 0, 1, all odd n | — | — (write dropped) | `00 11 22 33 44 55 66 77 88 99 AA BB CC DD EE FF` |
| 2 | 1 | 0 | `F7 11 22 33 44 55 66 77 88 99 AA BB CC DD EE FF` |
| 4, 6 | 2,3 | 1 | `00 F7 22 33 …` |
| 8, 10 | 4,5 | 2 | `00 11 F7 33 …` |
| 12, 14 | 6,7 | 3 | `00 11 22 F7 …` |
| 16, 18 | 8,9 | 4 | `… 44→F7 …` |
| 20, 22 | 10,11 | 5 | |
| 24, 26 | 12,13 | 6 | |
| 28, 30 | 14,15 | 7 | |
| 32, 34 | 16,17 | 8 | |
| 36, 38 | 18,19 | 9 | |
| 40, 42 | 20,21 | 10 | |
| 44, 46 | 22,23 | 11 | |
| 48, 50 | 24,25 | 12 | |
| 52, 54 | 26,27 | 13 | |
| 56, 58 | 28,29 | 14 | |
| 60, 62 | 30,31 | 15 | `00 11 … EE F7` |
| 64, 66 | 32,33 | 0 (position wrapped) | `F7 11 22 33 …` |
| 68 | 34 | 1 | `00 F7 22 33 …` |

Structural checks independent of Δ:
- 35 pristine lines, 34 lines containing exactly one `$F7`.
- The `$F7` index walks 0,1,1,2,2,…,15,15 then wraps to 0,0,1 — every one of the 16 positions is exercised.
- No line ever has `$F7` at more than one index, and the written value never lands anywhere except the current-position byte.

The printed text is mirrored to the cartridge-RAM string starting at `$A004` ([common/shell.s:194-225](../../../src/roms/test/blargg-test-roms/dmg_sound/source/common/shell.s)) — dump it from the headless harness and diff it to find the first divergent line.

## Emulator pitfalls
- **No write gate at all.** This repo's `Ch3::write_wave_ram` ([src/apu/ch3.rs:100-107](../../../src/apu/ch3.rs)) redirects to `wave_ram[wave_pos/2]` whenever the channel is active, with no window check — that is **CGB** behaviour. Result: all 69 iterations write `$F7` (never dropped), so the 35 lines that should be pristine are wrong ⇒ CRC failure. You need the same `wave_form_just_read` flag used by 09 and 10; outside the window the write must be **silently discarded**, not applied at the addressed byte.
- **Honouring the written address.** The write targets `$FF30` every time. If you apply it at `addr − $FF30` instead of `wave_pos/2`, index 0 gets `$F7` on every hit and indices 1–15 are never touched — the alternation would still be there but the walking-index pattern is gone.
- **Coarse APU stepping.** The write commits in the 3rd M-cycle of `ldh (n8),a` (4 T before the instruction retires). If the APU is advanced by whole instructions, the effective probe time is off by up to ±12 T = ±3 slots on the 4-T fetch grid. Sub-instruction APU catch-up is required for this cluster.
- **Reloading the frequency timer on the `NR33` write** at +20 T pins the first fetch at a fixed offset for all iterations, so the phase never sweeps and every line is identical.
- **Freezing the channel when muted.** `NR32 = 0` (output level mute) must not stop the frequency timer / sample fetch; if it does, `wave_pos` stays 0 and every hit writes to index 0.
- **Wrong Δ.** Off by 2 T inverts which iterations hit; off by 4 T shifts the walking index by one fetch.
- **Position wrap** is over 32 nibbles, not 16 bytes — the last three iterations depend on it.
- **Wave RAM must be readable normally once `NR30 = 0`.** The readback uses plain `ld a,(hl+)` at `$FF30+`, so it also exercises the non-`ldh` path through your MMU into the APU wave region — make sure `0xFF30..=0xFF3F` is routed identically for both addressing modes.

## Notes
- CRC-32 (reflected, poly `$EDB88320`, init `$FFFFFFFF`, final complement) is computed over the 1104 raw byte values only; spaces and newlines use `print_char_nocrc` and are excluded ([common/numbers.s:16-23](../../../src/roms/test/blargg-test-roms/dmg_sound/source/common/numbers.s), [common/crc.s](../../../src/roms/test/blargg-test-roms/dmg_sound/source/common/crc.s)).
- Since 09 (read gate), 10 (trigger corruption gate) and 12 (write gate) all hang off the *same* `wave_form_just_read` flag and the *same* +208 T probe, implementing the flag correctly should flip all three at once. Fix order: (1) add the latch flag with a 2-T lifetime, (2) gate reads → `$FF`, (3) gate writes → dropped, (4) gate the trigger corruption and add the 4-byte block copy.
- The only genuinely uncertain parameter above is Δ. The +208 T probe, the 4-T fetch period, the 2-T-per-iteration sweep and the walking `$F7` index all follow directly from the assembly.
- The `delay 1000` at [:37](../../../src/roms/test/blargg-test-roms/dmg_sound/source/12-wave write while on.s) has no analogue in test 10, which reads wave RAM immediately after `NR30 = 0`. If your emulator passes 12 but fails 10 on the readback bytes, suspect a lingering post-DAC-off lock.
