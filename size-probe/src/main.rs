//! Size probe: runs the keyboard logic in a loop with opaque inputs so the
//! compiler keeps all of it. All state lives in one static, so `.bss`
//! shows the firmware state's RAM.
#![no_std]
#![no_main]

use core::hint::black_box;
use core::mem::MaybeUninit;
use core::panic::PanicInfo;

use cortex_m_rt::entry;
use kb_core::debounce::Debouncer;
use kb_core::host::{HostMessage, REPORT_SIZE, Reassembler};
use kb_core::keymaps::corne;
use kb_core::{Config, Engine, Output};
use kb_display::frame::{Frame, Half};
use kb_display::screens::{self, Style};
use kb_display::ssd1306;
use kb_display::status::{Status, Wpm};

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    loop {}
}

struct State {
    engine: Engine<'static, { corne::KEYS }>,
    debounce: Debouncer<{ corne::KEYS }>,
    host: Reassembler,
    status: Status,
    wpm: Wpm,
    frames: [Frame; 2],
}

static mut STATE: MaybeUninit<State> = MaybeUninit::uninit();

#[entry]
fn main() -> ! {
    // SAFETY: single-threaded, the only reference to STATE.
    let s = unsafe {
        (*core::ptr::addr_of_mut!(STATE)).write(State {
            engine: Engine::new(&corne::KEYMAP, Config::default()),
            debounce: Debouncer::new(5, 5),
            host: Reassembler::new(),
            status: Status::default(),
            wpm: Wpm::new(),
            frames: [Frame::default(), Frame::default()],
        })
    };
    let mut now = 0u32;
    loop {
        let raw: [bool; corne::KEYS] = black_box([false; corne::KEYS]);
        let engine = &mut s.engine;
        s.debounce.update(&raw, now, |pos, down| {
            if down {
                engine.press(pos, now);
            } else {
                engine.release(pos, now);
            }
        });
        s.engine.tick(now);
        while let Some(out) = s.engine.poll() {
            match out {
                Output::Report(r) => {
                    black_box(r.nkro_bitmap());
                    black_box(r.boot_keys());
                    s.wpm.key_pressed(now);
                }
                Output::Layer(l) => s.status.layer = l,
                Output::CapsWord(on) => s.status.caps_word = on,
                Output::System(a) => s.status.apply(a),
                Output::Mouse(action, down) => {
                    black_box((action, down));
                }
                Output::Pointer(on) => {
                    black_box(on);
                }
            }
        }

        let report: [u8; REPORT_SIZE] = black_box([0; REPORT_SIZE]);
        match s.host.feed(&report) {
            Some(HostMessage::Notify(text)) => s.status.notification = Some(text),
            Some(HostMessage::Clear) => s.status.notification = None,
            None => {}
        }

        s.status.wpm = s.wpm.wpm();
        let style = if black_box(now) & 1 == 0 {
            Style::Portrait
        } else {
            Style::Zmk
        };
        for (i, half) in [Half::Left, Half::Right].into_iter().enumerate() {
            screens::render(
                &mut s.frames[i],
                half,
                style,
                &s.status,
                corne::KEYMAP.layer_names,
            );
            ssd1306::init(|w| {
                black_box(w);
            });
            ssd1306::flush(&s.frames[i], |w| {
                black_box(w);
            });
        }
        now = now.wrapping_add(1);
    }
}
