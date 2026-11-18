# Performance and latency of fw-nrf, kb-core and kb-display

Status: investigation (November 2026). No source files were changed; the only additions are the
benchmark crate `bench/` and this document.

## TL;DR

- **The 1 kHz loop has plenty of headroom.** On the nRF52840 one idle tick costs about
  60–80 µs (6–8 % of the CPU). Most of that is the busy-wait in the matrix scan, not the engine.
  A key event adds about 100 µs, and about 90 % of that goes to two loops over 256 entries
  (`emit_report` and `Report::boot_keys`).
- **For latency the logic matters, not the CPU.** A plain key takes about 5.5–7 ms from contact
  to USB, and 5 ms of that is the defer debounce. Keys that start a combo (t, y, j, k, z, x, c, v
  on the base layer) are held back for up to 50 ms. In slow typing every one of them waits the
  full 50 ms; that is 9 of the 43 letters in "the quick brown fox…".
- **The display task does not block on I²C**: the transfer uses EasyDMA and interrupts. It does
  render on the executor thread, though, about 0.4–1 ms of CPU, every 50 ms, even when nothing
  changed.
- **Power: the loop polls at 1 kHz forever.** Together with an HFXO that never stops, the
  instruction cache left off and the LDO regulator, this is roughly 0.5–0.75 mA while awake and
  idle. An interrupt-driven idle scan (ZMK's approach) brings this down to a few µA.

## How to reproduce

```sh
# host benchmarks (engine, debounce, report helpers, screens, SSD1306 bytes)
./dev sh -c 'cd bench && cargo run --release'

# ARM build, sizes, symbols, disassembly
./dev sh -c 'cd fw-nrf && cargo build --release'
./size-report
./dev sh -c 'cd fw-nrf && B=$(rustc --print sysroot)/lib/rustlib/x86_64-unknown-linux-gnu/bin;
  $B/llvm-size -A target/thumbv7em-none-eabihf/release/fw-nrf;
  $B/llvm-nm --size-sort --demangle -S target/thumbv7em-none-eabihf/release/fw-nrf | tail -40;
  $B/llvm-objdump -d --demangle --no-show-raw-insn target/thumbv7em-none-eabihf/release/fw-nrf > /tmp/fw.s'
```

`bench/` is a standalone crate (it has its own `[workspace]`, like `fw-nrf`), so the main workspace
is untouched. It builds into `target/bench`, which is git-ignored. It uses plain `std::time::Instant`
loops: each case is calibrated to about 20 ms per sample, and the median of 7 samples is reported.
It has no dependencies beyond `kb-core` and `kb-display`, and builds offline.

**Host:** AMD Ryzen AI 7 445, `powersave` governor, cpufreq max 2.0 GHz (cores measured at
1.4–2.0 GHz). Results vary by about ±10 % between runs. The host cycle counts below assume 2 GHz.

## 1. Engine cost per event (host)

| Case | Host time |
|---|---|
| Rolling typing "the quick brown fox…", per event (press or release + drain) | 200–240 ns |
| Mod-taps (Alt+A by interrupt, Bspc tap, Alt by timeout), per event | 180–225 ns |
| Combos (copy, lone J, 6-key `reset_bluetooth` on nav), per event | 72–76 ns |
| Layers (sym, wm via conditional layer, nav), per event | 115–155 ns |
| Full 1 kHz loop simulation while typing, per ms (tick + poll + events) | 15 ns |
| `tick` + `poll`, idle | 7 ns |
| `tick` with a combo key buffered (recomputes `combo_deadline`) | 26–28 ns |
| `next_deadline` with a combo key buffered | 22–25 ns |
| Press + release of Q, incl. poll (2 reports) | 330–400 ns |
| `binding()` for all 42 positions with wm active (4-layer `lookup`) | 88 ns (2 ns each) |
| Model of `emit_report` (rebuild a report from 256 counts) | 125–160 ns |
| `Report::boot_keys` (2 keys down) | 263–280 ns |
| `Report::keys().count()` | 185–270 ns |
| `report::diff` (used by sim and tests only) | 630–710 ns |
| `Debouncer::update`, 42 keys, idle or with 2 bouncing keys | 22 ns |
| `Engine::new` | 43 ns |
| `size_of::<Engine<42>>()` | 2880 B (2176 B of it is the 64-entry output queue) |

Hot spots, in order of cost:

1. **`emit_report`** rebuilds the whole report from `key_counts: [u8; 256]` on every `key_down`
   and `key_up`. A plain tap therefore runs the 256-entry loop twice. This is about 40–50 % of a
   plain key event on the host, and more on the M4 (see section 3).
2. **`Report::boot_keys` / `Report::keys()`** test all 256 bits with `contains` until 6 keys are
   found. With fewer than 6 keys down (the normal case) every bit is tested. fw-nrf calls it once
   for every report.
3. **`candidates()` / `combo_deadline()`** recompute `Combo::mask()` (a fold over the combo's keys)
   and `combo_enabled` (with `highest_layer` and `layers.contains`) for all 6 combos on every
   call. `combo_press` calls `candidates` twice, and `tick` calls `combo_deadline` again on every
   loop pass while a key is buffered. The cost is small, about 20–30 ns on the host, but it is
   per call.
4. `lookup` is negligible: at most 4 layers, about 2 ns.
5. `report::diff` (630+ ns, two 256-bit loops) is not used in the firmware, only in `sim` and the
   tests.

### Engine-added latency (measured by the bench, 1 kHz simulation)

The bench measures the time from the debounced press to the first report that contains the key:

| Typing pattern | Avg | Max | Delayed keys |
|---|---|---|---|
| Rolling (60 ms per key, held 70 ms, 10 ms overlap) | 3.0 ms | 50 ms | 9 of 43: the first `t` 50 ms; c, k, x, j, v, t, z, y 10 ms each (flushed by the previous key's release) |
| Slow (150 ms per key, held 80 ms, no overlap) | 10.5 ms | 50 ms | 9 of 43: t, c, k, x, j, v, t, z, y, 50 ms each |

Every key that can start a combo waits until the combo resolves. That happens when another key
is pressed, any key is released, or the 50 ms timeout passes. `t` alone is about 9 % of English
letters.

## 2. Display cost (host)

| Case | Host time | Lit pixels |
|---|---|---|
| `render` Zmk, left (BLE) | 2.8–2.9 µs | 246 |
| `render` Zmk, right (notification) | 6.4–6.6 µs | 344 |
| `render` Portrait, left, USB (what fw-nrf shows) | 4.4–4.5 µs | 349 |
| `render` Portrait, left, BLE + CAPS badge | 6.2–6.4 µs | 647 |
| `render` Portrait, right, no notification | 1.2 µs | 107 |
| `render` Portrait, right, notification (word-wrapped) | 11.6–12.0 µs | 521 |
| `ssd1306::frame_data` (513 bytes, inverted) | 18 ns | |
| `Frame == Frame` (512 bytes) | 11 ns | |

The cost scales with the number of pixels each glyph visits. Each glyph pixel goes through
embedded-graphics' iterator, `Portrait::draw_iter` (bounds check, `module_xy` rotation) and then
`Frame::set` (a second bounds check and a read-modify-write of one byte). The filled CAPS badge
goes through the default `fill_solid` → `fill_contiguous` → `draw_iter`, one pixel at a time
(330 pixels). Landscape (`Style::Zmk`) is cheaper per glyph because it skips the rotation, but it
uses the bigger 7×13 font everywhere.

## 3. Estimate for the nRF52840 (Cortex-M4F, 64 MHz)

### Assumptions

- **Static cycle counts** come from the release disassembly of `fw-nrf` (opt-level `s`, LTO),
  using Cortex-M4 timings. Most ALU instructions take 1 cycle; LDR/LDRB take 2 (1 when
  pipelined); a taken branch takes 1 + P, with P = 1–3 (pipeline refill).
- **Flash wait states / instruction cache:** embassy-nrf 0.11 never sets `NVMC.ICACHECNF`, and its
  reset value is "cache off" (Zephyr/ZMK turn it on with `CONFIG_NRF_ENABLE_ICACHE`). With the
  cache off, fetches after branches cost extra cycles. The estimates below therefore use a range,
  with the upper end assuming about 1.3–1.5× for the cache being off.
- **Branchy code that has not been analysed statically** (embedded-graphics rendering) is scaled
  from the host. A Zen core at 2 GHz runs about 3–4 instructions per cycle on this code, and the
  M4 about 0.7–0.9, so the M4 needs 3–5× the host's cycles. Combined with the 31× clock ratio
  this gives about 100–150× the host time. A blanket factor like this is **wrong for simple loops**:
  the host unrolls or vectorises the 256-entry loops (about 1.3 host cycles per entry), while the
  M4 needs 8–14 cycles per entry. That is roughly 300–400× the host time, so the loops are counted
  instruction by instruction instead.

### Per-tick work (main loop, every 1 ms)

| Part | Basis | Estimate on chip |
|---|---|---|
| Matrix scan: 6 × `cortex_m::asm::delay(128)` | The inlined loop runs 129 iterations of `subs`/`bne` at 3–5 cycles each, about 390–650 cycles = **6–10 µs per column, not ~2 µs** | 36–60 µs |
| Matrix scan: 4 row reads + `matrix_position` per column | About 25 instructions per row in the disassembly | 3–5 µs |
| `Debouncer::update` over 42 keys (idle path) | About 10 instructions per key | 8–10 µs |
| `engine.tick` + `poll` (idle) | Early exits | < 1 µs |
| `Instant::now`, `power.tick`, RTC IRQ, time queue, executor poll, Ticker | Estimate | 5–10 µs |
| **Idle tick total** | | **≈ 55–85 µs (5–8 % CPU)** |

### Per key event (on top of the tick)

| Part | Basis | Estimate on chip |
|---|---|---|
| `emit_report`: 256 × (`ldrb`, `cbz` taken, `adds`, `cmp`, `bne` taken) | 8–10 cycles × 256, plus memclr and memcmp | 2.2–2.8 k cycles = 35–45 µs per `key_down`/`key_up` |
| `Report::boot_keys` in `main`: 256 × (10 instructions, 2 branches) | 12–14 cycles × 256 | 3.1–3.6 k cycles = 50–55 µs per report |
| Combo stage, `lookup`, behaviors, queue push/pop (33-byte memcpys) | Estimate from function sizes | 0.5–1 k cycles = 8–15 µs |
| **One key change → one USB report** | | **≈ 95–120 µs** |

Worst realistic millisecond: all six keys of the 6-key combo released in the same scan
(6 × ~100 µs), with a display render queued behind it on the executor (up to 1 ms, see below).
That is about 1.5 ms, so one or two ticks start late. `Ticker::next` catches up immediately
(it checks `expires_at <= now` and adds one period at a time), so no scans are lost; they are
only shifted by up to about 1 ms. In the normal case the loop uses less than 20 % of the 1 ms
budget even while typing fast.

### Display on chip

- `render` Portrait/left/USB: 4.4 µs × 100–150 ≈ **0.45–0.7 ms**, up to about 1 ms with the cache
  off. A cross-check by pixel count gives about 1,200 glyph/icon/line pixel visits at
  30–50 cycles each, which is 36–60 k cycles = 0.55–0.95 ms, consistent with the scaled estimate.
- Portrait/right with a notification (later stage, right half): 12 µs → **1.2–1.8 ms**.
- `frame_data` + `Frame` compare + `Frame::default`: under 50 µs.
- I²C: 9-byte header + 513-byte data transaction ≈ (1 + 513) × 9 bits / 400 kHz ≈ **11.6 ms plus
  about 0.3 ms for the header**, done by EasyDMA. `Twim::write` awaits the STOPPED/ERROR interrupt
  (`async_wait`), so **the transfer does not block the keyboard loop**.
- **CPU-bound part:** `screens::render` runs synchronously in `display_task`, on the same
  thread-mode executor as the main loop. `SCREEN` is signalled every 50 ms. While `Active`, the
  task re-renders every time and only then compares the frame with `shown`. That is about
  0.5–1 ms of CPU 20 times a second (**1–2 % CPU**, and up to 1 ms of jitter on the 1 kHz loop)
  for frames that are almost always identical.

### Flash and RAM (`./size-report` sections)

- `.text` 30,652 B + `.rodata` 4,448 B + vector table 256 B + `.data` 24 B ≈ 35.4 KB of flash
  (4.4 % of the 792 KB firmware region). `.data` + `.bss` = 8,152 B of RAM (3.1 %), not counting
  the stack.
- Largest functions: the main task `poll` (5.8 KB, with the scan, debouncer and output handling
  inlined), `usb_task` (3.9 KB), the display task (2.3 KB), `Text::draw::<Portrait>` (2.7 KB) and
  `Engine::tick` (0.5 KB). The `Engine` functions are all under 250 B each.
- Task pools: main 3.7 KB (it holds the 2.9 KB `Engine`), display 1.8 KB (`Frame`, `shown` and the
  513-byte `frame_data` array live in the future). None of this is a concern.

## 4. Latency budget: key contact → USB report

| Stage | Plain key | Notes |
|---|---|---|
| Wait for the next scan | 0–1 ms (avg 0.5) | 1 kHz `Ticker` |
| Debounce (defer, 5 ms stable) | 5 ms + bounce time | The timer restarts on every bounce, so a switch that bounces for 2 ms costs 7 ms |
| Scan, debounce and engine CPU | ≈ 0.1 ms | Section 3 |
| Engine logic | 0 | **Combo-start keys: up to 50 ms** (section 1). **Mod-tap tap: sent on release** (≈ hold time, 50–150 ms). Mod-tap hold: at the next key press or after 200 ms |
| Channel → `hid_task` | µs | Same executor pass, after `main` awaits the `Ticker` |
| USB IN endpoint, host polls every 1 ms | 0–1 ms (avg 0.5) | Not synchronised with the scan tick. Two reports in the same ms (a mod-tap tap sends press + release) take 2 polls |
| **Total, plain key** | **≈ 5.1–7 ms (avg ≈ 6 ms) + bounce** | ZMK with default debounce is in the same range |

The display task does not add latency. Its I²C transfer is asynchronous; only the 0.5–1 ms
render can push back one scan tick (see above).

## 5. Power-relevant behaviour

What fw-nrf does now:

- The `main` loop wakes 1000 times a second in every power state except deep sleep: `Active`,
  and also `Idle` with the OLED off, for up to 1 hour before System OFF. Each wake is about
  60–80 µs of CPU, about 7 % duty, mostly spent in the settle busy-waits.
- `HfclkSource::ExternalXtal`: `embassy_nrf::init` starts the HFXO and never stops it, even on
  battery.
- `NrfConfig::default()` leaves both DCDC regulators off (LDO) and never enables the instruction
  cache.

Rough idle current while awake. These are nRF52840 product-specification typicals and should be
measured with a PPK2 before relying on them:

| Part | Current |
|---|---|
| CPU running from flash ≈ 6.3 mA (LDO) / 3.3 mA (DCDC), × 7 % duty | ≈ 0.45 mA (LDO) / 0.23 mA (DCDC); more with the cache off |
| HFXO running permanently | ≈ 0.25 mA |
| System ON base (RTC, RAM retention) | ≈ 3 µA |
| **Total, idle but awake** | **≈ 0.5–0.75 mA**, which is about 0.5–0.75 mAh for every idle hour before deep sleep |

Interrupt-driven idle (what ZMK's `kscan-gpio-matrix` does unless `CONFIG_ZMK_KSCAN_MATRIX_POLLING`
is set):

1. When no key is down and every debouncer is settled, drive **all** columns high and arm a
   GPIOTE PORT/SENSE (`DETECT`) interrupt on the rows. fw-nrf already does exactly this for System
   OFF in `sleep.rs`. In Embassy: `Input::wait_for_high()` on each row (the `gpiote` feature is
   already enabled), combined with `select`.
2. The CPU sleeps in WFE; only the RTC runs. No 1 kHz wakes.
3. On a row interrupt, go back to scanning at 1 kHz (ZMK: `debounce-scan-period-ms`, 1 ms) until
   all keys are released and the debouncers are idle, then re-arm.
4. Keep waking for engine deadlines while a combo or mod-tap is pending (`Engine::next_deadline`),
   and for `Power::next_deadline`, using a `Timer::at` instead of the Ticker.

Expected idle current: a few µA plus the OLED (which is off in `Idle`) and, later, the BLE
connection events. That is about 100× less than polling, for the large part of the day the
keyboard is idle. The lower-effort companions are: enable the DCDC (`config.dcdc.reg1 = true`;
nice!nano v2 has the inductor, and ZMK's board definition runs it in DCDC mode, but check against
the board files), enable `NVMC.ICACHECNF.CACHEEN`, and request the HFXO only while VBUS is
present. On battery, HFINT is enough without USB; BLE will later manage the HFXO through the
radio stack.

## 6. Ranked optimisation suggestions

| # | Change | Expected gain | Effort | Risk |
|---|---|---|---|---|
| 1 | **Interrupt-driven idle scan** (section 5) | Idle current ≈ 0.5–0.75 mA → a few µA; about 100× on battery for idle time | Medium (scan/idle state machine in `fw-nrf/src/main.rs`; deadlines from `Engine::next_deadline` and `Power::next_deadline`) | Medium: must not miss a press during the switch from idle to scanning (rescan immediately on wake), and must not re-arm while a debouncer is mid-change |
| 2 | **Cheap power and speed settings:** enable the ICACHE, DCDC `reg1`, and start the HFXO only with VBUS | Roughly 1.3–2× faster code and lower run current (DCDC ≈ half the current of the LDO); ≈ 0.25 mA less on battery from the HFXO | Low (a few lines in `main`) | Low (check DCDC against the nice!nano schematic; USB needs the HFXO, so start it when VBUS appears) |
| 3 | **Shorter press latency:** eager press debounce (report on the first closed reading, then ignore the key for 5 ms; keep defer for releases), or `press_ms` 1–2 | −3…−5 ms on **every** key, which is most of the 6 ms total | Low (new mode in `core/src/debounce.rs`, with tests) | Low–medium: a noise spike becomes a press. Unlikely with a diode matrix and pull-downs. ZMK users often run `debounce-press-ms = 1` |
| 4 | **Combo latency:** add ZMK's `require-prior-idle-ms` (skip combo buffering when the previous key press was < ~150 ms ago), and/or shorten the timeout to 30–35 ms for the 2-key combos | Removes the 10–50 ms delay on t/y/j/k/z/x/c/v during typing (9 of 43 letters in the bench sentence) | Low–medium (engine + keymap field, with tests) | Low: combos have to be pressed from a short pause, which matches how they are used |
| 5 | **Render only on change:** in `display_task`, skip `render` when `Screen` is equal to the last one (add `PartialEq` to `Screen`; `Status` and `PowerState` already have it), or signal `SCREEN` from `main` only when it changed | Saves 0.5–1 ms of CPU 20×/s (1–2 % CPU) and removes the 1 ms jitter on the scan tick | Trivial | None |
| 6 | **Fix the matrix settle delay:** `asm::delay(128)` runs 129 loop iterations of 3–5 cycles, about 6–10 µs, not the ~2 µs in the comment. Use `delay(64 * 2 / 3)` or a DWT-cycle-based wait, or measure the actually needed settle time (ZMK's `CONFIG_ZMK_KSCAN_MATRIX_WAIT_BEFORE_INPUTS` defaults to 0) | 25–50 µs per tick, about half the idle tick | Trivial | Low: check on the hardware for ghost/missed keys with long row traces |
| 7 | **O(1) report building:** keep the `Report` bitmap up to date in `key_down`/`key_up` (set the bit on a 0→1 count, clear it on 1→0), so `emit_report` only merges mods and compares. Rewrite `Report::keys`/`boot_keys`/`diff` over the 32 bytes (`while byte != 0 { trailing_zeros }`) | ≈ 95–120 µs → ≈ 10–15 µs per key event on chip; host `press+release` 330 → under 100 ns | Low (`engine.rs`, `report.rs`; `core/tests/corne.rs` covers the behaviour) | Low |
| 8 | **Combo fast path:** precompute each combo's mask, plus a union mask of all combo keys for the active layer. If `combo_len == 0` and the position is not in the union mask, go straight to `behavior_press` | About 0.3–0.5 k cycles less per non-combo press; `tick` no longer folds masks while a key is buffered | Low | Low |
| 9 | **Cheaper portrait drawing:** implement `fill_solid`/`fill_contiguous` for `Portrait` and `Frame`, set pixels without the double bounds check, or draw into a 32×128 native buffer and transpose it in 8×8 blocks | 2–3× faster `render` (≈ 0.5 ms → ≈ 0.2 ms) | Medium | Low (screen tests compare pixels) |
| 10 | **Partial OLED updates:** send only the changed column range or pages (the SSD1306 window commands are already in `FLUSH_HEADER`) | WPM or layer changes would send ~50–150 bytes instead of 513: shorter bus activity (≈ 11.6 ms → 2–4 ms) and less I²C current | Medium | Low (the emulator decodes the windows) |

Items 5, 6 and 7 are small, independent edits. Items 1 and 2 matter most for battery life, and
items 3 and 4 for feel.

## 7. Correctness and latency problems noticed along the way

1. **The OLED wakes up every hour on USB power.** When `Power` reaches `Sleep` and
   `sleep::usb_powered()` is true, `main` calls `power.activity(now)`, which sets the state back to
   `Active`. The next `SCREEN` signal, within 50 ms, then switches the display on again, and it
   stays on for the 30 s idle timeout. The comment says "only the display blanks", but the effect
   is the opposite. Keep the state at `Idle` or `Sleep` without touching `last_activity` (for
   example a `Power` method that clamps to `Idle`), or have the display task treat
   `Sleep` like `Idle`.
2. **Reports are dropped when the HID channel is full.** `REPORTS.try_send` silently drops new
   reports once 16 are queued. `HidWriter::write` → `wait_data_ready` blocks while the host
   suspends the device: the endpoint stays enabled but is not polled. Typing during host suspend
   then loses the newest reports, including releases, and after resume the host sees the old
   presses without their releases, so keys stick until pressed again. Fix: on overflow, drop the
   oldest queued report or collapse to the latest state, so the final state always gets through.
   Consider remote wakeup.
3. **Boot report rollover.** `boot_keys` sends the 6 *lowest usages*, not the first 6 pressed, and
   with more than 6 keys down it silently omits keys. The boot protocol expects `ErrorRollOver`
   (0x01 in all six slots). This rarely matters, but it can drop a key while chording on the wm
   layer (Super + Ctrl + Shift + key is only one non-modifier, so fine there).
4. **Combo-start keys add 10–50 ms** (sections 1 and 4). This is the main user-visible latency
   source after debounce, and `t` is the most frequent letter affected. ZMK has the same mechanism
   but offers `require-prior-idle-ms`.
5. **The mod-tap thumbs (F20/Bspc, F20/Del) are hold-preferred without quick-tap.** Backspace
   is only sent on release, holding Backspace to auto-repeat is impossible (it becomes F20), and
   rolling from Backspace into the next letter turns into F20+letter. This is the same behaviour as
   the ZMK config it mirrors, but `quick-tap-ms` / `hold-trigger-key-positions` would help.
6. **Theoretical stuck modifier:** if `held_hold_taps` (8 slots) is full, `decide_hold` still
   presses the hold key but never records it, so its release never happens. It cannot occur with 3
   mod-tap keys, but a `debug_assert!` or a fallback would make it safe for other keymaps.
7. **The matrix settle comment is off by 3–5×** (about 6–10 µs per column, not about 2 µs). This
   is harmless for correctness, but it costs CPU and power (suggestion 6).
8. **The display re-renders every 50 ms** even when nothing changed (suggestion 5).
9. Not a bug, but worth knowing: the engine's output queue (64 × 34 B) is 75 % of the `Engine`'s
   2.9 KB. If outputs are ever polled less often, an overflow drops them silently
   (`Queue::push` returns `false` and the result is ignored).

## Appendix: benchmark output (one run)

```text
sizes: Engine<42> 2880 B, Output 34 B, Frame 512 B

== engine-added latency (press -> report with the key) ==
rolling typing (60 ms/key, 10 ms overlap)    avg   3.0 ms, max 50 ms; 9 of 43 keys delayed: t:50 c:10 k:10 x:10 j:10 v:10 t:10 z:10 y:10
slow typing (150 ms/key, no overlap)         avg  10.5 ms, max 50 ms; 9 of 43 keys delayed: t:50 c:50 k:50 x:50 j:50 v:50 t:50 z:50 y:50

== engine (Corne keymap) ==
typing "the quick brown fox ...": per event       201.5 ns (86 events)
typing "the quick brown fox ...": 1 kHz loop, per ms       15.1 ns (tick + poll + events)
mod-taps: per event                               182.2 ns (8 events)
mod-taps: 1 kHz loop, per ms                        7.2 ns (tick + poll + events)
combos (2-key, lone key, 6-key): per event         75.2 ns (20 events)
combos (2-key, lone key, 6-key): 1 kHz loop, per ms        7.7 ns (tick + poll + events)
layers (sym, wm, nav): per event                  114.9 ns (10 events)
layers (sym, wm, nav): 1 kHz loop, per ms           6.8 ns (tick + poll + events)
Engine::new                                        42.5 ns
tick + poll, idle                                   6.8 ns
tick, combo key buffered (not due)                 26.1 ns
next_deadline, combo key buffered                  22.0 ns
press+release Q (+ poll)                          332.6 ns (2 reports)
binding() x42 on wm layer                          87.3 ns

== report ==
emit_report model (256 counts -> Report)          162.5 ns
Report::boot_keys (2 keys down)                   263.0 ns
Report::keys().count()                            185.6 ns
report::diff                                      630.2 ns

== debounce (42 keys) ==
update, all keys idle                              22.6 ns
update, 2 keys toggling every 3 ms                 22.5 ns

== display ==
render Zmk, left (BLE)                           2837.1 ns
render Zmk, right (notification)                 6438.2 ns
render Portrait, left (USB, fw-nrf)              4428.5 ns
render Portrait, left (BLE + CAPS badge)         6173.0 ns
render Portrait, right (no notification)         1168.8 ns
render Portrait, right (notification)           11982.6 ns
ssd1306::frame_data (513 bytes)                    18.3 ns
Frame == Frame (512 bytes, equal)                  10.9 ns
```

## Applied (2026-11-15)

What came out of this report and was implemented:

| Item | Change | Effect (measured on the host with `bench/`) |
|---|---|---|
| Report building | `Engine` keeps the report bitmap and modifier bits up to date per key; `Report::keys` skips empty bytes | typing 201 → 45 ns per event, mod-taps 182 → 42 ns, press+release Q 330 → 85 ns |
| Boot report > 6 keys | `Report::boot_keys` sends ErrorRollOver (0x01) in all slots | correct HID behaviour instead of the 6 lowest codes |
| Mod-tap hold slots | 16 slots; when none is free the key sends its tap, so no modifier can stay held | covered by `engine::tests` |
| Combo latency | `require_prior_idle_ms` (ZMK `require-prior-idle-ms`), 150 ms on the typing combos; only keycodes count | rolling typing: 9 → 1 of 43 keys delayed (avg 3.0 → 1.2 ms); slow typing at exactly 150 ms per key: 9 → 7 |
| Debounce | fw-nrf uses an eager press (0 ms) with a 5 ms release debounce | saves ~5 ms on every press |
| Idle scanning | fw-nrf waits on a row interrupt (all columns high) when no key is held, nothing is pending and no engine timeout is due; it still wakes for the idle/sleep deadline | no 1 kHz wake-ups while idle |
| USB power | `Power::set_sleep_allowed(false)` while VBUS is present, instead of faking activity | the display no longer turns on for 30 s every hour |
| Report queue | when full, the queue is cleared and the newest report kept | the last state (with every release) always reaches the host |
| Display | the display task skips identical `Screen`s | no render every 50 ms when nothing changed |
| Settle time | `asm::delay(32)` (~1.5–2.5 µs) instead of `delay(128)` | ~4× shorter column scan |
| Instruction cache | enabled at boot | fewer flash wait states |

Not done:

- **DC/DC regulator:** whether the nice!nano v2 has the inductor REG1 needs isn't confirmed. Enabling DC/DC without it stops the chip, so it stays off until the schematic is checked.
- **HFXO only on USB:** moot until Bluetooth exists, whose stack manages the crystal itself.
