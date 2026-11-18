# Renode for the nRF52840 firmware

Can Renode, Antmicro's open-source emulator, run and test the real `fw-nrf` firmware before it
goes on the nice!nano? Short answer: **yes, for boot, key-matrix and OLED tests on both the
`usb-only` and the `ble` builds, after a handful of small platform fixes. It cannot test USB, and it
cannot test the Bluetooth split link.** All of this was tried on Renode 1.17.0 (portable Linux build
`renode_1.17.0+20261114git89ea9e587`, self-contained .NET 10, no root needed) with ELFs built in the dev
container. Every result below comes from those runs unless it says otherwise.

## 1. What the stock `nrf52840.repl` models

From `platforms/cpus/nrf52840.repl` in the release, plus what the runs logged when the firmware
touched each block. When no model exists, Renode falls back to the SVD file: reads return the SVD
reset value and writes are dropped, with a warning for each access.

| Block | Model | What we saw |
|---|---|---|
| CPU, NVIC | `CortexM` (cortex-m4) | Fine. Reset fetches SP/PC from VTOR, so `cpu VectorTableOffset 0x26000` is needed because there is no bootloader. |
| CLOCK / POWER | `NRF_CLOCK` (shared 0x40000000) | HFCLK/LFCLK start and STARTED events work, and MPSL's clock driver gets its events. It also reports `USBREGSTATUS` as VBUS present / output ready (our `vbus_task` saw USB power), but USB power events and timing are not real. |
| RTC0/1/2 | `NRF52840_RTC` | Works, with two gaps. **The platform declares `rtc1` with 3 compare channels, but the chip has 4**, and embassy's time driver uses CC[3] for its half-period interrupt. **OVRFLW is not handled.** For runs under about 256 s this doesn't matter. Fix: an overlay with `rtc1: numberOfEvents: 4`. |
| TIMER0–4 | `NRF52840_Timer` | Works for MPSL. `BITMODE` and `SHUTDOWN` are ignored. |
| GPIO P0/P1 | `NRF52840_GPIO` | IN/OUT/DIR/PIN_CNF work. **`LATCH` and `DETECTMODE` are not implemented.** |
| GPIOTE | `NRF52840_GPIOTasksEvents` | IN events and the PORT event fire. embassy's PORT handler, however, reads `P0.LATCH` to find which pin woke it. LATCH always reads 0, so `Input::wait_for_high()` never completes and **key presses don't wake the idle key loop**. Fix: a one-line read hook (see §3). |
| PPI | `NRF52840_PPI` | Present. MPSL/SDC configure it with no errors logged. |
| EGU0–5 | `NRF52840_EGU` | Present. MPSL's low-priority SWI works. |
| RNG | `NRF52840_RNG` | Works (SDC seeding). |
| ECB | `NRF52840_ECB` | Present. |
| **CCM, AAR** | none | BLE link encryption (CCM) and address resolution (AAR) are missing, so pairing and encrypted links cannot work even if a connection were made. |
| **TEMP** | none | **MPSL calls `mpsl_temperature_get()` in `mpsl_init` and waits forever** for `EVENTS_DATARDY`. Fix: a 10-line Python stub. |
| **FICR** | tag only (reads 0) | **MPSL resets the chip** (SYSRESETREQ inside `mpsl_init`) when the undocumented part/revision words at 0x10000130/134 read 0. Fix: `sysbus Tag` them to 8/0. |
| **NVMC** | none (SVD) | `ICACHECNF` and `CONFIG` are ignored. Flash is plain `MappedMemory`, so CPU stores land without erase semantics. Bond storage via MPSL timeslots was not exercised (no pairing happens). |
| **SAADC** | none (SVD) | `battery_task` hangs on `calibrate()` (no `CALIBRATEDONE`), and the screen keeps the default 100 %. Harmless, and stubbable like TEMP. |
| TWI0/1 | `NRF52840_I2C` | **Legacy TWI only, no TWIM/EasyDMA.** `TXD.PTR`/`MAXCNT` writes are "unhandled", ENABLE=6 is rejected ("Wrong enabled value"), and no byte reaches a slave. embassy's `twim` doesn't hang, because each write just fails. Fix: a small TWIM model (see §3). |
| **USBD** | none (SVD) | embassy-usb writes `ENABLE`, then waits for `EVENTCAUSE.READY`, which never arrives. Enumeration is impossible. Renode has no nRF USBD model and no USB host side for it. |
| RADIO | `NRF52840_Radio` + BLE medium | Advertising TX works: the right half sent advertising packets onto `CreateBLEMedium`. Scanning RX on the left logged `ERROR radio: Unhandled short DisabledRSSIStop!`, and many registers (CCA, BCC, `WHITEEN`, `PHYEND`, the MPSL FEM bits) are unhandled. |

Renode's BLE support was built around **Zephyr's open-source link layer** on the nRF52840 (two
`central_hr`/`peripheral_hr` nodes on a wireless medium). It was not built for Nordic's closed
SoftDevice Controller. I found no documentation or report of nrf-sdc or MPSL working under Renode.
One renode-infrastructure PR notes that a Nordic SoftDevice peripheral against a Zephyr central gave
"no functional connection".

## 2. Does our binary boot to USB start?

Runs: 10 s of emulated time each. Host time was about 10–15 s per run, at about 1× real time.

**`left,usb-only` on the stock platform:** boots, starts the display and battery tasks, enters the
key loop, writes `USBD.ENABLE` (after the USB errata pokes at 0x4006EC00), then idles in the executor
`WFE`. Only peripherals are missing: no OLED bytes (TWIM), battery stuck (SAADC), USB stuck waiting
for READY (USBD).

**`left,ble` and `right,ble` on the stock platform:** the chip resets after about 1 ms inside
`mpsl_init` (`nvic: Resetting platform with SYSRESETREQ`). After the reset VTOR is 0 and there is no
bootloader, so the CPU runs off into empty flash. Locating it took `cpu LogFunctionNames true true`:
the last calls were `mpsl_init` → reads of FICR 0x10000130/134 → reset. **This is a Renode platform
gap, not a firmware bug.** On hardware the firmware's own panic hook would not be involved either
(it was not `sys_reset` from `diag.rs`).

**`ble` builds with three fixes** (FICR part/revision tags, TEMP stub, 4-channel RTC1): MPSL and the
SDC initialise. The right half advertises, about 60 radio events in 9 s. The left half sets scan
parameters and enables scanning. `mpsl_clock_hfclk_src_request` completes, `vbus_task` and `usb_task`
start, and `USBD.ENABLE` is written. In other words, **the left BLE build gets through every boot
stage (`radio` → `split` → `hfclk` → `usb`)** and the OLED then shows the normal status screen.
With the OLED model, `BOOT / radio` is on screen at t = 40 ms and the status screen by t = 0.54 s.

**Two machines on one BLE medium** (left central plus right peripheral, `emulation CreateBLEMedium`,
10 µs quantum, 10 s): no link (`split::LINKED` stays 0 on both). The right half transmits, but the left
radio model doesn't support the RX shortcuts the SDC scanner uses.

**What this means for the "BLE build never enumerated USB" bug:** Renode would have shown that the
firmware reaches `start_usb` and enables USBD, i.e. no hang or panic in MPSL init, HFCLK or the split
task. It could not have shown anything past that point. There is no USBD model, and the POWER/USB
regulator behaviour (VBUS events, `USBPWRRDY`, the HFCLK that MPSL owns) is simplified, so a bug in
how USB power or the crystal is handed over under MPSL is invisible in Renode.

## 3. OLED, keys and Robot Framework tests

Renode compiles C# peripherals at runtime with `include @file.cs`. A prebuilt `.dll` is *not* accepted
by `include`, because it is parsed as a script. Instead, the PoC concatenates
`emulator/Ssd1306/Ssd1306.cs` unchanged, with only its file-scoped namespace turned into a block, into
one file with three glue classes (about 200 lines in total):

- `I2C.NRF52840_TWIM`: TWIM with EasyDMA. STARTTX/STARTRX/STOP, `SHORTS` LASTTX→STOP and LASTTX→STARTRX,
  STOPPED/ERROR/LASTTX/LASTRX events with IRQ, ANACK when no slave answers, reads and writes RAM through
  the system bus. It is a `SimpleContainer<II2CPeripheral>`, so slaves attach at an address.
- `I2C.CorneSsd1306`: an `II2CPeripheral` that feeds each write to `Kb.Ssd1306.Ssd1306(32)` and exposes
  `IsLit`, `LitPixels`, `Portrait()` (32×128 ASCII art, `#` = lit) and `Landscape()` to the monitor.
- `Miscellaneous.CorneKey`: one switch plus diode. Its input is the column GPIO, its output drives the
  row GPIO, and `Press`/`Release` can be called from the monitor.

Platform overlay (`twi0` replaced, since a model's type cannot be changed by an overlay):

```text
twi0: I2C.NRF52840_TWIM @ sysbus 0x40003000
    -> nvic@0x3
oled: I2C.CorneSsd1306 @ twi0 0x3C

// left thumb key 36 (mo(1)): column P1.15 -> switch -> row P0.11
key36: Miscellaneous.CorneKey @ gpio0 11
    -> gpio0@11
gpio1:
    15 -> key36@0
```

plus these monitor commands:

```text
sysbus Tag <0x10000130, 0x10000133> "ERRATA_PART" 0x8   # MPSL chip check
sysbus Tag <0x10000134, 0x10000137> "ERRATA_REV" 0x0
sysbus SetHookAfterPeripheralRead gpio0 "if offset == 0x20: value = self.ReadDoubleWord(0x10) & 0x01400800"  # LATCH = IN & rows
cpu VectorTableOffset 0x26000
```

The rendered screen matches the real one. It shows `USB`, `100%`, the battery icon, the layer name,
`WPM 0`, and `BOOT` / stage name on the diagnostics screen. Holding key 36 changes the layer name from
the default to the symbols layer, and releasing it changes it back.

A Robot suite run with `renode-test` (Robot Framework 6.1 in a scratch venv) passed 3 of 3 tests in 8 s:

```robot
Screen Should Contain
    [Arguments]    ${golden}
    ${screen}=    Execute Command    sysbus.twi0.oled Portrait
    ${want}=      Get File    ${S}/tests/${golden}
    Should Contain    ${screen}    ${want}

*** Test Cases ***
Holding Lower Shows The Symbols Layer
    Create Corne    ${ELF}
    Execute Command    emulation RunFor "1"
    Execute Command    sysbus.gpio0.key36 Press
    Execute Command    emulation RunFor "0.1"
    Screen Should Contain    layer-symbols.txt
    Execute Command    sysbus.gpio0.key36 Release
    Execute Command    emulation RunFor "0.1"
    Screen Should Contain    layer-default.txt
```

The tests were: usb-only boots to the status screen with the default layer; holding Lower shows
symbols; the BLE build gets past MPSL to the status screen. A deliberately wrong golden file makes the
test fail, so the assertion works. The golden files are ASCII crops of the screen. They could just as
well be generated on the host by `kb-display` (the same renderer), so that the expected images never
drift from the code.

Other things that fit easily: assert on the `PANIC` crash screen (inject a panic, add a reset macro
that reloads the ELF, and check that RAM `.uninit` survives); `Assert` on `ext_power` (P0.13) dropping
in deep sleep; time-based idle and sleep tests, which are cheap because virtual time runs at about 1×
or faster when the CPU is idle.

## 4. Running it

Everything ran from a scratch directory outside the repo and needed no installs. It is a 73 MB download:
`https://builds.renode.io/renode-latest.linux-portable.tar.gz`, then
`renode --disable-gui --console -e "include @script.resc"` and `renode-test suite.robot`. In a repo, a
portable Renode download is cached by CI (or the `antmicro/renode-test-action` GitHub action is used),
and the `.cs`/`.repl`/`.robot` files go in e.g. `emulator/renode/`. The scratch files from this
investigation (not in the repo):
`model/CorneRenode.cs`, `scripts/corne-full.repl`, `scripts/corne-keys.repl`, `tests/corne.robot`,
`logs/`.

Useful debugging tools Renode gave us without a probe: `cpu LogFunctionNames true true` (a call trace
with symbols from the ELF, which is how the MPSL reset was found), `cpu AddHook <addr> "<python>"`,
`sysbus LogPeripheralAccess <p> true`, and the "unimplemented register" warnings themselves, which
work as a map of which hardware the firmware touches. Renode also has a GDB server (`machine
StartGdbServer 3333`), so probe-rs-style stepping works in the emulator.

## 5. Recommendation

**Worth doing (about 1 day to put in the repo):**

- A CI boot smoke test for all four feature builds. It catches panics, hangs and resets during boot
  (MPSL init, HFCLK request, task spawning) and checks that each build reaches `start_usb`/the status
  screen. This would have ruled out "it hangs before USB" for the BLE bug.
- Key-matrix to engine to OLED end-to-end tests on the left `usb-only` build: layers, combos, caps
  word, idle and sleep timing, crash screen. They reuse the SSD1306 model and golden frames from
  `kb-display`. Adding more `CorneKey`s is one repl line per key (or a generated matrix of 42).
- Upstreamable fixes: the RTC1 channel count, GPIO LATCH/DETECTMODE, a TWIM model, a TEMP model, and
  the FICR values.

**What it can't replace:**

- **USB**: no USBD model and no host side. Enumeration, HID reports to a PC and USB power handling
  under MPSL need hardware.
- **BLE with nrf-sdc**: MPSL and the SDC do boot and drive the radio, but the radio model lacks the
  scanner's shortcuts plus CCM/AAR, so there is no split link, pairing or bonding. Making it work would
  mean extending the C# radio model against a closed blob, with no specification of what the blob
  expects. That is open-ended work and not worth it here. Keep testing the split protocol logic on
  the host (`kb-core`).
- Analog and timing truth: SAADC/battery, real clock accuracy, power consumption, I²C electrical
  problems.

**Compared with an SWD probe (Raspberry Pi Pico running debugprobe, about €5, plus probe-rs and defmt):**
the probe is what would have found the real USB bug. It gives live `defmt` logs over RTT, breakpoints
and register inspection on the real USBD/POWER/CLOCK, and `probe-rs run` flashing in seconds. It needs
the nice!nano's SWD pads (SWDIO/SWCLK on the back) wired once, and the UF2 bootloader can stay. Renode
complements it rather than replacing it: it gives repeatable tests in CI with no hardware attached,
and the probe gives the truth about the peripherals Renode doesn't model. If only one is affordable
in time, get the probe first for bring-up of USB and BLE, then add Renode for regression tests of
keys, display and boot.

## Sources

- Renode platform and models: `platforms/cpus/nrf52840.repl` and `tests/platforms/NRF52840.robot`
  in the Renode 1.17 release ([renode/renode](https://github.com/renode/renode), models in
  [renode/renode-infrastructure](https://github.com/renode/renode-infrastructure)).
- [Bluetooth Low Energy simulation in Renode](https://renode.readthedocs.io/en/latest/tutorials/ble-simulation.html)
- [Developing and testing BLE on nRF52840 with Renode and Zephyr (Antmicro)](https://antmicro.com/blog/2022/04/developing-and-testing-ble-on-nrf52840-with-renode-and-zephyr/)
  and the [Nordic DevZone version](https://devzone.nordicsemi.com/nordic/nordic-blog/b/blog/posts/developing-and-testing-ble-products-on-nrf52840-in-renode-and-zephyr)
- [BLE HCI integration with Renode](https://renode.readthedocs.io/en/latest/tutorials/ble-hci-integration.html)
- [renode-infrastructure PR #56](https://github.com/renode/renode-infrastructure/pull/56) (Nordic
  radio/PPI work; reports no functional connection with a Nordic SoftDevice)
- [multi-node `nrf52840-ble-zephyr.resc`](https://opensecura.googlesource.com/3p/renode/renode/+/f26d37f9e0f336acb85506582f252e6a18af232b/scripts/multi-node/nrf52840-ble-zephyr.resc)
