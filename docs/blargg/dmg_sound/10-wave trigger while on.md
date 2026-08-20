# 10-wave trigger while on

**Suite:** dmg_sound
**Source:** [10-wave trigger while on.s](../../../src/roms/test/blargg-test-roms/dmg_sound/source/10-wave trigger while on.s)
**ROM:** [10-wave trigger while on.gb](../../../src/roms/test/blargg-test-roms/dmg_sound/rom_singles/10-wave trigger while on.gb)
**Expected CRC (DMG):** `0x533D6D4D`
**Expected CRC (CGB):** `0x8130733A`

> Part of the CH3 wave-RAM-while-on cluster: [09](09-wave read while on.md) (CPU *reads*), **10** (re-*trigger* corruption), [12](12-wave write while on.md) (CPU *writes*). Same skeleton, same probe offset of **+208 T-cycles after the first trigger**; only the operation at the probe differs.

## Summary
Tests the DMG **wave-RAM corruption on re-trigger** quirk: if CH3 is re-triggered at the exact moment it is latching a sample byte, the first bytes of wave RAM are clobbered. If the byte being read is one of `$FF30`–`$FF33`, only `$FF30` is overwritten with that byte; if it is one of the other twelve, the **aligned 4-byte group** containing it (`$FF34-37`, `$FF38-3B`, or `$FF3C-3F`) is copied over `$FF30`–`$FF33`. If the re-trigger does *not* land in the access window, wave RAM is left completely untouched. The 69 iterations sweep the re-trigger 2 T-cycles earlier each time relative to the channel's 4-T fetch grid, so the re-trigger alternately lands inside and outside the window, and the position at which it lands walks through all 16 wave bytes — exercising the single-byte case and all three 4-byte-block cases. After each re-trigger the channel is switched off and all 16 wave bytes are printed.

## Hardware behaviour exercised
- **DMG wave RAM corruption on trigger** (Pan Docs, *Obscure behaviour*): triggering CH3 while it reads a sample byte alters the first four bytes of wave RAM.
  - byte offset `o = current_sample_index >> 1`
  - `o < 4`  ⇒ `wave[0] = wave[o]` (single byte)
  - `o ≥ 4`  ⇒ `wave[0..3] = wave[(o & ~3) .. (o & ~3)+3]` (aligned 4-byte block copy)
- **The corruption is gated on the same ~2-T-cycle access window as reads/writes** (`wave_form_just_read`). Outside it, a re-trigger is harmless. This is the single most commonly mis-implemented part: emulators often corrupt on *every* re-trigger.
- **The corruption happens before the trigger's own state reset** — it uses the position the channel had at that instant, and only then does the trigger zero the sample index and reload the period.
- **CH3 frequency timer** `= (2048 − freq) × 2` T-cycles per nibble advance; `NR33 = $FE`, `freq_hi = 7` ⇒ `freq = $7FE` ⇒ **4 T-cycles per fetch**.
- **`NR33` writes do not reload the running timer** — the first fetch after the initial trigger uses the period from `freq = $799 + n`.
- **Turning the DAC off (`NR30 = 0`) disables the channel and unlocks wave RAM immediately**, which is how the test reads the 16 bytes back at full speed.
- Channel is inaudible throughout (`NR51 = 0` at [:9](../../../src/roms/test/blargg-test-roms/dmg_sound/source/10-wave trigger while on.s), `NR32 = $00` at [:24](../../../src/roms/test/blargg-test-roms/dmg_sound/source/10-wave trigger while on.s)) but must keep fetching.
- Length is disabled (`NR34 = $87`, bit 6 clear), `NR31` untouched.

## DMG vs CGB
- `REQUIRE_DMG`/`REQUIRE_CGB` are both commented out in-source ([:3-4](../../../src/roms/test/blargg-test-roms/dmg_sound/source/10-wave trigger while on.s)); the define comes from the wla command line. The `dmg_sound` build checks **`$533D6D4D`**; the byte-identical `cgb_sound/source/10-wave trigger while on.s` built with `-DREQUIRE_CGB` checks `$8130733A`.
- **CGB does not have the corruption quirk.** On CGB the expected output should be 69 identical unmodified lines (`00 11 22 … FF`), which is why the CRCs differ.

## Sub-tests
**None.** No `set_test` call anywhere, so `test_code` remains `$FF` from `init_testing` ([common/testing.s:49](../../../src/roms/test/blargg-test-roms/dmg_sound/source/common/testing.s)). One assertion only: `check_crc_dmg_cgb` at [:11](../../../src/roms/test/blargg-test-roms/dmg_sound/source/10-wave trigger while on.s) covering all 69 × 16 = **1104 printed bytes**.

Failure path: `check_crc_` prints the computed CRC → `test_failed` → `test_code == $FF` maps to **A = 1** ([common/testing.s:91-93](../../../src/roms/test/blargg-test-roms/dmg_sound/source/common/testing.s)) → `"Failed"` on screen and **`$01` at `$A000`**.

> Result code `0x01` is *not* "sub-test 1 failed" — it is the generic "the CRC check failed" code. Numbered `10:fail` codes exist only in the combined `dmg_sound.gb` built via the unshipped `build_multi.s`.

## Test sequence
1. [:9](../../../src/roms/test/blargg-test-roms/dmg_sound/source/10-wave trigger while on.s) — `NR51 = 0`, mute.
2. [:10](../../../src/roms/test/blargg-test-roms/dmg_sound/source/10-wave trigger while on.s) — `loop_n_times test,69` ⇒ `for_loop test,0,68,+1`, **n = 0…68**.
3. [:15-16](../../../src/roms/test/blargg-test-roms/dmg_sound/source/10-wave trigger while on.s) — `B = $99 + n`.
4. [:21-22](../../../src/roms/test/blargg-test-roms/dmg_sound/source/10-wave trigger while on.s) — `load_wave` sets `NR30 = 0` (unlock) and reloads the ramp `$00,$11,…,$FF` ([:48-49](../../../src/roms/test/blargg-test-roms/dmg_sound/source/10-wave trigger while on.s)). **Wave RAM is refreshed every iteration**, so corruption never accumulates.
5. [:23-26](../../../src/roms/test/blargg-test-roms/dmg_sound/source/10-wave trigger while on.s) — `NR30 = $80`, `NR32 = $00`, `NR33 = B`.
6. [:27](../../../src/roms/test/blargg-test-roms/dmg_sound/source/10-wave trigger while on.s) — `NR34 = $87`: **first trigger**, `freq = 1945 + n`, initial period `206 − 2n` T-cycles.
7. [:30](../../../src/roms/test/blargg-test-roms/dmg_sound/source/10-wave trigger while on.s) — `NR33 = $FE` ⇒ 4-T fetch period from the second fetch onwards.
8. [:31](../../../src/roms/test/blargg-test-roms/dmg_sound/source/10-wave trigger while on.s) — `delay_clocks 168` = **42 M-cycles** exactly (`push af`(4) + `ld a,13`(2) + `call delay_a_20_cycles`(33) + `pop af`(3); [common/delay.s:105-128](../../../src/roms/test/blargg-test-roms/dmg_sound/source/common/delay.s)).
9. [:32](../../../src/roms/test/blargg-test-roms/dmg_sound/source/10-wave trigger while on.s) — `NR34 = $87` again: **the re-trigger under test**.
10. [:33](../../../src/roms/test/blargg-test-roms/dmg_sound/source/10-wave trigger while on.s) — `delay_clocks 40` = 10 M-cycles (`delay_short_ 10` = `call delay_unrolled_+0`: `call`(6) + `ret`(4)); lets the re-trigger settle before power-down.
11. [:36](../../../src/roms/test/blargg-test-roms/dmg_sound/source/10-wave trigger while on.s) — `NR30 = 0`: DAC off ⇒ channel off ⇒ wave RAM unlocked.
12. [:37-42](../../../src/roms/test/blargg-test-roms/dmg_sound/source/10-wave trigger while on.s) — read `$FF30`…`$FF3F` via `ld a,($FF00+c)` and `print_a` each (2 hex digits + space; only the raw byte enters the CRC). `bit 6,c` terminates when `C` reaches `$40`.
13. [:43](../../../src/roms/test/blargg-test-roms/dmg_sound/source/10-wave trigger while on.s) — `print_newline` (**not** CRC'd; [common/printing.s:36-41](../../../src/roms/test/blargg-test-roms/dmg_sound/source/common/printing.s)).

### Exact probe timing
`t = 0` at the M-cycle in which the **first** `NR34` write commits ([:27](../../../src/roms/test/blargg-test-roms/dmg_sound/source/10-wave trigger while on.s)):

| M-cycle | Instruction | Event |
|---|---|---|
| 0 | `ldh (NR34),a` M3 | **trigger #1** |
| +1,+2 | `ld a,$FE` | |
| +3,+4,+5 | `ldh (NR33),a` | `NR33 = $FE` commits at **+5 M = +20 T** |
| +6 … +47 | `delay_clocks 168` | 42 M |
| +48,+49 | `ld a,$87` | |
| +50,+51,+52 | `ldh (NR34),a` | **re-trigger commits at +52 M = +208 T** |

Identical to 09's read probe (09 uses `delay_clocks 176` + a 3 M-cycle `ldh a,(n)`; 10 uses `delay_clocks 168` + a 5 M-cycle `ld a,n` / `ldh (n),a` pair — 44+3 = 42+5 = 47 M). So all three ROMs in this cluster probe at **trigger + 208 T**.

First fetch at `trigger + (206 − 2n) + Δ`, where Δ is the wave trigger delay (SameBoy ≈ +4 T, others +6 T). Hence:

```
d(n) = 208 − (206 − 2n) − Δ = 2n + 2 − Δ
hit  ⇔  d(n) ≥ 0  and  d(n) mod 4 == 0
j(n) = d(n)/4 + 1            (which fetch is coincident with the re-trigger)
pos  = j mod 32              (nibble index at that moment)
o    = pos >> 1              (byte offset that gets copied)
```

`Δ mod 4` chooses whether even or odd `n` hit; a 2-T error inverts the pattern.

## Expected output
69 lines, each `16 × "XX "` then a newline; then `"Passed"`.

Derived expectation with Δ = 6 T (even `n` hit, first hit at `n = 2`) — **derived from the model, not a hardware capture**:

| iterations (n) | j | pos | o | resulting wave RAM |
|---|---|---|---|---|
| 0, 1, all odd n | — | — | — | `00 11 22 33 44 55 66 77 88 99 AA BB CC DD EE FF` (untouched — re-trigger missed the window) |
| 2 | 1 | 1 | 0 | `wave[0]=wave[0]` — untouched (self-copy) |
| 4, 6 | 2,3 | 2,3 | 1 | `11 11 22 33 44 …` |
| 8, 10 | 4,5 | 4,5 | 2 | `22 11 22 33 44 …` |
| 12, 14 | 6,7 | 6,7 | 3 | `33 11 22 33 44 …` |
| 16 … 30 | 8…15 | 8…15 | 4-7 | `44 55 66 77` `44 55 66 77` `88 99 AA BB` `CC DD EE FF` |
| 32 … 46 | 16…23 | 16…23 | 8-11 | `88 99 AA BB` `44 55 66 77` `88 99 AA BB` `CC DD EE FF` |
| 48 … 62 | 24…31 | 24…31 | 12-15 | `CC DD EE FF` `44 55 66 77` `88 99 AA BB` `CC DD EE FF` |
| 64, 66 | 32,33 | 0,1 | 0 | untouched (position wrapped; self-copy) |
| 68 | 34 | 2 | 1 | `11 11 22 33 44 …` |

Key structural checks independent of Δ:
- 35 lines are the pristine ramp; 34 lines are "hit" lines, of which 4 (`o = 0`) are indistinguishable from a miss because the copy is a self-copy.
- Exactly three distinct 4-byte-block results appear, and they persist for 8 consecutive hit-iterations each.
- Bytes 4–15 are **never** modified — only `$FF30`–`$FF33` can change.

The printed text is mirrored to the cartridge-RAM string at `$A004` ([common/shell.s:194-225](../../../src/roms/test/blargg-test-roms/dmg_sound/source/common/shell.s)); dump it to locate the first bad line instead of bisecting via the CRC.

## Emulator pitfalls
- **Corrupting on every re-trigger.** This repo does exactly that: [src/apu/ch3.rs:157-163](../../../src/apu/ch3.rs) corrupts whenever `enabled && dac_enabled`, with no window check. Correct behaviour: corrupt **only** when the trigger coincides with the sample latch (the same `wave_form_just_read` flag that gates reads in 09 and writes in 12).
- **Missing the 4-byte block copy.** The same code does `wave_ram[0] = wave_ram[(wave_pos/2 + 1) % 16]` — a single byte, always. Offsets ≥ 4 must copy four aligned bytes into `wave[0..3]`. Every hit line with `o ≥ 4` (the majority: 24 of 34) will be wrong.
- **Off-by-one source offset.** The `+ 1` in `(wave_pos/2 + 1) % 16` is a heuristic that compensates for having no window. With a real window the source is `wave_pos / 2` exactly — the byte the channel has *just* latched.
- **Corrupting after the trigger reset.** If you zero `wave_pos` before computing `o`, every hit degenerates to `o = 0` (self-copy) and the output looks pristine everywhere — the same symptom as having no corruption at all.
- **Coarse APU stepping.** The re-trigger write commits in the 3rd M-cycle of `ldh (n8),a`. If the APU is advanced by a whole instruction before/after the write, the effective probe time moves by up to ±12 T = ±3 positions on the 4-T grid, scrambling which `o` each iteration sees.
- **Reloading the period on the `NR33` write** at +20 T flattens the phase sweep — every iteration would produce the same line.
- **Not ticking CH3 when muted** (`NR32 = 0`, `NR51 = 0`) freezes `wave_pos` at 0 ⇒ every hit is a self-copy ⇒ 69 pristine lines.
- **Position wrap at 32 nibbles**, not 16 bytes — visible in the last three iterations.
- **Wave RAM must be immediately readable after `NR30 = 0`.** If your implementation keeps the lock for a while after the DAC is turned off, all 16 printed bytes become `$FF`.

## Notes
- CRC-32 is over the 1104 raw byte values only; the interleaved spaces and the 69 newlines are printed with `print_char_nocrc` and excluded ([common/numbers.s:16-23](../../../src/roms/test/blargg-test-roms/dmg_sound/source/common/numbers.s), [common/printing.s:24-41](../../../src/roms/test/blargg-test-roms/dmg_sound/source/common/printing.s)).
- Unlike 12, this test has no settling `delay` after `NR30 = 0` — it reads wave RAM the very next M-cycles, so a stale lock is immediately fatal.
- The only genuinely uncertain parameter in this analysis is Δ (trigger→first-fetch delay). Everything else follows directly from the assembly.
- Because wave RAM is reloaded at the top of every iteration ([:21-22](../../../src/roms/test/blargg-test-roms/dmg_sound/source/10-wave trigger while on.s)), a wrong line does not poison subsequent lines — each line is an independent observation.
