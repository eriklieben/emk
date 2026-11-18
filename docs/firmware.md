# Firmware: building, flashing, log port

Reference for `fw-nrf` (nice!nano v2, nRF52840, Embassy). The [README](../README.md) has the short
version.

## Build

```sh
BLE=1 ./fw-nrf/build-uf2 erik_kbd              # → target/fw-nrf-left-ble-erik_kbd.uf2
BLE=1 SIDE=right ./fw-nrf/build-uf2 erik_kbd   # → target/fw-nrf-right-ble-erik_kbd.uf2
./fw-nrf/build-uf2                                 # left half, USB only, for all three keyboards
LOG=0 ...                                          # without the USB log port
SIGNED=1 ...                                       # `boot` asks for serial DFU (signed bootloader)
LOCK=1 ...                                         # lock the debug port: read signed-bootloader.md first
```

Each build also writes a `.hex` and, if the signing key `~/.config/emk/keys/emk-fw.pem` exists, a
signed DFU package `.zip` for halves with the signed bootloader (Adafruit 0.11.0, `SIGNED_FW=1`:
no drive, signed firmware only). `./fw-nrf/build-bootloader` builds that bootloader;
[signed-bootloader.md](signed-bootloader.md) has the switch-over, the recovery and the tamper rules.

The name is what the computer sees (USB product name and Bluetooth name). It is a build-time
constant, one build per keyboard (`erik_kyb_grs`, `erik_kyb_blk`, `erik_kyb_wh`).

Not yet in the firmware: NKRO.

## USB interfaces

The left half on USB is one composite device (1209:0001), in this order, so the numbers of the
older interfaces never move:

| Interface | What |
|---|---|
| 0 | keyboard (boot protocol, 8-byte reports) |
| 1 | mouse (no report IDs) |
| 2, 3 | log port (CDC ACM, `usb-log` builds; `/dev/serial/by-id/usb-erik_*-if02`) |
| 4 (2 with `LOG=0`) | Raw HID: usage page 0xFF60, usage 0x61, 32-byte input and output reports, no report IDs (as QMK's) |

The app's `HardwarePort` and `./hil` find the log port as before (`-if0*`, USB vendor ID).

## Messages from the computer (Raw HID)

`kbnotify` (see the [README](../README.md#messages-from-the-computer)) writes 32-byte output
reports, format in `core/src/host.rs`: a message in up to 5 chunks (128 bytes of UTF-8 at most), or
a clear. The keyboard never answers. The way through the firmware:

1. **USB**: the Raw HID interface's interrupt OUT pipe and SET_REPORT requests (Windows'
   `HidD_SetOutputReport`) both end in `raw_hid_task` (`fw-nrf/src/main.rs`).
   **Bluetooth**: report ID 3 in the HID report map, an output report characteristic (Report
   Reference `03 02`, write and write without response) in `ble/src/hid.rs`; only from the
   selected profile's computer, once the link is encrypted.
2. A `kb_core::host::Reassembler` per source puts the chunks together; control characters are
   left out (tabs and line breaks become spaces).
3. The message goes into `LeftState::inbox` (`ble/src/notify.rs`): the left half's own status
   (`Status.notification`, not drawn on the left screen; the USB-only right half shows it), and the
   split link.
4. **Split link**: characteristic `8f0e0006-…` on the right half's split service, written without
   response by the left half: the message again as `kb_core::host` reports, with the left half's own
   message id. A message that comes while the halves are apart goes out when they link again, unless
   it is older than 15 s.
5. The right half's key loop puts it in its status (it wakes the screen), and the screen shows it
   for 15 s or until a clear (`kb_core::host::Shown`).

Adding Raw HID changed the Bluetooth report map: a computer paired before keeps the old one cached
and doesn't see the Raw HID collection. Forget the keyboard there, clear the profile and pair again.

Anything on a computer the keyboard is connected to (USB, or the profile's paired computer) can send
a message; it can only show text on the right screen, nothing else (no keys, no settings).

## Flash

`./hil flash left` restarts the left half into its bootloader through the log port by itself.
The right half has no log port: double-tap its reset button, then run `./hil flash right`. Without
`./hil`: double-tap reset on a half's nice!nano and copy its `.uf2` onto the `NICENANO` drive. On a
half with the signed bootloader `./hil flash` sends the signed `.zip` over serial DFU instead.

`./hil` remembers which nice!nano is which half by its USB serial (`.hil-halves`, local only) and
refuses to flash the wrong one.

USBGuard blocks new USB devices, including the bootloader that appears after a restart.
`./hil allow` allows the blocked keyboard and bootloader devices (temporary).

The UF2 bootloader is never overwritten, so a bad build can't brick a board: double-tap reset and
flash something else.

## Going back to ZMK

Double-tap reset and copy a ZMK `.uf2` onto the drive. Local backups of the ZMK builds are in
`backup/zmk/` (not in the repository). Then pair the halves again (below). On a half with the signed
bootloader there is no drive: ZMK would have to be signed too, see
[signed-bootloader.md](signed-bootloader.md).

## Pairing the halves again

The halves pair with each other by themselves the first time. After flashing the other half with
another firmware (ZMK, say), hold each half's outermost top key (Tab/Esc on the left, `\` on the
right) while resetting it, until the OLED says `PAIR / forgetting other half`. Do both halves.

The link stage shows at the bottom of the screens while it connects or when it failed; once linked
it hides itself after 4 seconds. The `Link?` key on the nav layer (V) shows it again.

While resting, the link between the halves runs at a 45 ms interval instead of 7.5 ms to save
power; the first key after a rest can take up to 45 ms.

## Log port and `./hil`

`usb-log` builds (the default) add a USB serial port next to the keyboard on the left half. It keeps
the last 4 KB of log lines, from boot on. Commands, one per line: `s` status, `d` the whole log
again, `t` trace reports, `m` mute (trace without sending), `k12` tap key position 12, `p12` / `r12`
press / release it. Studio (live keymap editing) and the app's Debug mode use the same port
([studio-protocol.md](studio-protocol.md)).

Access without root (and keep ModemManager off it); the second line is for `kbnotify` (Raw HID over
USB and Bluetooth: the HID device is `0003:1209:0001.*` on USB, `0005:1209:0001.*` over Bluetooth):

```sh
sudo tee /etc/udev/rules.d/70-erik-kyb.rules <<'EOF'
SUBSYSTEM=="tty", ATTRS{idVendor}=="1209", ATTRS{idProduct}=="0001", TAG+="uaccess", ENV{ID_MM_DEVICE_IGNORE}="1"
SUBSYSTEM=="hidraw", KERNELS=="*:1209:0001.*", TAG+="uaccess"
EOF
sudo udevadm control --reload && sudo udevadm trigger
```

`./hil` wraps it. Run `./hil` for the list:

| Command | What |
|---|---|
| `./hil flash left\|right [file]` | flash a half (see above) |
| `./hil bootinfo left\|right` | which bootloader a half has |
| `./hil allow` | allow blocked keyboard/bootloader devices in USBGuard |
| `./hil log [seconds]` | print the log |
| `./hil cmd s` | send a log-port command and print the reply |
| `./hil wait 'split: linked'` | wait until a log line matches |
| `./hil pc` | the PC's view: Bluetooth pairing/connection and input devices |
| `./hil test` | smoke test: halves linked, a muted key tap reaches the output |

## Pins

From ZMK's `corne` shield and the nice!nano Pro Micro map:

| What | Pins |
|---|---|
| Rows | P0.22, P0.24, P1.00, P0.11 |
| Left columns | P0.31, P0.29, P0.02, P1.15, P1.13, P1.11 |
| OLED | SDA P0.17, SCL P0.20 |
| OLED power | P0.13 |

## Size and speed

`./size-report` builds `size-probe` (the shared logic alone, bare metal) and writes
`target/size-report.json`; the app's chip panel shows it. [performance.md](performance.md) has
measurements; `./dev sh -c 'cd bench && cargo run --release'` re-runs the benchmark.
[renode.md](renode.md) looks at running the firmware in Renode.
