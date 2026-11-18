//! Host timings for the keymap engine, the debouncer and the OLED screens.
//!
//!   ./dev sh -c 'cd bench && cargo run --release'
//!
//! Plain `Instant` loops: every case runs for ~20 ms per sample, 7 samples,
//! and the median time per iteration is printed. Numbers are host numbers;
//! see docs/performance.md for the translation to the nRF52840.

use std::hint::black_box;
use std::time::Instant;

use kb_core::debounce::Debouncer;
use kb_core::keymaps::corne::{self, KEYMAP};
use kb_core::report::{Report, diff};
use kb_core::{Config, Engine, host::Text};
use kb_display::frame::{Frame, Half};
use kb_display::screens::{self, Style};
use kb_display::ssd1306;
use kb_display::status::{Status, Transport};

type Kb = Engine<'static, { corne::KEYS }>;

/// Median nanoseconds per call of `f`.
fn measure(mut f: impl FnMut()) -> f64 {
    // Calibrate the iteration count to ~20 ms per sample.
    let mut iters = 1u64;
    loop {
        let t = Instant::now();
        for _ in 0..iters {
            f();
        }
        if t.elapsed().as_millis() >= 20 {
            break;
        }
        iters *= 2;
    }
    let mut samples: Vec<f64> = (0..7)
        .map(|_| {
            let t = Instant::now();
            for _ in 0..iters {
                f();
            }
            t.elapsed().as_nanos() as f64 / iters as f64
        })
        .collect();
    samples.sort_by(f64::total_cmp);
    samples[3]
}

fn row(name: &str, ns: f64, per: &str) {
    println!("{name:<44} {ns:>10.1} ns {per}");
}

// ---- engine scenarios --------------------------------------------------------

/// A key event: (time in ms from the start of the scenario, position, pressed).
type Event = (u32, u8, bool);

/// Positions of letters on the Corne default layer.
fn letter_pos(c: char) -> Option<u8> {
    let rows = ["qwertyuiop", "asdfghjkl", "zxcvbnm"];
    let starts = [1u8, 13, 25];
    for (row, start) in rows.iter().zip(starts) {
        if let Some(i) = row.find(c) {
            return Some(start + i as u8);
        }
    }
    (c == ' ').then_some(38)
}

/// Rolling typing: a key every 60 ms, each held 70 ms (10 ms overlap), ~200 WPM.
fn typing(text: &str) -> Vec<Event> {
    let mut ev = Vec::new();
    for (i, pos) in text.chars().filter_map(letter_pos).enumerate() {
        let t = i as u32 * 60;
        ev.push((t, pos, true));
        ev.push((t + 70, pos, false));
    }
    ev.sort_by_key(|e| e.0);
    ev
}

/// Mod-tap Alt/Tab (12) and F20/Bspc (37): hold + key, quick tap, hold past the tapping term.
fn mod_taps() -> Vec<Event> {
    vec![
        (0, 12, true),
        (20, 13, true),
        (60, 13, false),
        (90, 12, false), // Alt+A (decided by the press)
        (200, 37, true),
        (260, 37, false), // tap: Bspc
        (400, 12, true),
        (700, 12, false), // hold past 200 ms: Alt
    ]
}

/// Combos: copy (26+27), escape (19+20), and the 6-key `reset_bluetooth` on nav.
fn combos() -> Vec<Event> {
    vec![
        (0, 26, true),
        (8, 27, true),
        (80, 26, false),
        (85, 27, false),
        (200, 19, true),
        (230, 19, false), // lone J: released before the timeout
        (300, 41, true),  // nav
        (320, 5, true),
        (325, 6, true),
        (330, 4, true),
        (335, 3, true),
        (340, 7, true),
        (345, 8, true),
        (400, 5, false),
        (401, 6, false),
        (402, 4, false),
        (403, 3, false),
        (404, 7, false),
        (405, 8, false),
        (450, 41, false),
    ]
}

/// Layers: symbols, nav, the conditional wm layer (symbols + nav) with Super+1, then back.
fn layers() -> Vec<Event> {
    vec![
        (0, 36, true),
        (20, 14, true),
        (40, 14, false), // symbols: 2
        (60, 41, true),
        (80, 13, true),
        (100, 13, false), // wm: Super+1
        (120, 36, false),
        (140, 18, true),
        (160, 18, false), // nav: Left
        (180, 41, false),
    ]
}

const PERIOD: u32 = 2_000;

/// Feed the events only (press/release + drain outputs), like an event-driven caller.
fn run_events(kb: &mut Kb, events: &[Event], base: u32) {
    for &(t, pos, down) in events {
        if down {
            kb.press(pos, base + t);
        } else {
            kb.release(pos, base + t);
        }
        while let Some(out) = kb.poll() {
            black_box(out);
        }
    }
    kb.tick(base + PERIOD - 1);
    while kb.poll().is_some() {}
}

/// What fw-nrf does: every millisecond deliver that ms's events, tick, and drain outputs.
fn run_loop(kb: &mut Kb, events: &[Event], base: u32) {
    let mut next = 0;
    for ms in 0..PERIOD {
        let now = base + ms;
        while next < events.len() && events[next].0 == ms {
            let (_, pos, down) = events[next];
            if down {
                kb.press(pos, now);
            } else {
                kb.release(pos, now);
            }
            next += 1;
        }
        kb.tick(now);
        while let Some(out) = kb.poll() {
            black_box(out);
        }
    }
}

/// Slow typing: a key every 150 ms, each held 80 ms (no overlap).
fn slow_typing(text: &str) -> Vec<Event> {
    text.chars()
        .filter_map(letter_pos)
        .enumerate()
        .flat_map(|(i, pos)| {
            let t = i as u32 * 150;
            [(t, pos, true), (t + 80, pos, false)]
        })
        .collect()
}

/// Engine-added latency: ms from a (debounced) press to the first report
/// containing that key's usage, driven like the fw-nrf loop.
fn latency(name: &str, events: &[Event]) {
    let mut kb = Engine::new(&KEYMAP, Config::default());
    let mut pending: Vec<(u32, u8, char)> = Vec::new(); // (press time, usage, letter)
    let mut delays: Vec<(u32, char)> = Vec::new();
    let mut next = 0;
    let end = events.last().map_or(0, |e| e.0) + 300;
    for now in 0..end {
        while next < events.len() && events[next].0 == now {
            let (_, pos, down) = events[next];
            if down {
                if let kb_core::Action::Key(k) = kb.binding(pos) {
                    let letter = (b'a' + k.code.wrapping_sub(0x04)) as char;
                    pending.push((now, k.code, if k.code == 0x2C { ' ' } else { letter }));
                }
                kb.press(pos, now);
            } else {
                kb.release(pos, now);
            }
            next += 1;
        }
        kb.tick(now);
        while let Some(out) = kb.poll() {
            if let kb_core::Output::Report(r) = out {
                pending.retain(|&(t, code, c)| {
                    let seen = r.contains(code);
                    if seen {
                        delays.push((now - t, c));
                    }
                    !seen
                });
            }
        }
    }
    let delayed: Vec<String> = delays
        .iter()
        .filter(|d| d.0 > 0)
        .map(|d| format!("{}:{}", d.1, d.0))
        .collect();
    let avg = f64::from(delays.iter().map(|d| d.0).sum::<u32>()) / delays.len() as f64;
    let max = delays.iter().map(|d| d.0).max().unwrap_or(0);
    println!(
        "{name:<44} avg {avg:>5.1} ms, max {max:>2} ms; {} of {} keys delayed: {}",
        delays.iter().filter(|d| d.0 > 0).count(),
        delays.len(),
        delayed.join(" "),
    );
}

fn engine_benches() {
    println!("\n== engine-added latency (press -> report with the key) ==");
    let text = "the quick brown fox jumps over the lazy dog";
    latency("rolling typing (60 ms/key, 10 ms overlap)", &typing(text));
    latency("slow typing (150 ms/key, no overlap)", &slow_typing(text));

    println!("\n== engine (Corne keymap) ==");
    let scenarios = [
        (
            "typing \"the quick brown fox ...\"",
            typing("the quick brown fox jumps over the lazy dog"),
        ),
        ("mod-taps", mod_taps()),
        ("combos (2-key, lone key, 6-key)", combos()),
        ("layers (sym, wm, nav)", layers()),
    ];
    for (name, events) in &scenarios {
        let mut kb = Engine::new(&KEYMAP, Config::default());
        let mut base = 0u32;
        let per_run = measure(|| {
            run_events(&mut kb, events, base);
            base = base.wrapping_add(PERIOD);
        });
        row(
            &format!("{name}: per event"),
            per_run / events.len() as f64,
            &format!("({} events)", events.len()),
        );

        let mut kb = Engine::new(&KEYMAP, Config::default());
        let mut base = 0u32;
        let per_run = measure(|| {
            run_loop(&mut kb, events, base);
            base = base.wrapping_add(PERIOD);
        });
        row(
            &format!("{name}: 1 kHz loop, per ms"),
            per_run / f64::from(PERIOD),
            "(tick + poll + events)",
        );
    }

    let mut kb = Engine::new(&KEYMAP, Config::default());
    row(
        "Engine::new",
        measure(|| {
            black_box(Engine::new(&KEYMAP, Config::default()));
        }),
        "",
    );

    let mut now = 0u32;
    row(
        "tick + poll, idle",
        measure(|| {
            now += 1;
            kb.tick(black_box(now));
            black_box(kb.poll());
        }),
        "",
    );

    // A combo key buffered: every tick recomputes combo_deadline() over all combos.
    let mut kb = Engine::new(&KEYMAP, Config::default());
    kb.press(26, 0);
    row(
        "tick, combo key buffered (not due)",
        measure(|| kb.tick(black_box(10))),
        "",
    );
    row(
        "next_deadline, combo key buffered",
        measure(|| {
            black_box(kb.next_deadline());
        }),
        "",
    );

    // Plain key press + release: two emit_report() calls over 256 key counts.
    let mut kb = Engine::new(&KEYMAP, Config::default());
    let mut now = 0u32;
    row(
        "press+release Q (+ poll)",
        measure(|| {
            now += 10;
            kb.press(1, now);
            kb.release(1, now + 5);
            while let Some(o) = kb.poll() {
                black_box(o);
            }
        }),
        "(2 reports)",
    );

    // Lookup through 4 layers (wm active: trans falls through 3 -> 2 -> 1 -> 0 on some keys).
    let mut kb = Engine::new(&KEYMAP, Config::default());
    kb.press(36, 0);
    kb.press(41, 1);
    row(
        "binding() x42 on wm layer",
        measure(|| {
            for pos in 0..corne::KEYS as u8 {
                black_box(kb.binding(black_box(pos)));
            }
        }),
        "",
    );
}

// ---- report helpers and a model of emit_report ----------------------------------

/// What `Engine::emit_report` does today: rebuild the report from 256 counts.
fn emit_model(mod_counts: [u8; 8], key_counts: &[u8; 256]) -> Report {
    let mut r = Report::default();
    for (i, c) in mod_counts.into_iter().enumerate() {
        if c > 0 {
            r.mods |= 1 << i;
        }
    }
    for (code, &c) in key_counts.iter().enumerate() {
        if c > 0 {
            r.set(code as u8, true);
        }
    }
    r
}

fn report_benches() {
    println!("\n== report ==");
    let mut counts = [0u8; 256];
    counts[0x04] = 1;
    counts[0x1E] = 1;
    let mods = [0u8; 8];
    row(
        "emit_report model (256 counts -> Report)",
        measure(|| {
            black_box(emit_model(black_box(mods), black_box(&counts)));
        }),
        "",
    );

    let mut r = Report::default();
    r.set(0x04, true);
    r.set(0x1E, true);
    row(
        "Report::boot_keys (2 keys down)",
        measure(|| {
            black_box(black_box(&r).boot_keys());
        }),
        "",
    );
    row(
        "Report::keys().count()",
        measure(|| {
            black_box(black_box(&r).keys().count());
        }),
        "",
    );
    let prev = Report::default();
    row(
        "report::diff",
        measure(|| {
            diff(black_box(&prev), black_box(&r), |c, p| {
                black_box((c, p));
            });
        }),
        "",
    );
}

// ---- debounce ----------------------------------------------------------------

fn debounce_benches() {
    println!("\n== debounce (42 keys) ==");
    let mut d = Debouncer::<{ corne::KEYS }>::new(5, 5);
    let raw = [false; corne::KEYS];
    let mut now = 0u32;
    row(
        "update, all keys idle",
        measure(|| {
            now += 1;
            d.update(black_box(&raw), now, |p, s| {
                black_box((p, s));
            });
        }),
        "",
    );

    let mut d = Debouncer::<{ corne::KEYS }>::new(5, 5);
    let mut raw = [false; corne::KEYS];
    let mut now = 0u32;
    row(
        "update, 2 keys toggling every 3 ms",
        measure(|| {
            now += 1;
            if now.is_multiple_of(3) {
                raw[1] = !raw[1];
                raw[20] = !raw[20];
            }
            d.update(black_box(&raw), now, |p, s| {
                black_box((p, s));
            });
        }),
        "",
    );
}

// ---- display -------------------------------------------------------------------

fn display_benches() {
    println!("\n== display ==");
    let names = KEYMAP.layer_names;
    let usb = Status {
        transport: Transport::Usb,
        peripheral_connected: false,
        layer: 1,
        wpm: 87,
        ..Status::default()
    };
    let ble_caps = Status {
        caps_word: true,
        layer: 2,
        wpm: 123,
        ..Status::default()
    };
    let note = Status {
        notification: Some(Text::new(
            "Build ok on erikwm: 42 tests passed, 0 failed, see log",
        )),
        ..Status::default()
    };
    let cases = [
        ("render Zmk, left (BLE)", Half::Left, Style::Zmk, &ble_caps),
        (
            "render Zmk, right (notification)",
            Half::Right,
            Style::Zmk,
            &note,
        ),
        (
            "render Portrait, left (USB, fw-nrf)",
            Half::Left,
            Style::Portrait,
            &usb,
        ),
        (
            "render Portrait, left (BLE + CAPS badge)",
            Half::Left,
            Style::Portrait,
            &ble_caps,
        ),
        (
            "render Portrait, right (no notification)",
            Half::Right,
            Style::Portrait,
            &Status::default(),
        ),
        (
            "render Portrait, right (notification)",
            Half::Right,
            Style::Portrait,
            &note,
        ),
    ];
    let mut frame = Frame::default();
    for (name, half, style, status) in cases {
        row(
            name,
            measure(|| {
                screens::render(&mut frame, half, style, black_box(status), names);
                black_box(&frame);
            }),
            "",
        );
        let lit: u32 = frame.buf.iter().map(|b| b.count_ones()).sum();
        println!("{:<44} {lit:>10} lit pixels", "");
    }
    row(
        "ssd1306::frame_data (513 bytes)",
        measure(|| {
            black_box(ssd1306::frame_data(black_box(&frame)));
        }),
        "",
    );
    let other = frame.clone();
    row(
        "Frame == Frame (512 bytes, equal)",
        measure(|| {
            black_box(black_box(&frame) == black_box(&other));
        }),
        "",
    );
}

fn main() {
    if cfg!(debug_assertions) {
        println!("warning: debug build, run with --release");
    }
    println!(
        "sizes: Engine<42> {} B, Output {} B, Frame {} B",
        size_of::<Kb>(),
        size_of::<kb_core::Output>(),
        size_of::<Frame>(),
    );
    engine_benches();
    report_benches();
    debounce_benches();
    display_benches();
}
