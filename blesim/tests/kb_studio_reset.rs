//! Studio's reset clears the edited keymap in flash too: after a restart
//! the keyboard has its compiled keymap again.

mod common;
mod kb;

use blesim::Air;
use embassy_time::Duration;
use kb::*;
use kb_ble::LeftState;
use kb_ble::keymap::SharedStudio;
use kb_core::action::kp;
use kb_core::keymaps::corne;
use kb_core::keys::Z;

type CorneStudio = SharedStudio<{ corne::KEYS }, 4>;

fn left_state(studio: &'static CorneStudio) -> &'static LeftState<'static> {
    let state: &'static LeftState<'static> = Box::leak(Box::new(LeftState::new(Box::leak(
        Box::new(kb_ble::log::LogRing::new()),
    ))));
    state.keymap.attach(studio);
    state
}

fn start(flash: &mut RamFlash, server: &kb_ble::hid::HidServer<'_>) -> &'static CorneStudio {
    let studio: &'static CorneStudio = Box::leak(Box::new(SharedStudio::new(&corne::KEYMAP)));
    let left = left_state(studio);
    let never = Stop::new();
    let air = Air::new();
    let loaded = run(left_half(&air, left, flash, server, &never), async {
        wait_until(Duration::from_secs(2), || {
            log_text(left.log).contains("keymap: 1 studio record(s) read")
        })
        .await
    });
    dump("left", left.log);
    assert!(loaded, "the cleared record was read");
    studio
}

#[test]
fn studio_reset_clears_the_stored_keymap() {
    // Arrange: an edit saved, then reset on the next start
    let mut flash = RamFlash::new();
    let hid_server = kb_ble::hid::hid_server("left");
    let never = Stop::new();
    let first: &'static CorneStudio = Box::leak(Box::new(SharedStudio::new(&corne::KEYMAP)));
    let left = left_state(first);
    let air = Air::new();
    run(
        left_half(&air, left, &mut flash, &hid_server, &never),
        async {
            first.with(|s| {
                s.unlock(0);
                s.set(0, 1, kp(Z), 0).unwrap();
                s.save(0).unwrap();
            });
            wait_until(Duration::from_secs(2), || {
                log_text(left.log).contains("record(s) stored")
            })
            .await;
        },
    );
    let second: &'static CorneStudio = Box::leak(Box::new(SharedStudio::new(&corne::KEYMAP)));
    let left = left_state(second);
    let air = Air::new();
    let edited_after_restart = run(
        left_half(&air, left, &mut flash, &hid_server, &never),
        async {
            wait_until(Duration::from_secs(2), || {
                log_text(left.log).contains("studio record(s) read")
            })
            .await;
            let edited = second.with(|s| s.binding(0, 1));
            second.with(|s| {
                s.unlock(0);
                s.reset(0).unwrap();
            });
            wait_until(Duration::from_secs(2), || {
                log_text(left.log).contains("record(s) stored")
            })
            .await;
            edited
        },
    );
    dump("left (reset)", left.log);

    // Act
    let third = start(&mut flash, &hid_server);

    // Assert
    assert_eq!(edited_after_restart, Some(kp(Z)));
    assert_eq!(&third.with(|s| *s.layers())[..], &corne::LAYERS[..]);
}
