//! The firmware's Bluetooth (`kb-ble`) keeps Studio's edited combos in the
//! bond storage too: a saved combo set, spread over several records, is
//! back after a restart, next to the key edits.

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
use kb_core::studio::combo::parse_spec;

type CorneStudio = SharedStudio<{ corne::KEYS }, 4>;

fn left_state(studio: &'static CorneStudio) -> &'static LeftState<'static> {
    let state: &'static LeftState<'static> = Box::leak(Box::new(LeftState::new(Box::leak(
        Box::new(kb_ble::log::LogRing::new()),
    ))));
    state.keymap.attach(studio);
    state
}

#[test]
fn saved_combos_survive_a_restart() {
    // Arrange: ten more combos (several records) and a key edit, saved
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
                for i in 0..10u8 {
                    let text = format!("long_combo_name {},{} 1 50 0 kp:Z", i, i + 1);
                    s.add_combo(&parse_spec(text.split(' ')).unwrap(), 0)
                        .unwrap();
                }
                s.remove_combo(0, 0).unwrap();
                s.set(0, 1, kp(Z), 0).unwrap();
                s.save(0).unwrap();
            });
            wait_until(Duration::from_secs(2), || {
                log_text(left.log).contains("record(s) stored")
            })
            .await
        },
    );
    dump("left", left.log);
    let expected = studio.with(|s| *s.combos());
    let restarted_studio: &'static CorneStudio =
        Box::leak(Box::new(SharedStudio::new(&corne::KEYMAP)));
    let restarted = left_state(restarted_studio);
    let air = Air::new();

    // Act
    let read = run(
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
    assert!(stored, "the records were written");
    assert!(read, "the records were read back");
    let (combos, binding, dirty) =
        restarted_studio.with(|s| (*s.combos(), s.binding(0, 1), s.dirty()));
    assert_eq!(combos, expected);
    assert_eq!(combos.len(), 15);
    assert_eq!(binding, Some(kp(Z)));
    assert!(!dirty);
    assert!(!log_text(restarted.log).contains("not used"));
}
