# Security review: someone reflashing the keyboard while I'm away

Question: *"If I walk away from my desk, could someone reflash the keyboard with a key logger? Can we
make Bluetooth stop connecting when the keys don't match, so a reflashed keyboard is noticeable?"*

Short answer:

- **Yes, and it is easy.** With the current setup (stock Adafruit UF2 bootloader 0.6.0, SWD left
  open by Embassy's default config), someone with 30 to 60 seconds and a USB cable can install any
  firmware. The bonds stay in flash, so the replacement firmware keeps connecting to your computers
  as if nothing happened.
- **They don't even need to reflash.** The bootloader's `CURRENT.UF2` file is a full readout of the
  application flash, including our bond region. Copying it gives the attacker every Bluetooth
  long-term key (LTK). With a cheap sniffer they can then decrypt your keystrokes over the air,
  silently, for as long as you keep those bonds. Nothing changes on the keyboard, so you have
  nothing to notice.
- **The "stop connecting if the keys don't match" check can't be done in our firmware.** The
  attacker's firmware replaces ours, so our check is simply gone. The check has to come from
  something the attacker cannot replace or read: a bootloader that only accepts signed firmware,
  with the debug port locked. With those two in place your idea comes for free. The only way left
  to install other firmware is a full chip erase over SWD, which also erases the bonds, so the
  keyboard stops connecting until someone pairs it again. Bluetooth encryption itself then proves
  that the device is the one you paired with.
- **Cheapest real fix today:** don't leave the keyboard (both halves) unattended in a shared space.
- **Strong fix (medium effort):** Adafruit bootloader 0.10.0 or newer built with `SIGNED_FW=1`, plus
  `Debug::Disallowed` (APPROTECT) in our firmware, on both halves. Get an SWD probe first.

The rest of this document explains why and gives the plan.

---

## 1. What we have now

| Part | State | Security effect |
|---|---|---|
| Bootloader | Adafruit nRF52 UF2 bootloader 0.6.0 at `0xF4000`, MBR + S140 area below `0x26000` | Accepts **any** UF2 (no signature). Exposes `CURRENT.UF2` (flash readback). Entered by double-tapping reset, or by the app (GPREGRET `0x57`). |
| App | `0x26000`, 760 KB (`fw-nrf/memory.x`) | Our firmware. |
| Bond storage | `STORAGE` `0xE4000..0xEC000` (32 KB), `sequential-storage` map, plain postcard (`ble/src/storage.rs`) | Holds `BondInformation` (LTK, peer identity/IRK) for the split link (key `0`) and the 10 profiles (`0x10+i`). **Not encrypted, not access-protected.** |
| Debug port (SWD) | `embassy_nrf::init(NrfConfig::default())` → `config.debug = Debug::Allowed` | On build code F chips Embassy actively writes `UICR.APPROTECT = HwDisabled` and `APPROTECT.DISABLE` at every boot; on older chips debug is open by default. **SWD is open either way** (see `embassy-nrf-0.11.0/src/lib.rs`, "Setup debug protection"). |
| USB log port | CDC-ACM with `boot`, `p`/`r`/`k` (key injection), `t` (trace reports), `d` (log replay) | Any process on the PC that can open the tty can restart the keyboard into the bootloader and then flash it. There is no physical confirmation. |
| Pairing | LE Secure Connections, Just Works (`NoInputNoOutput`) | Safe against passive sniffing *during pairing*. Not safe against an active man-in-the-middle during pairing (no numeric comparison). Pairing is rare, so the risk is small. |

### Facts checked in the bootloader source

- `CURRENT.UF2` is generated on the fly from flash: `read_block()` in
  [`src/usb/uf2/ghostfat.c`](https://github.com/adafruit/Adafruit_nRF52_Bootloader/blob/master/src/usb/uf2/ghostfat.c)
  does `memcpy(bl->data, (void *)addr, ...)` for every address from `USER_FLASH_START` and stops only
  at `addr < CFG_UF2_FLASH_SIZE` (1 MB). `USER_FLASH_END` is `BOOTLOADER_REGION_START -
  DFU_APP_DATA_RESERVED`
  ([`uf2cfg.h`](https://github.com/adafruit/Adafruit_nRF52_Bootloader/blob/master/src/usb/uf2/uf2cfg.h)).
  For 0.6.0 the reserved area was 28 KB
  ([changelog 0.6.1](https://github.com/adafruit/Adafruit_nRF52_Bootloader/blob/master/changelog.md):
  "Increased application reserved from 28KB to 40KB"), so the file covers `0x1000..0xED000`.
  **Our bond region `0xE4000..0xEC000` is inside the file.** Reading raw sectors past the end of
  the file with `dd` on the block device gives the rest of the 1 MB (ZMK settings and the bootloader
  itself).
- A UF2 write only touches the pages named in the UF2 file (`flash_nrf5x_write(bl->targetAddr,
  ...)`). Nothing else is erased, so **reflashing keeps the bonds**. That is why pairings survive
  `./hil flash` today, and it is also what lets a malicious replacement keep working.
- 0.6.0 checks no signature on UF2. UF2 has no field for one.
- The bootloader can also update itself from a UF2 with family `0xd663823c`
  (`CFG_UF2_FAMILY_BOOT_ID`). An attacker could therefore also replace the bootloader with one that
  hides itself.

---

## 2. Threat model

**Attacker:** someone at the office or a visitor who gets a few minutes alone with the keyboard,
maybe several times. They have a laptop or phone with a USB cable, maybe a $10 BLE sniffer dongle,
and perhaps (less likely) an SWD probe and a soldering iron. Out of scope: an attacker who already
controls your computer (they don't need the keyboard), and lab attacks that take hours with an
oscilloscope.

**What they want:** your keystrokes (passwords, messages), or to type commands into your computer.

### Attacks, cheapest first

| # | Attack | Time and tools | Works today? | Would you notice? |
|---|---|---|---|---|
| A1 | **Dump the bonds:** double-tap reset, copy `CURRENT.UF2`, unplug. Pull the LTKs from the `0xE4000` region (postcard data, easy to parse). Then sniff with an nRF52840 dongle and the [nRF Sniffer](https://docs.nordicsemi.com/bundle/nrfutil/page/nrfutil-ble-sniffer/guides/common_sniffing_actions.html), which takes a known "SC LTK" and decrypts a bonded connection each time it re-encrypts (every reconnect, e.g. after sleep). | ~30 s at the desk, then passive from up to tens of metres | **Yes** | **No.** The firmware is unchanged and nothing on the keyboard or the PC looks different. |
| A2 | **Malicious firmware by UF2:** our firmware (or ZMK) plus a logger, dragged onto `NICENANO`. It reads the existing bonds, keeps the same name and address, and reconnects to your PCs normally. It can send keystrokes in BLE advertisements to a phone nearby, store them in the ~700 KB of free flash for a later visit, or **type** commands (BadUSB style) while the screen is unlocked. | ~30 to 60 s | **Yes** | No. It can copy our OLED screens exactly. |
| A3 | **Reflash from the PC:** malware on your computer sends `boot` to the log port and copies a UF2. This needs no physical access, and the keyboard then carries the malware further (to other PCs on other profiles). | Seconds, remote | **Yes** (left half, `usb-log` builds) | No |
| A4 | **SWD:** a probe on the nice!nano's SWD pads (on the underside) reads or writes all flash, UICR and the bootloader. | Minutes with pogo pins or a clip, needs case access | **Yes** (APPROTECT is kept disabled) | No |
| A5 | **Right half:** the same as A1/A2 on the right half. Its flash holds the split-link bond, so a malicious right half still links to the left and logs right-hand keys, or the attacker sniffs the split link. | Same | **Yes** | No |
| A6 | **Full chip erase over SWD (CTRL-AP ERASEALL),** then the attacker's firmware. Works even with APPROTECT on, but it [erases flash, UICR and RAM](https://docs.nordicsemi.com/bundle/ps_nrf52840/page/dif.html), so all bonds are gone. | Minutes plus a probe | Yes | **Yes**, if you treat "the keyboard needs pairing again" as an alarm. The attacker can't re-pair it with your PC without unlocking the PC. |
| A7 | **Glitch APPROTECT** ([LimitedResults, CVE-2020-27211](https://nvd.nist.gov/vuln/detail/CVE-2020-27211)): a voltage fault during boot reopens debug on older nRF52840 revisions without erasing, so the flash can be read. | Hours, board removed, glitch setup. Hardened in build code F silicon. | Theoretically | No, but it is far outside "walked away for lunch". |
| A8 | **Hardware swap or implant:** a different nice!nano, or a logger wired into the matrix. | Minutes to hours | Yes | A swapped MCU has no bonds, so you notice once A1 is closed. An implant on the matrix wires is not stopped by any firmware. |

**Why your idea fails against today's setup:** in A1, A2 and A5 the attacker's code (or a copy of
your flash) has the same keys as your firmware. "The keys don't match" never happens, because the
keys were never secret from whoever controls the bootloader or the SWD port. Moving the bonds,
hashing the firmware, showing a hash on the OLED, or adding a challenge-response from the PC doesn't
help either. Without a protected secret, malicious firmware can answer every check just as ours
does, or display whatever ours would display.

**Rule of thumb:** a keyboard can only prove it is genuine with a secret that no newly flashed
firmware can read. On this chip, only two things can enforce that: the bootloader (it runs first and
decides what gets flashed) and APPROTECT (it decides whether a debugger can read flash).

---

## 3. Options

### 3.1 Your idea, done properly: bonds become the tamper seal

With **(a)** a bootloader that accepts only signed firmware and has no readback, and **(b)**
APPROTECT enabled, the only way left to run other code is ERASEALL (A6). ERASEALL wipes the LTKs.
After that:

- your computers refuse the keyboard. It can't complete encryption with the stored LTK, and Linux,
  macOS and Windows show it as disconnected or ask to pair again;
- the left half won't link to the right half (and the reverse), because the split bond is gone too.

This *is* the "it won't connect anymore" signal you asked for. It is cryptographic: BLE encryption
with the bonded LTK is a challenge-response that proves the keyboard holds the key, so no extra
protocol is needed. What you have to do: **never casually re-pair a keyboard that asks for it.** If
it needs pairing again and you didn't do anything, assume it has been tampered with.

### 3.2 Signed-firmware bootloaders for the nRF52840

| Option | What it gives | What it doesn't stop | Effort and risk | Workflow impact |
|---|---|---|---|---|
| **Adafruit bootloader ≥ 0.10.0 with `SIGNED_FW=1`** ([PR #375](https://github.com/adafruit/Adafruit_nRF52_Bootloader/pull/375), merged Jan 2026, [releases](https://github.com/adafruit/Adafruit_nRF52_Bootloader/releases)) | Serial (USB CDC) and BLE OTA DFU accept only packages signed with your P-256 key (`adafruit-nrfutil keys --gen-key`, `dfu genpkg --key-file`). **UF2/MSC is compiled out**, so A1 (`CURRENT.UF2`) and A2/A3 are closed. 0.11.0 also uses ACL to write-protect the MBR and bootloader against the app ([PR #384](https://github.com/adafruit/Adafruit_nRF52_Bootloader/pull/384)). Same memory layout and MBR/S140 arrangement as now, so `memory.x` stays the same. | It checks signatures at **update time**, not at every boot, so it relies on APPROTECT to stop SWD writes. The feature is new and "disabled by default", so it has had little testing. **Never build with `FORCE_UF2=1`** (the README calls it insecure: UF2 skips the signature check). | Low to medium. Built with `make BOARD=... SIGNED_FW=1 SIGNED_FW_QX=... SIGNED_FW_QY=...`. It can be installed with the existing UF2 self-update (`update-*.uf2`) without a probe, but if it fails to boot you need SWD to recover. **Test on a spare nice!nano first.** Check the SoftDevice ID in `INFO_UF2.TXT` for `--sd-req`. | `./hil flash` becomes: build `.hex`, then `adafruit-nrfutil dfu genpkg --application fw.hex --key-file ~/keys/kb.pem --sd-req <id> pkg.zip`, then `adafruit-nrfutil dfu serial --package pkg.zip -p /dev/ttyACMx --singlebank`. The `boot` command should write GPREGRET `0x4E` (serial-only DFU) instead of `0x57`. Double-tap reset still enters DFU, just without a drive. Keep the signing key off the keyboard and ideally behind a passphrase. |
| **embassy-boot + embassy-boot-nrf** with `ed25519-salty` ([book](https://embassy.dev/book/dev/bootloader.html), [docs.rs](https://docs.rs/embassy-boot/)) | All in Rust, Ed25519. The *running app* receives the update into a DFU partition and calls `verify_and_mark_updated`. The bootloader swaps and supports rollback. | Same "verify at update time" model, so it needs APPROTECT too. You must write the transport yourself (USB/BLE DFU in our app). | High. New flash layout (bootloader at `0x0`, partitions, state). The Adafruit bootloader, MBR and SoftDevice area go away, and everything is installed over SWD. No drag-and-drop. If the app breaks, double-tap recovery is gone unless you build it. | A large change. Only worth it if you want a pure-Rust stack. |
| **MCUboot** ([docs](https://docs.mcuboot.com/)) / **Nordic Secure Immutable Bootloader** (nRF Connect SDK, [firmware update guide](https://developer.nordicsemi.com/nRF_Connect_SDK/doc/2.1.2/nrf/ug_fw_update.html)) | Mature, signed images (Ed25519 or ECDSA), verified at boot. | They are built around Zephyr/NCS, so using them with an Embassy app is not a natural fit. | High | Large |

**Recommendation:** use the Adafruit bootloader with `SIGNED_FW=1`. It is the smallest change from
what you have and keeps the layout, double-tap-to-DFU and BLE OTA. Embassy-boot is the purist
option for later.

### 3.3 APPROTECT (lock the debug port)

- In our firmware, set `config.debug = embassy_nrf::config::Debug::Disallowed;` before
  `embassy_nrf::init`. Embassy then writes `UICR.APPROTECT = Enabled` once and resets. On build code F
  chips, protection is also on at every reset unless firmware turns it off
  ([Nordic DIF](https://docs.nordicsemi.com/bundle/ps_nrf52840/page/dif.html),
  [NCS AP-Protect](https://developer.nordicsemi.com/nRF_Connect_SDK/doc/latest/nrf/security/ap_protect.html)).
  The bootloader doesn't open it (Adafruit
  [issue #325](https://github.com/adafruit/Adafruit_nRF52_Bootloader/issues/325): it never writes
  `APPROTECT.DISABLE`).
- **Gives:** no SWD read or write (A4). The only way left is ERASEALL, which wipes everything (A6),
  so the tamper seal from 3.1 works.
- **Doesn't stop:** UF2 (A1/A2) on its own. Alone it is almost worthless today; combined with 3.2 it
  is essential. Older silicon (build code before F) can be glitched (A7). Check your chips: FICR
  `INFO.VARIANT` at `0x10000104` (the letter in bits 15:8 is the build code; this is what Embassy
  reads too).
- **Risk:** after locking, your own recovery is ERASEALL + reflashing bootloader, SoftDevice and app
  over SWD. **Have a working probe first** (a Raspberry Pi Pico with `debugprobe` firmware works;
  `probe-rs erase --allow-erase-all` or `nrfjprog --recover`). Locking happens only when the app
  first runs with `Disallowed`. A dev build with `Allowed` written afterwards can't unlock it again
  (UICR bits can only be set back by an erase).

### 3.4 ACL (nRF52840 memory access control)

8 page-aligned regions, read and/or write protection, **write-once until the next reset**
([Nordic ACL](https://docs.nordicsemi.com/bundle/ps_nrf52840/page/acl.html)).

- Set by **our app** it protects nothing against an attacker, because their app runs instead of
  ours and simply doesn't set it.
- Set by the **bootloader** it could hide a secret page from every app that the bootloader doesn't
  trust (a measured-boot style attestation key). That means patching the bootloader to hash or check
  the app on every boot. This is only interesting if you later want an attestation key that even a
  correctly signed but buggy app can't leak. Not needed for this threat model once 3.2 + 3.3 are done.

### 3.5 Things that sound good but don't help (alone)

- **Encrypting the bonds with a key in flash:** the key is readable together with them.
- **Encrypting the bonds with a PIN typed at power-on:** it stops the offline dump (A1) only if the
  PIN is long, but a reflashed keylogger (A2) just records the PIN. A short PIN can be brute-forced
  from the dump.
- **Firmware hash on the OLED, or a "version" query from the PC:** malicious firmware prints or
  answers whatever ours would.
- **Host challenge-response over Raw HID or GATT** with a secret in our flash: same problem until
  3.2 + 3.3. After that it adds little, because the LTK already does it (3.1). It could still be a
  nice explicit check: "kbnotify verify" says *genuine* or *not genuine*, instead of you having to
  interpret a disconnect.
- **Secrets only in RAM:** bonds must survive power-off, so this doesn't work for LTKs.
- **USB-only mode at work:** removes over-the-air sniffing (A1's second half), but a reflashed USB
  keyboard logs and injects just as well (A2). Useful only as a partial measure.

### 3.6 Non-technical measures (work today, cost nothing)

1. **Take it with you or lock it away.** Both halves matter (A5). In a shared office a drawer with a
   lock is enough for this threat model.
2. **Tamper-evident marks:** glitter nail polish over a case screw, or a fragile sticker across the
   reset button. Photograph it once. It is crude, but it catches A2/A4 if they need the case open
   or the button pressed.
3. **Treat "needs pairing again" as an alarm** (after the medium tier, this becomes the main signal).
4. **If you suspect a dump (A1): rotate the keys.** Forget every profile on the keyboard *and* on
   each computer, forget the split bond (hold the outermost top key at boot on both halves), and pair
   again. This makes a stolen LTK useless.
5. **Pair new computers at home,** not in a busy place (Just Works has no MITM protection during
   pairing).

---

## 4. Plan

### Tier 1: now (hours, no bricking risk)

1. **Physical:** lock the keyboard away or take it with you whenever you leave it in a shared space.
   Rotate all bonds once if it has already been left unattended (3.6 point 4).
2. **Gate the `boot` command** (closes A3): accept `boot` only while a physical key is held (for
   example the same "outermost top key" convention as the split-bond reset, checked in
   `fw-nrf/src/logport.rs` under `Command::Bootloader`), or send daily-use builds with `LOG=0`.
   `./hil flash left` would then print `hold <key> and…` or fall back to double-tap. The `p/r/k`
   injection and `t` trace (which prints every report, i.e. keystrokes, to the tty) also only belong
   in dev builds.
3. **Check the chip revision** of each nice!nano: add the FICR `INFO.VARIANT` build code to the
   `s` status line, or read it over SWD. Build code F or later is where APPROTECT is hardened against
   the known glitch.
4. **Get an SWD probe working** on a spare nice!nano (`probe-rs info`, then flash the stock
   bootloader hex back). This is the safety net for tier 2.

Tier 1 does **not** close A1/A2 against someone at your desk. Only physical control does, until tier 2.

### Tier 2: medium (a weekend; closes A1 to A5)

Prepared: [signed-bootloader.md](signed-bootloader.md) has the step-by-step procedure for moving a
keyboard to it. `fw-nrf/build-bootloader`, `fw-nrf/build-uf2` (`.zip`, `SIGNED=1`, `LOCK=1`) and `./hil
flash` are ready for it.

1. On a **spare** nice!nano: build Adafruit bootloader ≥ 0.10.0 (preferably 0.11.0, which adds ACL
   self-protection) with `BOARD=nice_nano SIGNED_FW=1 SIGNED_FW_QX=… SIGNED_FW_QY=…` (keys from
   `adafruit-nrfutil keys --gen-key kb.pem` and `--show-vk code`). Install it with the UF2
   self-update (`update-*.uf2`) or over SWD. Check that it enumerates as CDC only, accepts a signed
   package and **rejects** an unsigned one and one signed with a different key.
2. In the firmware: `config.debug = Debug::Disallowed` (maybe behind a `LOCK=1` build flag so dev
   boards stay debuggable), and `boot` → GPREGRET `0x4E`.
3. Change `./hil flash`: `uf2conv`/drive copy → `adafruit-nrfutil dfu genpkg` + `dfu serial`. Keep
   the old path for unlocked dev boards. The `.hil-halves` serial check can stay (the bootloader's
   USB serial number is unchanged).
4. Roll out to both halves of the keyboard you take to work. Then **rotate all bonds** (new LTKs:
   any earlier copy of `CURRENT.UF2` holds the old ones).
5. Note in `README.md`: "unexpected re-pair request means tampering; don't re-pair, reflash
   (ERASEALL + bootloader + app) and rotate."

After tier 2, someone at your desk can't read the keys and can't install unsigned firmware. Their
only options (erase the chip, or swap the MCU) destroy the bonds, so the keyboard stops connecting,
which is the behaviour you asked for.

### Tier 3: strong or nice-to-have (optional)

1. **Host-side watcher** (a small `kbnotify` subcommand or a shell script next to `./hil pc`): warn
   when a device named like the keyboard asks to pair, or when the bonded keyboard hasn't connected
   for more than N minutes while the PC is in use. It turns the silent "won't connect" into an alert.
2. **Explicit attestation:** a per-device Ed25519 or HMAC key created at provisioning, kept in the
   storage region, and a challenge-response command over Raw HID (`kbnotify verify`). It is only
   meaningful after tier 2. It gives a clear "genuine" answer even over USB, where there is no LTK.
3. **Personal boot phrase on the OLED:** a short secret phrase you choose, kept in flash and shown at
   boot by our firmware. After tier 2 an attacker can't read it, so if the phrase is missing or wrong
   you know straight away, even without a computer. Cheap to add.
4. **Bootloader-enforced ACL secret** (3.4) or a full verified boot (embassy-boot or a patched
   Adafruit bootloader that checks the app signature at every boot). This defends against a signed
   but compromised build. Only worth it if your signing key or build machine is in the threat model.
5. Shield the SWD pads or fill them with epoxy, and use a case that needs a tool to open (raises the
   cost of A6/A8).

---

## 5. Sources

- Adafruit nRF52 bootloader: [repo](https://github.com/adafruit/Adafruit_nRF52_Bootloader),
  [`ghostfat.c` (CURRENT.UF2 readback, UF2 write path)](https://github.com/adafruit/Adafruit_nRF52_Bootloader/blob/master/src/usb/uf2/ghostfat.c),
  [`uf2cfg.h` (flash range, bootloader family ID)](https://github.com/adafruit/Adafruit_nRF52_Bootloader/blob/master/src/usb/uf2/uf2cfg.h),
  [changelog](https://github.com/adafruit/Adafruit_nRF52_Bootloader/blob/master/changelog.md),
  [PR #375 signed firmware](https://github.com/adafruit/Adafruit_nRF52_Bootloader/pull/375),
  [PR #384 ACL protection](https://github.com/adafruit/Adafruit_nRF52_Bootloader/pull/384),
  [issue #325 APPROTECT on rev 3](https://github.com/adafruit/Adafruit_nRF52_Bootloader/issues/325),
  [releases](https://github.com/adafruit/Adafruit_nRF52_Bootloader/releases)
- Nordic nRF52840: [debug and access port protection (DIF)](https://docs.nordicsemi.com/bundle/ps_nrf52840/page/dif.html),
  [ACL](https://docs.nordicsemi.com/bundle/ps_nrf52840/page/acl.html),
  [NCS AP-Protect guide](https://developer.nordicsemi.com/nRF_Connect_SDK/doc/latest/nrf/security/ap_protect.html),
  [nRF Sniffer: sniffing bonded devices with a known LTK](https://docs.nordicsemi.com/bundle/nrfutil/page/nrfutil-ble-sniffer/guides/common_sniffing_actions.html),
  [NCS firmware update / secure boot](https://developer.nordicsemi.com/nRF_Connect_SDK/doc/2.1.2/nrf/ug_fw_update.html)
- APPROTECT glitch: [LimitedResults, "nRF52 Debug Resurrection"](https://limitedresults.com/2020/06/nrf52-debug-resurrection-approtect-bypass/),
  [CVE-2020-27211](https://nvd.nist.gov/vuln/detail/CVE-2020-27211),
  [CNX Software summary](https://www.cnx-software.com/2020/06/15/nordic-semi-nrf52-wisocs-are-susceptible-to-debug-resurrection-using-approtect-bypass/)
- Embassy: [bootloader book (ed25519-salty)](https://embassy.dev/book/dev/bootloader.html),
  [embassy-boot docs](https://docs.rs/embassy-boot/), `embassy-nrf 0.11.0` `src/lib.rs` (the
  `Debug` config and "Setup debug protection", in `.cargo-home/registry/...`)
- MCUboot: [docs.mcuboot.com](https://docs.mcuboot.com/)
- This repo: `fw-nrf/memory.x`, `ble/src/storage.rs`, `fw-nrf/src/main.rs` (`NrfConfig::default()`),
  `fw-nrf/src/logport.rs` (`Command::Bootloader`), `core/src/console.rs`, `hil` (`flash`)
