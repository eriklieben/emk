//! Run the keyboard engine on Linux.
//!
//! Reads a real keyboard (grabbed, so the desktop no longer sees it), maps
//! its keys to Corne positions, runs them through `kb-core` with your keymap
//! and types the result through a virtual keyboard (`/dev/uinput`).
//!
//! Press the physical Esc key to quit.

mod hid;
mod layout;

use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, Device, EventType, InputEvent, KeyCode};
use kb_core::keymaps::corne;
use kb_core::report::diff;
use kb_core::{Config, Engine, Output, Report, keys};

const VIRTUAL_NAME: &str = "kb-sim corne";

struct Args {
    device: Option<String>,
    list: bool,
    watch: bool,
    verbose: bool,
}

fn parse_args() -> Args {
    let mut args = Args {
        device: None,
        list: false,
        watch: false,
        verbose: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--list" => args.list = true,
            "--watch" => args.watch = true,
            "-v" | "--verbose" => args.verbose = true,
            "--device" => args.device = it.next(),
            "-h" | "--help" => {
                println!(
                    "kb-sim [--device /dev/input/eventN] [--watch] [-v]\n\
                     \n  --list     list keyboards\
                     \n  --device   keyboard to use (default: first one found)\
                     \n  --watch    don't grab or type; only print what would be sent\
                     \n  -v         print every key change\
                     \n\nPhysical Esc quits."
                );
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown argument: {other} (try --help)");
                std::process::exit(2);
            }
        }
    }
    args
}

fn is_keyboard(dev: &Device) -> bool {
    let named_us = dev.name().is_some_and(|n| n.starts_with("kb-sim"));
    let keys = dev.supported_keys();
    !named_us
        && keys.is_some_and(|k| {
            k.contains(KeyCode::KEY_A)
                && k.contains(KeyCode::KEY_SPACE)
                && k.contains(KeyCode::KEY_ENTER)
        })
}

fn open_keyboard(path: Option<&str>) -> Result<(String, Device), Box<dyn std::error::Error>> {
    if let Some(p) = path {
        return Ok((p.to_string(), Device::open(p)?));
    }
    evdev::enumerate()
        .find(|(_, d)| is_keyboard(d))
        .map(|(p, d)| (p.display().to_string(), d))
        .ok_or_else(|| {
            "no keyboard found. Can you read /dev/input/event*? (see README: Permissions)".into()
        })
}

fn virtual_keyboard() -> std::io::Result<VirtualDevice> {
    let mut keys = AttributeSet::<KeyCode>::new();
    for usage in 0..=255u8 {
        if let Some(code) = hid::to_linux(usage) {
            keys.insert(KeyCode::new(code));
        }
    }
    VirtualDevice::builder()?
        .name(VIRTUAL_NAME)
        .with_keys(&keys)?
        .build()
}

/// Wait until no key is held, so the Enter that started us is released
/// before we grab (otherwise the desktop would see it stuck).
fn wait_for_all_keys_up(dev: &Device) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(3) {
        match dev.get_key_state() {
            Ok(state) if state.iter().next().is_none() => return,
            Err(_) => return,
            _ => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

/// Block until `fd` is readable or `timeout_ms` passes (`-1` = forever).
fn wait_readable(fd: i32, timeout_ms: i32) -> bool {
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one valid pollfd for the duration of the call.
    unsafe { libc::poll(&raw mut pfd, 1, timeout_ms) > 0 }
}

fn send(vdev: &mut VirtualDevice, prev: &Report, next: &Report) -> std::io::Result<()> {
    // Releases and presses go in separate SYN frames, releases first, so
    // modifier changes never apply to the wrong key.
    let (mut releases, mut presses) = (Vec::new(), Vec::new());
    diff(prev, next, |usage, pressed| {
        if let Some(code) = hid::to_linux(usage) {
            let ev = InputEvent::new(EventType::KEY.0, code, i32::from(pressed));
            if pressed {
                presses.push(ev);
            } else {
                releases.push(ev);
            }
        }
    });
    if !releases.is_empty() {
        vdev.emit(&releases)?;
    }
    if !presses.is_empty() {
        vdev.emit(&presses)?;
    }
    Ok(())
}

fn describe(prev: &Report, next: &Report) -> String {
    let mut parts = Vec::new();
    diff(prev, next, |usage, pressed| {
        parts.push(format!(
            "{}{}",
            if pressed { '+' } else { '-' },
            keys::name(usage)
        ));
    });
    parts.join(" ")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args();

    if args.list {
        list_keyboards();
        return Ok(());
    }

    let (path, mut dev) = open_keyboard(args.device.as_deref())?;
    println!("keyboard: {path} ({})", dev.name().unwrap_or("?"));

    let mut vdev = take_over(&args, &mut dev)?;
    let verbose = args.verbose || args.watch;

    let mut engine = Engine::new(&corne::KEYMAP, Config::default());
    let start = Instant::now();
    let now = || start.elapsed().as_millis() as u32;
    let mut host = Report::default();
    println!("layer: {}", engine.layer_name(0));

    loop {
        let timeout = match engine.next_deadline() {
            Some(d) => d.saturating_sub(now()) as i32,
            None => -1,
        };
        if wait_readable(dev.as_raw_fd(), timeout) && !feed_keys(&mut dev, &mut engine, &now)? {
            break;
        }
        engine.tick(now());
        drain(&mut engine, &mut host, vdev.as_mut(), verbose)?;
    }

    if let Some(v) = vdev.as_mut() {
        send(v, &host, &Report::default())?;
        dev.ungrab()?;
    }
    println!("bye");
    Ok(())
}

/// `--list`: the keyboards this machine has, one per line.
fn list_keyboards() {
    for (path, dev) in evdev::enumerate().filter(|(_, d)| is_keyboard(d)) {
        println!("{}\t{}", path.display(), dev.name().unwrap_or("?"));
    }
}

/// The virtual keyboard the keymap types on, with the physical one grabbed;
/// `None` in watch mode, which only listens.
fn take_over(
    args: &Args,
    dev: &mut Device,
) -> Result<Option<VirtualDevice>, Box<dyn std::error::Error>> {
    if args.watch {
        println!("watch mode: not grabbing, not typing");
        return Ok(None);
    }
    let v = virtual_keyboard().map_err(|e| {
        format!("cannot create virtual keyboard via /dev/uinput: {e} (see README: Permissions)")
    })?;
    wait_for_all_keys_up(dev);
    dev.grab()?;
    println!("grabbed. Typing goes through your Corne keymap. Physical Esc quits.");
    Ok(Some(v))
}

/// Hand the waiting key events of `dev` to the engine. False once the quit
/// key is pressed.
fn feed_keys(
    dev: &mut Device,
    engine: &mut Engine<'_, { corne::KEYS }>,
    now: &impl Fn() -> u32,
) -> std::io::Result<bool> {
    for ev in dev.fetch_events()? {
        if ev.event_type() != EventType::KEY {
            continue;
        }
        let (code, value) = (ev.code(), ev.value());
        if code == layout::QUIT_KEY && value == 1 {
            return Ok(false);
        }
        match (layout::position(code), value) {
            (Some(pos), 1) => engine.press(pos, now()),
            (Some(pos), 0) => engine.release(pos, now()),
            _ => {} // autorepeat (2) or a key outside the Corne grid
        }
    }
    Ok(true)
}

/// Print what the engine says besides key reports.
fn print_output(engine: &Engine<'_, { corne::KEYS }>, out: Output) {
    match out {
        Output::Report(_) => {}
        Output::Layer(l) => println!("layer: {}", engine.layer_name(l)),
        Output::CapsWord(on) => println!("caps word: {}", if on { "on" } else { "off" }),
        Output::System(s) => println!("system: {s:?} (not simulated)"),
        Output::Mouse(m, down) => println!(
            "mouse: {m:?} {} (not simulated)",
            if down { "down" } else { "up" }
        ),
        Output::Pointer(on) => println!("pointer: {}", if on { "active" } else { "off" }),
    }
}

/// Act on everything the engine has to say: type the reports on `vdev`
/// (`host` is what it last typed) and print the rest.
fn drain(
    engine: &mut Engine<'_, { corne::KEYS }>,
    host: &mut Report,
    mut vdev: Option<&mut VirtualDevice>,
    verbose: bool,
) -> std::io::Result<()> {
    while let Some(out) = engine.poll() {
        if let Output::Report(r) = out {
            if verbose {
                println!("  {}", describe(host, &r));
            }
            if let Some(v) = vdev.as_deref_mut() {
                send(v, host, &r)?;
            }
            *host = r;
        } else {
            print_output(engine, out);
        }
    }
    Ok(())
}
