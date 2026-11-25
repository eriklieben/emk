# Signed bootloader

This is how to move both halves of a keyboard from the stock Adafruit UF2 bootloader 0.6.0 to
Adafruit bootloader **0.11.0 built with `SIGNED_FW=1`**. After the switch, a half accepts only firmware
signed with our key, has no `NICENANO` drive and no `CURRENT.UF2` readback, and write-protects its
MBR and bootloader with ACL while our firmware runs. The reasons are in
[security-review.md](security-review.md) (3.2 and tier 2).

Do the left half first, use it for a few days, then do the right half. Lock the debug port (`LOCK=1`)
only at the very end, and only if you have a working SWD probe.

The examples use the keyboard name `erik_kbd` (the default of `./hil`, `KB_NAME`). For another name,
pass it to `fw-nrf/build-uf2` and set `KB_NAME` to the same name for `./hil`. `<left serial>` and
`<right serial>` stand for each nice!nano's USB serial number (16 hex digits) as `./hil` prints it.

Everything below is prepared and checked without hardware (section 7 lists what that means). Nothing
in this document has been tried on a nice!nano yet.

---

## 1. What changes, checked in the 0.11.0 source

All of this comes from the tag `0.11.0` (commit `c67f0bc`) in `build/bootloader`, or from 0.6.0 where it
says so.

| | UF2 bootloader 0.6.0 (now) | Signed 0.11.0 (`SIGNED_FW=1`) |
|---|---|---|
| USB | `239a:00b3`, CDC + mass storage, interfaces `:020201:0a0000:080650:` | `239a:00b3` (same PID: `USB_DESC_CDC_ONLY_PID` = `USB_DESC_UF2_PID` on nice_nano), **CDC only**, `:020201:0a0000:` |
| USB serial | `%08X%08X` of FICR DEVICEID | the same string (written differently, same result), so `.hil-halves` still matches |
| Drive, `INFO_UF2.TXT`, `CURRENT.UF2` | yes | **gone**: `CFG_TUD_MSC` is undefined (`src/usb/tusb_config.h`), so `msc_uf2.c` compiles to nothing and the linker drops `ghostfat.c`. The built image has no `CURRENT UF2`, `INFO_UF2TXT` or `NICENANO`; the unsigned build of the same tag has all three |
| Serial DFU (USB CDC) | accepts anything (CRC16 only) | accepts only packages whose init packet carries a valid ECDSA P-256/SHA-256 signature by our key, and whose image matches the signed length and SHA-256 (`src/dfu_init.c`) |
| BLE OTA DFU | yes | still compiled in, same signature check. Entered only with GPREGRET `0xA8`/`0xB1` from the app (ours never writes those); nice_nano has no DFU buttons and `DEFAULT_TO_OTA_DFU` is off |
| Bootloader self-update by UF2 | yes (family `0xd663823c`) | gone with the drive. Only a **signed** bootloader package over DFU (device revision 52840) or SWD |
| ACL | no | before jumping to the app: MBR `0x0..0x1000` and bootloader + MBR params + settings `0xF4000..0x100000` write-protected until the next reset (`bootloader_util_app_start`) |
| APPROTECT | never touched | never touched (no `APPROTECT` anywhere in `src/` or `lib/sdk11`) |
| Version for the app | `TIMER2.CC[0]` = `0x000600` | `TIMER2.CC[0]` = `0x000B00`. Our firmware logs it: `boot: bootloader 0.11.0, ...` |

**How DFU mode is entered** (`src/main.c`, `check_dfu_mode`):

| How | 0.6.0 | Signed 0.11.0 |
|---|---|---|
| Double-tap reset (2nd tap within 500 ms) | drive + CDC, no timeout | CDC, no timeout |
| GPREGRET `0x57` (`DFU_MAGIC_UF2_RESET`), our `boot` command | drive + CDC | **CDC** (no MSC in the build). If USB doesn't enumerate within 3 s it starts the app again; once enumerated it waits |
| GPREGRET `0x4E` (`DFU_MAGIC_SERIAL_ONLY_RESET`), `boot` with `SIGNED=1` | CDC only | CDC, same 3 s rule |
| GPREGRET `0xA8` / `0xB1` | BLE OTA | BLE OTA (signed packages only) |
| GPREGRET `0x6D` | skip DFU and double-tap delay | same |
| No valid app | DFU, no timeout | serial DFU, no timeout |

So `0x57` works with both bootloaders: the firmware that's on the keyboard now can already be flashed
through the signed bootloader. `SIGNED=1` (feature `signed-boot`, `0x4E`) only makes the intent explicit.
I picked a build feature over runtime detection because the version in `TIMER2.CC[0]` doesn't say
whether the bootloader is signed, and nothing else simple does. Since both values work with both
bootloaders, a wrong guess costs nothing.

**Flash layout:** this stays the same. The bootloader is at `0xF4000` (38 KB), its config at
`0xFD800`, MBR params at `0xFE000` and settings at `0xFF000`. MBR + S140 6.1.1 (size `0x26000`, read
from the SoftDevice hex) sit below the app at `0x26000`. `fw-nrf/memory.x` doesn't change. The
reserved app data is 40 KB (`DFU_APP_DATA_RESERVED 10*4096`). Single-bank serial DFU erases exactly
the image size from `0x26000`, so our bonds at `0xE4000..0xEC000` survive a normal flash. (The left
half is 405 KB and ends at `0x88FC4`.)

**Things to know:**

- **A refused package still costs the app.** The start packet erases the app area *before* the
  init packet and its signature arrive. After a refusal the half sits in serial DFU with no app
  (bonds intact), until you flash a good package. `./hil flash` therefore runs `fw-nrf/dfu-verify.py`
  first and won't send a package the bootloader would refuse.
- **Anyone can erase the app and part of the bonds** over USB. The start packet's size is not
  signed, and it can be up to `0xC4000` (that erases up to `0xEA000`). This is denial of service,
  not a takeover: nothing unsigned gets installed, and lost bonds trigger the re-pair alarm (section 6).
- **Every package you ever sign stays installable** (no version check, `app_version` is
  `0xFFFFFFFF`). Don't sign builds you wouldn't want on the keyboard, for example test builds with
  the `t` report trace.
- **`--sd-req` must be exactly `0x00B6`** (S140 6.1.1, read from the SoftDevice hex: FWID at
  `0x300C`). One entry gives a 116-byte init packet, which the serial transport passes on whole. Two
  entries (e.g. `0x00B6,0xFFFE`) make it 118 bytes, the transport keeps 120, and the 106-byte
  extension overruns the bootloader's 104-byte `m_extended_packet` buffer. `0xFFFE` ("any
  SoftDevice") isn't needed and is meant for development only. `fw-nrf/build-uf2` always uses `0x00B6`.
- The 0.6.0 self-update copies the new bootloader to `0xA3000..0xAD000` first (it has
  `USER_FLASH_END` hard-coded to `0xAD000`). That area overwrites any app larger than 500 KB, but not
  the bonds. Ours are 405 KB (left) and 301 KB (right). `dfu-verify.py` checks this.
- The settings page is not in the update UF2, so the app that 0.6.0 marked valid (bank_0 valid,
  CRC 0) keeps booting under 0.11.0.

---

## 2. One-time setup

### 2.1 Tools (user venv, nothing system-wide)

```sh
python3 -m venv ~/.local/share/emk-venv
~/.local/share/emk-venv/bin/pip install cryptography intelhex \
  "git+https://github.com/adafruit/Adafruit_nRF52_nrfutil.git@7fdfe15feee5f304fb7d9b031721dcefa1f72b58"
```

Use that commit, not PyPI. `adafruit-nrfutil 0.5.3.post16` on PyPI is broken for signing on Python 3:
`keys --gen-key` crashes ("write() argument must be str, not bytes") and `load_key` uses
`c.encode('hex')`. The commit (upstream master, tag `0.5.3.post17`+) fixes both. It still reports its
version as `0.5.3.post16`.

The bootloader toolchain is the podman image `localhost/emk-blbuild` (Debian bookworm,
arm-none-eabi-gcc 12.2.1; upstream CI uses 12.3). `fw-nrf/build-bootloader` creates it on first use.

### 2.2 The signing key

Generate it once:

```sh
umask 077; mkdir -p ~/.config/emk/keys
~/.local/share/emk-venv/bin/adafruit-nrfutil keys --gen-key ~/.config/emk/keys/emk-fw.pem   # mode 600
~/.local/share/emk-venv/bin/adafruit-nrfutil keys --show-vk hex ~/.config/emk/keys/emk-fw.pem
```

```text
Qx: <64 hex digits>
Qy: <64 hex digits>
```

**No passphrase is possible.** nrfutil writes the key unencrypted and loads it with
`SigningKey.from_pem()` without a password. To keep it safe:

- Keep it out of the repo (it is), off the keyboard, and out of backups that other people can read.
- Better: store it encrypted, e.g. `age -p -o ~/.config/emk/keys/emk-fw.pem.age emk-fw.pem` (or
  `gpg -c`). Delete the plain file. Before a signing session, decrypt it to
  `$XDG_RUNTIME_DIR/emk-fw.pem` (tmpfs) and set `EMK_KEY=$XDG_RUNTIME_DIR/emk-fw.pem`. Every script
  here honours `EMK_KEY`.
- **Make one offline backup** (an encrypted USB stick in a drawer, or paper: the PEM is 241 bytes).
  If the key is lost, nothing can be flashed any more except with an SWD probe (section 5).
- Whoever has the key can sign firmware that the keyboard accepts. A stolen key brings back the
  situation we have today.

### 2.3 Access to the bootloader's serial port

`./hil` needs read and write access to the bootloader's tty, and ModemManager must leave it alone (its
probing would break the DFU protocol). Add this next to the log-port rule from README.md:

```sh
sudo tee /etc/udev/rules.d/71-nicenano-dfu.rules <<'EOF'
SUBSYSTEM=="tty", ATTRS{idVendor}=="239a", ATTRS{idProduct}=="00b3", TAG+="uaccess", ENV{ID_MM_DEVICE_IGNORE}="1"
EOF
sudo udevadm control --reload && sudo udevadm trigger
```

USBGuard: `./hil` already allows `239a:*` devices while it waits (`./hil allow`).

### 2.4 Build and check (no hardware)

```sh
./fw-nrf/build-bootloader                 # → target/bootloader/, checks the key inside it
BLE=1 ./fw-nrf/build-uf2 erik_kbd         # → target/fw-nrf-left-ble-erik_kbd.{uf2,hex,zip}
BLE=1 SIDE=right ./fw-nrf/build-uf2 erik_kbd
```

Success looks like this:

- `build-bootloader` ends with `the key compiled into the bootloader is the key file's public key`,
  `all checks passed` (twice: the key, then the rollback package), and lists
  `update-nice_nano_bootloader-0.11.0-signed_nosd.uf2` (75264 bytes, 147 blocks of family
  `0xd663823c`, blocks only at `0x0` (MBR, skipped by 0.6.0), `0xF4000..0xFD900` and UICR, with
  UICR `0x10001014 = 0xF4000`, `0x10001018 = 0xFE000`, board ID `0x239A00B3` = what 0.6.0 checks).
- `build-uf2` prints `...zip: signed, verified (...verify.txt)`. The `.verify.txt` ends with
  `the bootloader would accept this package` and `all checks passed`, including `starts at 0x26000`
  and `below the bond storage`.

To see that the check really rejects bad packages (done once already, see section 7):

```sh
N=~/.local/share/emk-venv/bin/adafruit-nrfutil; T=$XDG_RUNTIME_DIR/throwaway
$N keys --gen-key $T.pem
$N dfu genpkg --dev-type 0x0052 --sd-req 0x00B6 --application target/fw-nrf-left-ble-erik_kbd.hex --key-file $T.pem $T.zip
~/.local/share/emk-venv/bin/python3 -I fw-nrf/dfu-verify.py package $T.zip --expect-reject \
  --elf target/bootloader/nice_nano_bootloader-0.11.0-signed.elf --uf2 target/bootloader/update-nice_nano_bootloader-0.11.0-signed_nosd.uf2
# → FAIL bootloader: ECDSA P-256 signature ... / ok the bootloader would refuse this package, as expected
```

### 2.5 SWD probe (get it working before the right half and before any `LOCK=1`)

You don't need a probe for the switch itself, but you do need one for every recovery that the
bootloader can't do (section 5). A Raspberry Pi Pico with the
[debugprobe](https://github.com/raspberrypi/debugprobe) firmware (`debugprobe_on_pico.uf2`) works:

- Wiring: Pico GP2 → SWCLK, GP3 → SWDIO, GND → GND on the nice!nano's SWD pads (underside). Power
  the nice!nano from USB or its battery, not from the probe.
- Install [probe-rs](https://probe.rs) as a user (`cargo install probe-rs-tools` or their install
  script), plus the udev rules from their docs.
- Test on a **spare** nice!nano: `probe-rs info --protocol swd` should show the nRF52840 and its
  AHB-AP. Then do one full restore (section 5.3) on the spare, so you know the procedure works.

Ideally, also try the signed bootloader on the spare first (sections 3.2 to 3.5, including the
negative test in 3.5). The security review recommends this, and it is still the safest order.

---

## 3. Left half (erik_kbd left, serial `<left serial>`)

The left half has the log port, so `./hil` restarts it into its bootloader by itself.

### 3.1 Baseline: new firmware, still on 0.6.0

```sh
BLE=1 ./fw-nrf/build-uf2 erik_kbd
./hil flash left
./hil log 3 | grep 'boot:'
```

Success:

- `./hil flash` prints `bootloader: uf2 (serial <left serial>, port /dev/ttyACMx)`,
  `flashed target/fw-nrf-left-ble-erik_kbd.uf2`, `keyboard back after Ns`. `.hil-halves` now
  says `erik_kbd left <left serial> uf2`.
- The log shows `boot: bootloader 0.6.0, entry UF2, debug port open`.
- Optional: `./hil bootinfo left` prints `ID_USB_INTERFACES=:020201:0a0000:080650:` and
  `INFO_UF2.TXT` (`UF2 Bootloader 0.6.0 ... Board-ID: nRF52840-nicenano ... SoftDevice: S140 version
  6.1.1`). Press reset once to leave.

Way back: this is the normal UF2 flow. Any `.uf2` (ours or ZMK) works as before.

### 3.2 Install the signed bootloader (the point of no easy return)

This step is the risky one. If the new bootloader doesn't start, only an SWD probe brings the half
back. The checks in 2.4 must have passed.

```sh
./hil flash left target/bootloader/update-nice_nano_bootloader-0.11.0-signed_nosd.uf2
```

`./hil` copies it onto the `NICENANO` drive. 0.6.0 checks the board ID, stores the image at
`0xA3000`, has the MBR copy it to `0xF4000` and starts the new bootloader. That one finds the app
valid and starts it. Don't unplug or reset while it copies: the LED flickers for a few seconds.

Success:

- `flashed ...update-...uf2`, then `keyboard back after Ns` (the app runs on 0.11.0).
- `./hil log 3 | grep 'boot:'` shows **`boot: bootloader 0.11.0, entry UF2, debug port open`**.
  The version comes from the new bootloader, so this line proves the switch worked.

If it goes wrong:

- `cp` fails or the drive disappears early, and after a double-tap you still get a `NICENANO` drive
  whose `INFO_UF2.TXT` says 0.6.0: the update was not applied. Run it again.
- The keyboard is back, but the log still says `0.6.0`: same, run it again.
- Nothing appears on USB at all, even after a double-tap: the bootloader is broken. Restore it with
  the SWD probe (5.3). The bonds survive that if you skip the full erase (the chip is not locked yet).

### 3.3 Check the bootloader

```sh
./hil bootinfo left
```

Success: `bootloader: signed (serial <left serial>, ...)`, `ID_VENDOR_ID=239a`,
`ID_MODEL_ID=00b3`, **`ID_USB_INTERFACES=:020201:0a0000:`** (no `08`),
`no drive, CDC only: signed-only bootloader`. `lsblk -o NAME,LABEL` shows no `NICENANO`. A double-tap
reset gives the same result. Press reset once to start the firmware again.

### 3.4 First signed flash

```sh
SIGNED=1 BLE=1 ./fw-nrf/build-uf2 erik_kbd
./hil flash left
```

`./hil` sees the signed bootloader, picks `target/fw-nrf-left-ble-erik_kbd.zip`, checks it with
`dfu-verify.py` and runs
`adafruit-nrfutil dfu serial --package <zip> -p /dev/ttyACMx -b 115200 --singlebank` (the baud rate
doesn't matter over USB CDC; `--singlebank` because the bootloader is built single-bank).

Success:

- nrfutil's progress bar ends with `Device programmed.`, then `flashed ...zip`,
  `noting: erik_kbd left now has the signed bootloader (was uf2)`, `keyboard back after Ns`.
  `.hil-halves`: `erik_kbd left <left serial> signed`.
- Log: `boot: bootloader 0.11.0, entry serial DFU (signed), debug port open`.
- `./hil test` passes, and the computer is still connected (the bonds survived).

If it goes wrong:

- `serial DFU failed`: the half stays in its bootloader. Double-tap reset and run `./hil flash left`
  again.
- `no access to /dev/ttyACMx`: the udev rule (2.3) is missing.
- The bootloader timed out before nrfutil started (the 3 s rule while USBGuard was still blocking):
  double-tap and run `./hil flash left` again. With a double-tap there is no timeout.

### 3.5 Optional: watch the bootloader refuse a bad package

This proves on real hardware that the signature check works. It costs the app, which you then flash
again. `./hil` refuses to send such a package, so call nrfutil yourself:

```sh
./hil bootinfo left                 # note the port, leave it in the bootloader
~/.local/share/emk-venv/bin/adafruit-nrfutil dfu serial --package $XDG_RUNTIME_DIR/throwaway.zip \
  -p /dev/ttyACMx -b 115200 --singlebank
```

Expected: nrfutil fails instead of printing `Device programmed.` The bootloader resets on the
failed check (`APP_ERROR_CHECK` → `NVIC_SystemReset`). The half comes back as the bootloader (CDC
only), with no app. Then run `./hil flash left`
with the real package, and the keyboard reconnects without pairing again.

### 3.6 Use it

Use the keyboard for a few days with the left half signed and the right half on 0.6.0. Daily
flashing: `SIGNED=1 BLE=1 ./fw-nrf/build-uf2 erik_kbd && ./hil flash left`. The right half
still gets its `.uf2`.

---

## 4. Right half (erik_kbd right, serial `<right serial>`)

The right half has no log port, so `./hil` asks you to **double-tap reset** on it each time.

1. Baseline (as in 3.1): `BLE=1 SIDE=right ./fw-nrf/build-uf2 erik_kbd && ./hil flash right`,
   then double-tap. Success: `bootloader: uf2 (serial <right serial> ...)`, `flashed ...uf2`, and
   the halves link (`./hil wait 'split: linked'`).
2. Install the bootloader:
   `./hil flash right target/bootloader/update-nice_nano_bootloader-0.11.0-signed_nosd.uf2`, then
   double-tap. Success: the drive disappears, the half restarts and links to the left half again
   (`./hil wait 'split: linked' 30`). The right half's own version line is not visible without a
   log port, so check with step 3.
3. `./hil bootinfo right`, then double-tap. Success: `bootloader: signed`,
   `ID_USB_INTERFACES=:020201:0a0000:`. Press reset once.
4. `SIGNED=1 BLE=1 SIDE=right ./fw-nrf/build-uf2 erik_kbd && ./hil flash right`, then double-tap.
   Success: `Device programmed.`, `.hil-halves` says `signed`, the halves link, and `./hil test`
   passes.

The failure cases and ways back are the same as for the left half.

---

## 5. After both halves: lock, rotate, and recovery

### 5.1 Lock the debug port (optional, last)

Only do this when both halves have run the signed bootloader for a while **and** the probe (2.5)
has done a full restore on the spare. The lock makes SWD useless until a full chip erase, and that
erase also wipes the bonds, the SoftDevice, the bootloader and UICR. The way back is then section 5.3,
with re-pairing every computer.

```sh
LOCK=1 SIGNED=1 BLE=1 ./fw-nrf/build-uf2 erik_kbd              # → ...-locked.{uf2,hex,zip}
./hil flash left target/fw-nrf-left-ble-erik_kbd-locked.zip
LOCK=1 SIGNED=1 BLE=1 SIDE=right ./fw-nrf/build-uf2 erik_kbd
./hil flash right target/fw-nrf-right-ble-erik_kbd-locked.zip
```

The firmware writes `UICR.APPROTECT = Enabled` at its first start and resets once (Embassy,
`Debug::Disallowed`). Success: the left log shows `boot: ... debug port locked`. With the probe,
`probe-rs info` can no longer read the chip (only the CTRL-AP answers). Afterwards, normal builds
without `LOCK=1` are fine: Embassy can't set the UICR bits back to 1, so the lock stays.
`LOCK=1` outputs are named `-locked` on purpose, so `./hil flash` never picks them by default.

### 5.2 Rotate all bonds (both halves, every computer)

Any earlier `CURRENT.UF2` or raw dump holds the old LTKs. Do this after both halves are signed (and
locked, if you lock):

1. On each computer, remove the keyboard (`bluetoothctl remove <addr>`, or Windows/macOS settings).
2. On the keyboard, forget each used BT profile: select it, then press both profile keys at once.
3. Forget the split bond: hold Tab/Esc on the left half and `\` on the right half while resetting
   each, until the OLED says `PAIR / forgetting other half`.
4. Pair each computer again, at home, with the passkey. Check `./hil pc` and `./hil test`.

### 5.3 Recovery with an SWD probe

Use `--chip nRF52840_xxAA`. Check `probe-rs download --help`, because the option names change
between versions.

- **Bootloader broken, chip not locked** (e.g. step 3.2 went wrong): flash the bootloader back
  without erasing, which keeps the app and the bonds:
  `probe-rs download --chip nRF52840_xxAA --binary-format hex target/bootloader/nice_nano_bootloader-0.11.0-unsigned_nosd.hex`
  (plain UF2 bootloader, as before), or `...-signed_nosd.hex`. Then `probe-rs reset --chip nRF52840_xxAA`.
- **Chip locked, key lost, or after a tamper alarm**: full erase, then everything again.
  1. `probe-rs erase --chip nRF52840_xxAA --allow-erase-all` (ERASEALL through CTRL-AP: flash, UICR
     and RAM are gone, so the bonds are gone too).
  2. `probe-rs download --chip nRF52840_xxAA --binary-format hex target/bootloader/nice_nano_bootloader-0.11.0-signed_s140_6.1.1.hex`
     (MBR + S140 6.1.1 + bootloader + its UICR addresses), or `...-unsigned_s140_6.1.1.hex` for the
     plain UF2 bootloader. For the original stock bootloader, take
     `nice_nano_bootloader-0.6.0_s140_6.1.1.hex` from the
     [Adafruit 0.6.0 release](https://github.com/adafruit/Adafruit_nRF52_Bootloader/releases/tag/0.6.0).
  3. `probe-rs reset --chip nRF52840_xxAA`. The bootloader writes REGOUT0 = 3.3 V once and resets
     itself (`src/boards/boards.c`). With no app it waits in DFU.
  4. Flash the app: `./hil flash left|right` (`.zip` on signed, `.uf2` on UF2). If the bootloader
     kind changed, fix the 4th column in `.hil-halves` by hand first.
  5. Pair everything again (5.2). If the key was lost: generate a new one, rebuild the bootloader, and
     use the `-signed_s140_6.1.1.hex` built with it in step 2.

### 5.4 Going back to a UF2 bootloader without a probe

`build-bootloader` also makes `target/bootloader/rollback-to-uf2-nice_nano_bootloader-0.11.0.zip`.
This is a **signed bootloader package** (device revision 52840, `--sd-req 0x00B6`) containing the
plain 0.11.0 UF2 bootloader. The signed bootloader installs it like its own update, through the SDK11
bootloader-swap path (MBR `COPY_BL`), and that erases the app:

```sh
./hil bootinfo left       # note the port; the half waits in DFU
~/.local/share/emk-venv/bin/adafruit-nrfutil dfu serial \
  --package target/bootloader/rollback-to-uf2-nice_nano_bootloader-0.11.0.zip -p /dev/ttyACMx -b 115200 --singlebank
```

Success: after a reset the `NICENANO` drive is back (`INFO_UF2.TXT`: 0.11.0) with no app. Change
`signed` to `uf2` in `.hil-halves` by hand (otherwise `./hil` raises the tamper alarm, as it
should), then `./hil flash left` with the `.uf2`. Tried on a spare
board (section 7); try it on a spare again after rebuilding the bootloader.

---

## 6. Tamper alarm rules

Once both halves are signed (and ideally locked), the only way to run other code is a full erase or a
swapped MCU. Both destroy the bonds. So:

- **An unexpected request to pair again is an alarm.** Examples: the computer says the keyboard
  needs pairing, the keyboard shows `+` on a profile that you didn't forget, or the halves don't
  link (`Link?` key, `./hil wait 'split: linked'` times out). **Don't pair it again.** Unplug it,
  check the case seals (security-review 3.6), and recover it as in 5.3 (full erase, signed bootloader,
  app). Then rotate all bonds (5.2) from a clean computer.
- **`./hil` stops with `ALARM: ... had the signed bootloader and now shows a UF2 drive`**. Someone
  replaced the bootloader, or this is a different nice!nano with the same serial. Treat it the same
  way. (If you rolled it back yourself in 5.4, fix `.hil-halves`.)
- `./hil` refusing to flash because a package fails `dfu-verify.py` is not an alarm. Your build or
  your key doesn't match the bootloader.
- The bootloader version line is only a hint. It comes from `TIMER2.CC[0]`, which malicious firmware
  could fake. The real proof is the bond: the keyboard reconnects with its old LTK.

---

## 7. What was and wasn't verified

Verified without hardware (2026-11-15):

- The 0.11.0 source has the `nice_nano` board (`src/boards/nice_nano/`): `239A:00B3` for both modes,
  `NICENANO`, board ID `nRF52840-nicenano`, REGOUT0 3.3 V, no buttons.
- The signed build fits (33952 of 38912 bytes). Symbol `pk` (64 bytes at `0xFC1D4`) holds exactly the
  key file's Qx/Qy in the ELF, the `_nosd.hex` and the `update-*.uf2`. The image contains
  `uECC_verify`, `tc_sha256_*` and `bootloader_util_flash_protect` (ACL), and no MSC/UF2 code or
  strings.
- The update UF2 only touches what 0.6.0 accepts for a bootloader update (bootloader space and UICR
  with matching addresses; the MBR block is skipped), and its board ID matches 0.6.0's.
- `dfu-verify.py` replays the 0.11.0 checks (start packet, init packet as cut by the serial
  transport, `dfu_init_prevalidate`, `dfu_init_postvalidate`), with OpenSSL (via `cryptography`) as
  the verifier and python-ecdsa (as in nrfutil) as the signer. Our left and right packages and the
  rollback package pass. A package signed with a throwaway key, a flipped image byte, a flipped
  init-packet byte and an unsigned package are all refused. The throwaway package passes against its
  own key (so the check isn't just always failing).

Verified on hardware (2026-11-25), on a spare HW-954 nice!nano clone (nRF52840 `QIAAD0`, shipped
with UF2 bootloader 0.6.0 and S140 6.1.1), not on a keyboard half:

- The 0.6.0 → 0.11.0 signed self-update (3.2): `./hil flash` copied the update UF2, the app started
  again and logged `boot: bootloader 0.11.0`.
- The bootloader check (3.3): no drive after a double-tap or `./hil bootinfo`, CDC only
  (`ID_USB_INTERFACES=:020200:0a0000:`, so `00` rather than `01` in the CDC protocol field).
- A signed update (3.4): `Device programmed.`, the app started, `.hil-halves` noted `signed`.
- Refusing a package signed with a throwaway key (3.5): nrfutil failed, the bootloader reset into
  DFU without an app; the real package went on again afterwards.
- The rollback package (5.4): `Device programmed.`, then the `NICENANO` drive was back with
  `UF2 Bootloader 0.11.0`, no app; after changing `.hil-halves` back to `uf2`, `./hil flash` with
  the `.uf2` worked as before.

Not verified:

- The same on a keyboard half (a genuine nice!nano, which may have a different chip revision).
- Every probe-rs command (5.3): no probe was connected.
- That `Debug::Disallowed` locks these chips as described (it depends on the build code; check FICR
  `INFO.VARIANT` first, security-review tier 1 step 3).
