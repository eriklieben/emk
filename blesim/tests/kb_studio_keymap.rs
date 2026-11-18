//! The firmware's Bluetooth (`kb-ble`) keeps Studio's edited keymap in the
//! bond storage: a saved edit is back after a restart, next to the bonds.

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

#[test]
fn saved_studio_edit_survives_a_restart() {
    // Arrange: an edit saved on the first start
    let mut flash = RamFlash::new();
    let hid_server = kb_ble::hid::hid_server("left");
    let never = Stop::new();
    let studio: &'static CorneStudio = Box::leak(Box::new(SharedStudio::new(&corne::KEYMAP)));
    let left = left_state(studio);
    let air = Air::new();
    let stored = run(
        left_half(&air, left, &mut flash, &hid_server, &never),
        async {
            studio.with(|s| {
                s.unlock(0);
                s.set(1, 3, kp(Z), 0).unwrap();
                s.save(0).unwrap();
            });
            wait_until(Duration::from_secs(2), || {
                log_text(left.log).contains("keymap: 1 record(s) stored")
            })
            .await
        },
    );
    dump("left", left.log);
    let restarted_studio: &'static CorneStudio =
        Box::leak(Box::new(SharedStudio::new(&corne::KEYMAP)));
    let restarted = left_state(restarted_studio);
    let air = Air::new();

    // Act: start again on the same flash
    let applied = run(
        left_half(&air, restarted, &mut flash, &hid_server, &never),
        async {
            wait_until(Duration::from_secs(2), || {
                log_text(restarted.log).contains("studio record(s) read")
            })
            .await
        },
    );
    dump("left after the restart", restarted.log);

    // Assert
    assert!(stored, "the record was written");
    assert!(applied, "the record was read back");
    let (binding, dirty) = restarted_studio.with(|s| (s.binding(1, 3), s.dirty()));
    assert_eq!(binding, Some(kp(Z)));
    assert!(!dirty, "what came from flash is saved");
}
