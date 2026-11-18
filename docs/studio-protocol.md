# Studio: live keymap editing

EMK's own studio, not Vial or ZMK Studio. The keymap (4 layers × 42 positions,
`core/src/keymaps/corne.rs`) and its combos (up to 16) can be edited while the
keyboard runs, saved to flash, reverted and reset. The firmware speaks a text protocol on the USB log
port. The emulator (`kb-emu`) has the same keymap, lock and protocol behind a
C ABI. The protocol is version 2; version 1 had keys only. Code:
`core/src/studio/` (logic, encoding, records, combos, guard, protocol),
`ble/src/keymap.rs` (flash), `fw-nrf/src/logport.rs` (port), `emu/src/studio.rs`
and the `kb_emu_studio_*` functions in `emu/src/lib.rs`.

## Key positions

```text
 0  1  2  3  4  5      6  7  8  9 10 11
12 13 14 15 16 17     18 19 20 21 22 23
24 25 26 27 28 29     30 31 32 33 34 35
         36 37 38     39 40 41
```

Layers: 0 `default`, 1 `symbols` (hold 36), 2 `nav` (hold 41), 3 `wm` (both:
a conditional layer). `StudioUnlock` is on nav, position 35.

## Action encoding

Every binding is one token without spaces. Reading ignores case. Writing is
canonical, so compare the text the keyboard sends back.

| Text | Action |
|---|---|
| `trans` | `&trans`: falls through to the next active layer below |
| `none` | `&none` |
| `caps` | `&caps_word` |
| `kp:<key>` | `&kp`: press a key |
| `mo:<n>` | `&mo`: layer `n` while held (`n` < 4) |
| `mt:<hold>/<tap>` | `&mt`: `hold` when held, `tap` when tapped (hold-preferred, 200 ms) |
| `sys:<name>` | `BtPrev BtNext BtClear OutToggle StudioUnlock ShowLink RadioPower` |
| `mm:<dir>` | move the mouse pointer while held: `left down up right` |
| `mb:<button>` | a mouse button: `left right middle` |
| `ms:<dir>` | scroll while held: `left down up right` (left/right is horizontal pan) |
| `mp` | precise (slow) mouse movement while held |
| `mk:<button>/<key>` | the mouse `button` once the pointer is active, otherwise `kp:<key>` (see below) |
| `mk:scroll_<dir>/<key>` | scroll `<dir>` once the pointer is active, otherwise `kp:<key>` |

The pointer is active from the first `mm:` press until the layer it was pressed on is
released. Holding nav and pressing S/D/F/G activates it; letting go of nav ends it. `mp` and
`ms:` don't activate it.

An `mk:` key is decided when it is pressed, and its release always matches the press: a click
that started as a click releases as a click, and a scroll as a scroll, even if nav went first.

The compiled nav layer has `mk:left/BSPC` and `mk:right/SPACE` on the thumbs (37, 38), and
`mk:scroll_left/F2`, `mk:scroll_down/F3`, `mk:scroll_up/F4` and `mk:scroll_right/F5` on W E R T
(2-5). `mb:middle` on A (13) is unconditional. The
mouse keys need firmware with `caps=...,mouse` in `#V` (see Compatibility); older firmware
answers `err action`.

`<key>`:

- A ZMK key name: `A`..`Z`, `N1`..`N0`, `RET ESC BSPC TAB SPACE MINUS EQUAL
  LBKT RBKT BSLH SEMI SQT GRAVE COMMA DOT FSLH CAPS`, `F1`..`F24`, `PSCRN SLCK
  PAUSE_BREAK INS HOME PG_UP DEL END PG_DN RIGHT LEFT DOWN UP`, `LCTRL LSHFT
  LALT LGUI RCTRL RSHFT RALT RGUI`. This is `kb_core::keys::name`, and
  `kb_emu_studio_key_name` lists them.
- A shifted alias, meaning Left Shift plus a key: `EXCL AT HASH DLLR PRCNT CARET
  AMPS ASTRK LPAR RPAR UNDER PLUS LBRC RBRC PIPE TILDE COLON DQT LT GT QMARK`.
  `MEH` is Ctrl+Shift+Alt.
- A usage that has no name: `0x87`. This is one or two hex digits.
- Modifier functions around a key: `LC() LS() LA() LG() RC() RS() RA() RG()`,
  for example `LC(C)` or `LG(LS(N1))`.

The canonical form:

- The kinds are lower case and the key names upper case.
- Modifiers are written outermost first, in the order `LG LC LS LA RG RC RS RA`.
- An alias is written only for exactly Left Shift plus its key. So `kp:EXCL` is
  canonical, but Gui+Shift+1 is `kp:LG(LS(N1))`, not `kp:LG(EXCL)`.

Extra spellings that are read but never written: `ENTER RETURN ESCAPE BACKSPACE
SPC DELETE LCTL LSHIFT LCMD RCTL RSHIFT RCMD`.

Studio refuses a binding whose canonical encoding is longer than 40 characters
(`max=` in `#V`).

Legends: `kb_emu_studio_label(enc)` gives the legend the emulator draws for an
encoding. That is the firmware's own label code (`action_label`). For example
`kp:EXCL` is `!`, `mt:LALT/TAB` is `Tab\nAlt`, and `mo:2` is `nav`. Mouse legends:

| Encoding | Legend |
|---|---|
| `mm:left` | `◀ mouse` |
| `mb:left` | `L click` |
| `ms:down` | `scroll ↓` |
| `mp` | `Precise` |
| `mk:left/BSPC` | `Bksp\nL click` |
| `mk:scroll_left/F2` | `F2\nscroll ←` |

## Combo encoding

A combo is six words, separated by single spaces:

```text
<name> <positions> <layers> <timeout> <idle> <action>
copy   26,27       0        50        150    kp:LC(C)
-      25,26       *        50        0      sys:BtClear
```

| Word | Meaning |
|---|---|
| `name` | Up to 16 of `A-Z a-z 0-9 _`, or `-` for none |
| `positions` | 2 to 4 distinct positions, comma separated. Written ascending; read in any order |
| `layers` | The layers it is active on, comma separated (`0,2`), or `*` for all. Like ZMK, it is checked against the highest active layer |
| `timeout` | All keys must be down within this many ms of the first one, 1..65535. The compiled combos use 50 |
| `idle` | ZMK's `require-prior-idle-ms`, 0..65535. 0 is off. The combo only counts if no key was typed that long before |
| `action` | As in "Action encoding" |

The compiled combos are `cut`, `copy`, `paste`, `caps_word`, `escape` and
`reset_bluetooth`. `reset_bluetooth` is `25,26 2 50 0 sys:BtClear`: nav only, no
prior idle. Editing or adding other combos leaves it unchanged; it is just
another combo in the set and can itself be edited.

Indices are list positions 0..count-1. Removing a combo moves the later ones
down. Every accepted combo change answers with the whole list again, so the app
should always use the indices from the latest `#C` list.

## Protocol (USB log port)

The protocol runs on the same CDC port and line format as the debug stream
(`core/src/debug.rs`). Lines end in `\r\n`, and keyboard lines start with `#`,
a tag and a space. You don't need `#debug 1` for Studio. Commands are one per
line.

### Commands (app → keyboard)

| Command | Answer |
|---|---|
| `#studio get` | `#V`, then `#L` for every layer, then the `#C` list, then `#U`, `#D` |
| `#studio status` | `#U`, `#D` |
| `#studio combos` | the `#C` list: `#C <i> <combo>` for each, then `#C end <count>` |
| `#studio defaults` | `#LD` for every layer, then `#CD <i> <combo>` for each, then `#CD end <count>`. This is the compiled keymap of this firmware, whatever is edited |
| `#studio set <layer> <pos> <action>` | `#A set <layer> <pos> ok <canonical action>` or `#A set <layer> <pos> err <why>`, then `#U`, `#D` |
| `#studio check <layer> <pos> <action>` | `#A check <layer> <pos> ok <canonical action>` or `#A check <layer> <pos> err <why>`. Changes nothing and works locked |
| `#studio combo add <combo>` | `#A combo add <i> ok <canonical combo>` or `#A combo add new err <why>`. On ok, the `#C` list follows. Then `#U`, `#D` |
| `#studio combo set <i> <combo>` | `#A combo set <i> ok <canonical combo>` or `#A combo set <i> err <why>`. On ok, the `#C` list follows. Then `#U`, `#D` |
| `#studio combo remove <i>` | `#A combo remove <i> ok` or `#A combo remove <i> err <why>`. On ok, the `#C` list follows. Then `#U`, `#D` |
| `#studio combo check <i\|new> <combo>` | `#A combo check <i\|new> ok <canonical combo>` or `err <why>`. Checks `<combo>` as a replacement for combo `<i>`, or as a new combo. Changes nothing and works locked |
| `#studio save` | `#A save ok` or `#A save err <why>`, then `#U`, `#D` |
| `#studio revert` | `#A revert ok`, then `#L` for every layer, then the `#C` list, then `#D` |
| `#studio reset` | `#A reset ok` (then `#L` for every layer and the `#C` list) or `#A reset err <why>`, then `#U`, `#D` |
| `#studio lock` | `#A lock ok`, then `#U` |
| anything else starting with `#studio` and a space | `#A ? err syntax` |

`save`, `revert`, `reset` and the `dirty` state cover keys and combos together.

Removing a combo can't be checked with a dry run on the port. If the remove
would lock you out, the keyboard answers `#A combo remove <i> err lockout` and
changes nothing. The emulator has `kb_emu_studio_combo_check_remove`.

The `<why>` values:

| `<why>` | Meaning |
|---|---|
| `locked` | Press StudioUnlock first |
| `range` | No such layer or position |
| `action` | Not an action, or a `mo:` to a layer that doesn't exist |
| `long` | The encoding is over 40 characters |
| `lockout` | It would leave no reachable StudioUnlock key or combo (see Security) |
| `complex` | The keymap is too tangled to check, so it was refused |
| `positions` | Combo positions are not 2 to 4 distinct positions below 42 |
| `name` | The combo name is not up to 16 of `A-Z a-z 0-9 _` |
| `timing` | The combo timeout is not 1..65535, or idle is not 0..65535 |
| `full` | There are already 16 combos |
| `syntax` | A combo that isn't six words |

A layer in a combo's `layers` that doesn't exist answers `range`.

The first `#studio` command of a connection also reports `#U` and `#D` if they
weren't sent yet.

Command lines may be up to 160 bytes long; longer lines are dropped.

### Lines (keyboard → app)

```text
#V 2 layers=4 keys=42 store=flash max=40 combos=16 caps=check,combos,defaults,mouse names=default,symbols,nav,wm
#L 0 kp:ESC kp:Q kp:W ... mo:2            layer, then one action per position 0..41
#LD 0 kp:ESC kp:Q ...                     the same, compiled (#studio defaults)
#C 0 cut 25,26 0 50 150 kp:LC(X)          combo index, then its six words
#C end 6                                  end of the list, with the count
#CD 0 cut 25,26 0 50 150 kp:LC(X)         compiled combos (#studio defaults)
#CD end 6
#A set 0 12 ok mt:LCTRL/TAB               acknowledgement, in command order
#A check 2 35 err lockout
#A combo add 6 ok x 1,2 * 40 0 caps
#U unlocked 287                           seconds until it locks itself
#U locked
#D 1                                      live bindings or combos differ from what is stored (0: same)
```

- `#V`:
  - The protocol version, 2.
  - `combos=` is the most combos it holds.
  - `caps=` lists what this firmware can do beyond version 1: `check`, `combos`,
    `defaults`. Version 1 firmware sends no `combos=` and no `caps=`.
  - Its fields are `key=value` in a fixed order with `names=` last. Readers
    should look fields up by key and ignore ones they don't know. `store=flash` means saved edits survive a
  restart. `store=ram` is the USB-only build, which has no storage, and the
  emulator without a file. There, `save` works but only lasts until restart.
  `names=` lists the layer names.
- `#L` is always the live bindings. It is the raw binding of each layer, not
  resolved through `trans`.
- After the first `#studio` command, `#U` and `#D` also come by themselves
  whenever they change. That happens on the StudioUnlock key, the timeout,
  writes and flash results. A `#U unlocked <s>` comes again whenever the window
  restarts: on an unlock press or an accepted write.
- With flash, `#D 1` stays after `save` until the record is written, usually
  milliseconds. If the write fails, `#D` stays 1. Save again to retry; the log
  shows `storage: keymap record ... not stored`.
- Acknowledgements are queued (32). A reader that sends faster than it reads
  stops being read until they are out, so it never loses an `#A`.

### Compatibility and capability detection

- **Detect what the keyboard can do**: send `#studio get` and read the `#V`
  line.
  - No `#V` within about 1 s: firmware without Studio. It ignores `#studio`
    lines, which don't parse or are longer than its 16-byte line buffer.
  - `#V 1`, or no `caps=`: keys only. Don't send `check`, `combos`,
    `defaults` or `combo ...`; version 1 answers them with `#A ? err syntax`.
    Use the emulator's rules (`kb_emu_studio_check`) for dry runs, the
    emulator's compiled keymap as the default, and `Keymap.Combos` for
    read-only combos.
  - `#V 2` (or later) with `caps=`: check each capability by name. `mouse` means the mouse
    actions (`mm: mb: ms: mp mk:`) can be bound.
- **Old apps** never send `#studio`, so new firmware sends them no Studio lines.
  The debug stream (`#H #S #K #R #F`) is unchanged, and `#H` is still version 1.
- **v1 Studio clients**:
  - `get` now also sends the `#C` lines; skip tags you don't know.
  - The `#V` line got new fields before `names=`; find fields by key.
  - Everything else of version 1 is unchanged.
- **Unknown tags**: unknown `#X`/`#XY` tags must be skipped, as
  `DebugProtocol.Parse` already does for anything it doesn't know. Note that
  `#LD`, `#CD` and `#C end` have a two-letter tag or a non-number index.

## Security

- **Reads are always allowed**: `get`, `status`, `#L`.
- **Writes need the unlock**: `set`, `combo add|set|remove`, `save` and
  `reset` work only while Studio is unlocked. `check` and `combo check` work
  locked: they change nothing and don't extend the unlock.
  - The physical `StudioUnlock` key unlocks it. That is on nav, position 35:
    hold 41, press 35.
  - It stays unlocked until 5 minutes have passed (`UNLOCK_IDLE_MS`) since the
    last unlock press or accepted write. A refused write doesn't count.
  - `#studio lock` or a restart locks it.
  - Pressing the key again while unlocked restarts the 5 minutes; it does not
    toggle.
  - `revert` also works locked, because it can only go back to what was
    already saved.
- **The left OLED shows the lock state for 2.5 s after it changes**: an
  inverted box at the bottom, `STUDIO` / `OPEN` or `STUDIO` / `LOCKED`.
- **Lockout guard**: Studio refuses any key or combo change that would leave no
  `StudioUnlock` key or combo you can press (`err lockout`). Without one, the
  keyboard could never be edited or reset again without reflashing.
  - Reachability is checked like fingers. Start on layer 0 and hold keys that
    resolve to `mo:` on the active layers. Each physical key can be held only
    once, and conditional layers switch on as in the engine. Then look for an
    unheld key that resolves to `StudioUnlock`.
  - So these are refused:
    - removing the unlock key, `set 2 35 trans`;
    - removing the only way to nav, `set 0 41 kp:A`;
    - moving the unlock onto the key that holds its layer, `set 2 41
      sys:StudioUnlock` after it was removed from 35.
  - Moving the nav key works in two steps: first add `mo:2` elsewhere, then
    change 41.
  - Combos count:
    - A combo counts when it is active on the highest active layer and none of
      its keys are held. If its action is `sys:StudioUnlock`, the keyboard can
      be unlocked.
    - If its action is `mo:<n>`, its keys are held for layer `n` and the
      search goes on from there.
    - So an unlock combo on layer 0 lets you remove the unlock key. Removing or
      changing that combo afterwards is then refused.
    - A combo never takes a key away: pressed alone, a combo key goes through
      after the combo timeout.
  - The search is bounded. A keymap too tangled to check is refused
    (`err complex`).
- **At start**, stored edits that would lock out are dropped, for example after
  a firmware with a different compiled keymap. Keys and combos are dropped
  together and cleared in flash, and the log says so.
- **Combo validation**: every combo must have 2 to 4 distinct positions, a
  valid action (as for keys), existing layers, a timeout of 1..65535 ms and an
  idle of 0..65535 ms. There can be at most 16.

## Storage (firmware)

- **Map keys**: Studio uses keys `0x50`..`0x6F` in the left half's
  sequential-storage map, the same 32 KB region as the bonds. Its neighbours are
  0 split bond, `0x10+` profiles, `0x20` active profile, `0x30+` subscriptions
  and `0x40` radio. Combos use keys `0x70`..`0x77`.
- **Blocks**: each layer is cut into blocks of 21 positions. Each block with an
  edit has one record, so at most 8 records. A record holds only the positions
  that differ from the compiled keymap (`core/src/studio/record.rs`):

  ```text
  version(1) layer block bitmap[3] actions...
  ```

  Each action is: `00` trans, `01` none, `02` caps, `03 code mods`, `04 layer`,
  `05 hc hm tc tm`, `06 system#`, `07 mouse` (one byte: `0x0d` move, `0x1b` button, `0x2d`
  scroll with d = left down up right, b = left right middle; `0x30` precision), or
  `08 mouse code mods` (mouse-or-key; `mouse` as for `07`, a button or scroll only). Older firmware refuses a record with `07`/`08` as a
  whole, so its block goes back to compiled.
- **Size**: a record is at most 111 bytes. It must fit the map readers'
  128-byte buffers.
- **Saving**: `save` writes only the blocks that changed.
- **Clearing**: `reset` writes empty records, because flash written through MPSL
  can't drop items.
- **Versions**: a record of another version is skipped. This is how a future
  format migrates.
- **Combo records** (`core/src/studio/combo.rs`):
  - They are written only when the combos differ from the compiled set, or to
    clear a stored set. The whole set is split over as many records as needed;
    each record is at most 111 bytes, and 16 combos with long names take 6:

    ```text
    version(1) index set_id total count  then per combo:
    nkeys positions... action layers(bitmap) timeout(u16 LE) idle(u16 LE) namelen name...
    ```

  - `total = 0xFF` in record 0 marks "the compiled combos". Reset writes it.
    No record also means compiled.
  - Every save gets a new `set_id`. If a set has records of two ids (a save cut
    short) or records are missing, it is not used, and the log says
    `stored combos not used`.
  - Records after a complete set are leftovers of a larger earlier set and are
    ignored.
  - A combo record of another version is skipped, and the compiled combos are
    used.
- **USB-only build**: there is no storage map in this build, so editing works in
  RAM only (`store=ram`).

## Debug stream additions (mouse)

These lines are in the `#debug 1` stream next to `#R`:

```text
#M 01d4feb004ff01   the last mouse report for the computer: buttons, x (i16 LE), y (i16 LE), wheel (i8), pan (i8)
#S ... radio=4 pointer=1   pointer: 1 while the pointer is active (the thumbs click; the MOUSE bar)
```

- **Report layout**: x is right +, y is down +, wheel is up +, pan is right +.
- **Muted reports**: `#M` also shows reports that were muted (`m`), like `#R`.
- **Old apps**: they show `#M` as a log line and ignore `pointer=`, since it comes after
  `radio=`.

## Emulator C ABI (`kb-emu`, `emu/src/lib.rs`)

Built with `./dev cargo build --release -p kb-emu` →
`target/release/libkb_emu.so`.

- **Strings in**: (pointer, byte length), UTF-8, no NUL.
- **Strings out**: (buf, cap). The full length is returned, at most `cap` bytes
  are copied, and no NUL is added. This is the same as `kb_emu_label`.
- **Times**: `now_ms` is the same clock as `kb_emu_press` and `kb_emu_tick`.

Result codes of the functions that change something:

| Code | Meaning |
|---|---|
| 0 | ok |
| 1 | locked |
| 2 | range |
| 3 | action |
| 4 | long |
| 5 | lockout |
| 6 | complex |
| 7 | storage (the file couldn't be written; still dirty) |
| 8 | positions (combo) |
| 9 | name (combo) |
| 10 | timing (combo) |
| 11 | full (16 combos) |
| 12 | syntax (combo text isn't six words) |
| -1 | bad arguments (null emulator, invalid UTF-8) |

```c
uint8_t  kb_emu_studio_layer_count(void);                       // 4
size_t   kb_emu_studio_binding(Emu*, uint8_t layer, uint8_t pos, uint8_t* buf, size_t cap);         // live, encoded
size_t   kb_emu_studio_default_binding(Emu*, uint8_t layer, uint8_t pos, uint8_t* buf, size_t cap); // compiled
size_t   kb_emu_studio_layer(Emu*, uint8_t layer, uint8_t* buf, size_t cap);   // 42 encodings, space separated
int32_t  kb_emu_studio_set(Emu*, uint8_t layer, uint8_t pos, const uint8_t* enc, size_t len, uint32_t now_ms);
int32_t  kb_emu_studio_check(Emu*, uint8_t layer, uint8_t pos, const uint8_t* enc, size_t len);   // dry run, no lock check
int32_t  kb_emu_studio_save(Emu*, uint32_t now_ms);
int32_t  kb_emu_studio_revert(Emu*);                            // works locked
int32_t  kb_emu_studio_reset(Emu*, uint32_t now_ms);
void     kb_emu_studio_lock(Emu*, uint32_t now_ms);
uint32_t kb_emu_studio_unlock_remaining_ms(Emu*, uint32_t now_ms);   // 0 = locked
bool     kb_emu_studio_dirty(Emu*);
uint32_t kb_emu_studio_generation(Emu*);                        // changes whenever live bindings change
int32_t  kb_emu_studio_set_storage_path(Emu*, const uint8_t* path, size_t len);
         // 0 applied / no file yet, 1 ignored (bad file), 2 dropped (would lock out; file cleared), -1 bad args
bool     kb_emu_studio_command(Emu*, const uint8_t* line, size_t len, uint32_t now_ms);  // "#studio ..." as on the port
size_t   kb_emu_studio_next_line(Emu*, uint32_t now_ms, uint8_t* buf, size_t cap);      // one answer line incl. \r\n, 0 = none
uint32_t kb_emu_studio_line_max(void);                          // a buffer this big always fits a line
int64_t  kb_emu_studio_canonical(const uint8_t* enc, size_t len, uint8_t* buf, size_t cap);  // -1 = not an action
size_t   kb_emu_studio_label(const uint8_t* enc, size_t len, uint8_t* buf, size_t cap);      // legend ("" if invalid)
size_t   kb_emu_studio_key_name(uint32_t index, uint8_t* buf, size_t cap);                    // 0 past the end
size_t   kb_emu_studio_system_name(uint32_t index, uint8_t* buf, size_t cap);                 // 0 past the end

// Combos: text is the six words, e.g. "copy 26,27 0 50 150 kp:LC(C)"
uint8_t  kb_emu_studio_combo_max(void);                         // 16
uint8_t  kb_emu_studio_combo_count(Emu*);                       // live combos
size_t   kb_emu_studio_combo(Emu*, uint8_t index, uint8_t* buf, size_t cap);           // live combo, 0 past the end
uint8_t  kb_emu_studio_default_combo_count(Emu*);               // compiled combos
size_t   kb_emu_studio_default_combo(Emu*, uint8_t index, uint8_t* buf, size_t cap);   // compiled combo
int32_t  kb_emu_studio_combo_add(Emu*, const uint8_t* text, size_t len, uint32_t now_ms);  // new index = count - 1
int32_t  kb_emu_studio_combo_set(Emu*, uint8_t index, const uint8_t* text, size_t len, uint32_t now_ms);
int32_t  kb_emu_studio_combo_remove(Emu*, uint8_t index, uint32_t now_ms);
int32_t  kb_emu_studio_combo_check(Emu*, uint8_t index, const uint8_t* text, size_t len);  // index 255 = as a new combo; no lock check
int32_t  kb_emu_studio_combo_check_remove(Emu*, uint8_t index);                           // no lock check
```

Mouse (not Studio, but in the same library):

```c
// kb_emu_poll: kind 7 = a mouse report for the computer. mods = buttons
// (bit 0 left, 1 right, 2 middle); keys[0..7] = the report bytes as in #M.
// kind 0 = nothing to show (skip). Reports come every ms of motion on USB,
// at most every 8 ms on Bluetooth (kb_emu_set_transport).
typedef struct { int32_t x, y, wheel, pan; uint8_t buttons, pointer, moving, hires; } KbMouse;
bool kb_emu_mouse_state(Emu*, KbMouse* out);   // summed motion since start/reset; pointer = MOUSE bar shown
void kb_emu_mouse_reset_position(Emu*);         // x, y, wheel, pan back to 0 (buttons kept)
void kb_emu_mouse_set_resolution(Emu*, bool wheel_hires, bool pan_hires);  // the "computer" sets the
                                                // Resolution Multiplier: wheel/pan then in 1/16 detents
```

`hires` is bit 0 for the wheel and bit 1 for the pan. With a bit set, `wheel` / `pan` and the
report bytes count 1/16 detents; otherwise they count whole detents. A new emulator starts with
whole detents, as on macOS.

On the keyboard the computer sets the multipliers through the mouse's feature report: the
USB mouse interface's SET_REPORT(Feature), or over Bluetooth the HID report characteristic with
Report Reference `[2, 3]`. The feature report is one byte: bits 0-1 wheel, bits 2-3 pan, and a
value of 1 means ×16.

While `moving` is 1, `kb_emu_next_deadline` asks for a tick every millisecond. The mouse moves
in 1 ms steps, so tick at least that often for smooth motion.

The `kb_emu_studio_command` path supports every protocol command above,
including `check`, `defaults`, `combos` and `combo ...`. Its `#V` says
`store=flash` with a storage file and `store=ram` without.

Notes for the app:

- **Unlocking**: unlock exactly as on the keyboard: `kb_emu_press(41)`,
  `kb_emu_press(35)`, release both, then `kb_emu_poll`. The unlock takes effect
  when that output is polled. The left OLED frame shows the STUDIO box, and
  `kb_emu_poll` still reports system action 4.
- **Labels follow edits**: after any edit, `kb_emu_label` reflects the edit.
- **Two ways in**: the emulator can be driven by the direct functions, or by
  `kb_emu_studio_command` + `kb_emu_studio_next_line` with exactly the
  keyboard's text protocol. With the second, one protocol client in the app
  serves both the emulator and a real keyboard.
- **`kb_emu_studio_next_line` buffers**: a line is only consumed when it fits.
  Otherwise the call returns the needed length; call again with that size.
- **Emulated storage**: storage is in memory by default. It survives emulated
  restarts (reset / power switch / waking from deep sleep), but not the app.
  On such a restart, unsaved edits are dropped and Studio locks.
- **Storage file**: `kb_emu_studio_set_storage_path` makes it a text file, for
  example `~/.config/emk/emulator-keymap.txt`. Pass an absolute path; there is
  no `~` expansion.

  ```text
  # EMK emulator keymap: Studio's edits over the compiled keymap
  emk-keymap 2
  0 12 mt:LCTRL/TAB
  combos 2
  combo copy 26,27 0 50 150 kp:LC(C)
  combo - 1,2 * 40 0 caps
  ```

  - It holds one `<layer> <pos> <action>` line per binding that differs from the
    compiled keymap.
  - When the combos differ from the compiled ones, it also holds `combos
    <count>` and the whole set, one `combo <six words>` line each. With no
    `combos` line the compiled combos are used.
  - Version 1 files (keys only) are still read.
  - Saving rewrites it whole (temp file + rename), and the parent directory is
    created.
  - `kb_emu_studio_set_storage_path` returns 1 (ignored, compiled keymap) for a
    file with bad combo lines or a wrong count.
- **`kb_mirror_label` is not live**: it is unchanged and still shows the
  compiled keymap. For a real keyboard, take the legends from `#L` plus
  `kb_emu_studio_label`.
