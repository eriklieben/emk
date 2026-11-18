//! The lockout guard: a keymap Studio applies must keep a `StudioUnlock`
//! key that can actually be pressed, or the keyboard could never be edited
//! (or reset) again without flashing it.
//!
//! "Reachable" is checked the way fingers work: starting from the base
//! layer, hold keys that resolve to `&mo` (each physical key can be held only
//! once; conditional layers switch on as in the engine) and look for a key
//! that is not held and resolves to `StudioUnlock` on the active layers. So
//! moving the unlock key onto the position of the `&mo` that leads to its
//! layer is refused, as is removing the last way to that layer.
//!
//! Combos count too: one active on the highest active layer whose keys are
//! all free can be pressed, and if it is `StudioUnlock` that is enough; if it
//! is an `&mo`, its keys are held for that layer. (A combo never takes a
//! key away: pressed alone, a combo key goes through after the combo
//! timeout.) The search is exhaustive but bounded: a keymap too tangled to
//! check within [`BUDGET`] steps counts as unsafe.

use crate::action::{Action, SystemAction};
use crate::engine::{ComboDef, ConditionalLayer};

/// Most search steps before giving up.
pub const BUDGET: u32 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    /// A `StudioUnlock` key can be pressed.
    Yes,
    /// None can.
    No,
    /// Too tangled to tell within [`BUDGET`]: treat as [`Reach::No`].
    TooComplex,
}

/// Whether a `StudioUnlock` key (or combo) can be pressed with `layers` and `combos`.
#[must_use]
pub fn unlock_reachable<const N: usize>(
    layers: &[[Action; N]],
    conditional: &[ConditionalLayer<'_>],
    combos: &[ComboDef],
) -> Reach {
    let mut search = Search {
        layers,
        conditional,
        combos,
        budget: BUDGET,
        dead: [(0, 0); DEAD_ENDS],
        dead_len: 0,
    };
    search.from(1, 0)
}

/// Dead ends remembered (momentary layers, keys held) to skip repeats.
const DEAD_ENDS: usize = 64;

struct Search<'k, 'c, const N: usize> {
    layers: &'k [[Action; N]],
    conditional: &'k [ConditionalLayer<'c>],
    combos: &'k [ComboDef],
    budget: u32,
    dead: [(u32, u64); DEAD_ENDS],
    dead_len: usize,
}

impl<const N: usize> Search<'_, '_, N> {
    /// Active layers for the momentary ones, as `Engine::update_layers`.
    fn active(&self, momentary: u32) -> u32 {
        let mut layers = momentary;
        for c in self.conditional {
            let needed = c
                .if_layers
                .iter()
                .filter(|&&l| l < 32)
                .fold(0u32, |m, &l| m | 1 << l);
            if needed != 0 && momentary & needed == needed && c.then_layer < 32 {
                layers |= 1 << c.then_layer;
            }
        }
        layers
    }

    /// The action at `pos` with `active` layers, as `Engine::lookup`.
    fn resolve(&self, active: u32, pos: usize) -> Action {
        for layer in (0..self.layers.len()).rev() {
            if active & (1 << layer) != 0 {
                let action = self.layers[layer][pos];
                if action != Action::Trans {
                    return action;
                }
            }
        }
        Action::None
    }

    /// Already known to lead nowhere: the same layers with fewer keys held.
    fn known_dead(&self, momentary: u32, held: u64) -> bool {
        self.dead[..self.dead_len]
            .iter()
            .any(|&(m, h)| m == momentary && h & held == h)
    }

    /// Whether the unlock key can be pressed starting with `momentary`
    /// layers on and the `held` keys held.
    fn from(&mut self, momentary: u32, held: u64) -> Reach {
        if self.budget == 0 {
            return Reach::TooComplex;
        }
        self.budget -= 1;
        if self.known_dead(momentary, held) {
            return Reach::No;
        }
        if self.unlock_at_hand(momentary, held) {
            return Reach::Yes;
        }
        let result = self.through_layers(momentary, held);
        if result == Reach::No && self.dead_len < DEAD_ENDS {
            self.dead[self.dead_len] = (momentary, held);
            self.dead_len += 1;
        }
        result
    }

    /// Whether a free key, or a combo that can be pressed, is the unlock key
    /// right away.
    fn unlock_at_hand(&self, momentary: u32, held: u64) -> bool {
        const UNLOCK: Action = Action::System(SystemAction::StudioUnlock);
        let active = self.active(momentary);
        if (0..N).any(|pos| held & (1 << pos) == 0 && self.resolve(active, pos) == UNLOCK) {
            return true;
        }
        let highest = Self::highest(active);
        self.combos
            .iter()
            .filter(|c| Self::pressable(c, held, highest))
            .any(|c| c.action == UNLOCK)
    }

    /// The search on, through each momentary layer that a combo that can be
    /// pressed or a free key switches on.
    fn through_layers(&mut self, momentary: u32, held: u64) -> Reach {
        let active = self.active(momentary);
        let highest = Self::highest(active);
        let mut result = Reach::No;
        let combos = self.combos;
        for c in combos.iter().filter(|c| Self::pressable(c, held, highest)) {
            if let Some(layer) = self.new_layer(c.action, momentary) {
                match self.from(momentary | 1 << layer, held | c.keys) {
                    Reach::Yes => return Reach::Yes,
                    Reach::TooComplex => result = Reach::TooComplex,
                    Reach::No => {}
                }
            }
        }
        for pos in (0..N).filter(|&p| held & (1 << p) == 0) {
            if let Some(layer) = self.new_layer(self.resolve(active, pos), momentary) {
                match self.from(momentary | 1 << layer, held | 1 << pos) {
                    Reach::Yes => return Reach::Yes,
                    Reach::TooComplex => result = Reach::TooComplex,
                    Reach::No => {}
                }
            }
        }
        result
    }

    /// The highest of the `active` layers.
    fn highest(active: u32) -> u8 {
        31 - active.leading_zeros() as u8
    }

    /// Whether combo `c` can be pressed: it has keys, all of them on the
    /// keyboard and none `held`, and it is on for layer `highest`.
    fn pressable(c: &ComboDef, held: u64, highest: u8) -> bool {
        let positions = if N >= 64 { u64::MAX } else { (1u64 << N) - 1 };
        c.keys != 0 && c.keys & !positions == 0 && c.keys & held == 0 && c.enabled_on(highest)
    }

    /// The layer `action` switches on, if it is a momentary layer that exists
    /// and isn't on in `momentary` yet.
    fn new_layer(&self, action: Action, momentary: u32) -> Option<u8> {
        if let Action::MomentaryLayer(layer) = action
            && (layer as usize) < self.layers.len()
            && momentary & (1 << layer) == 0
        {
            Some(layer)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{NONE, TRANS, kp, mo, sys};
    use crate::keymaps::corne;
    use crate::keys::A;

    const UNLOCK: Action = sys(SystemAction::StudioUnlock);

    fn corne() -> Vec<[Action; corne::KEYS]> {
        corne::LAYERS.to_vec()
    }

    fn corne_combos() -> Vec<ComboDef> {
        corne::COMBOS
            .iter()
            .map(crate::engine::Combo::def)
            .collect()
    }

    fn combo(keys: u64, action: Action, layers: u32) -> ComboDef {
        ComboDef {
            keys,
            action,
            layers,
            timeout_ms: 50,
            require_prior_idle_ms: 0,
        }
    }

    #[test]
    fn unlock_combo_on_the_base_layer_is_reachable() {
        // Arrange: no unlock key, but a combo for it
        let mut layers = corne();
        layers[2][35] = TRANS;
        let combos = [combo(0b11, UNLOCK, 1)];

        // Act
        let reach = unlock_reachable(&layers, &corne::CONDITIONAL_LAYERS, &combos);

        // Assert
        assert_eq!(reach, Reach::Yes);
    }

    #[test]
    fn unlock_combo_on_another_layer_alone_is_not_enough() {
        // Arrange: the combo only works on nav, and nothing leads there
        let mut layers = corne();
        layers[2][35] = TRANS;
        layers[0][41] = kp(A);
        let combos = [combo(0b11, UNLOCK, 1 << 2)];

        // Act
        let reach = unlock_reachable(&layers, &corne::CONDITIONAL_LAYERS, &combos);

        // Assert
        assert_eq!(reach, Reach::No);
    }

    #[test]
    fn combo_that_holds_a_layer_leads_to_the_unlock() {
        // Arrange: nav only through a combo of 0 + 1
        let mut layers = corne();
        layers[0][41] = kp(A);
        let combos = [combo(0b11, mo(2), 1)];

        // Act
        let reach = unlock_reachable(&layers, &corne::CONDITIONAL_LAYERS, &combos);

        // Assert
        assert_eq!(reach, Reach::Yes);
    }

    #[test]
    fn combo_needing_the_unlock_position_does_not_count() {
        // Arrange: nav through a combo that includes position 35 itself
        let mut layers = corne();
        layers[0][41] = kp(A);
        let combos = [combo(1 << 35 | 1 << 34, mo(2), 1)];

        // Act
        let reach = unlock_reachable(&layers, &corne::CONDITIONAL_LAYERS, &combos);

        // Assert
        assert_eq!(reach, Reach::No);
    }

    #[test]
    fn combo_with_positions_past_the_keyboard_does_not_count() {
        // Arrange
        let mut layers = corne();
        layers[2][35] = TRANS;
        let combos = [combo(1 << 50 | 1, UNLOCK, 0)];

        // Act
        let reach = unlock_reachable(&layers, &corne::CONDITIONAL_LAYERS, &combos);

        // Assert
        assert_eq!(reach, Reach::No);
    }

    #[test]
    fn combo_checks_the_highest_layer() {
        // Arrange: an unlock combo for the base layer only, tried from symbols
        // after the base key positions are gone (nothing on base itself)
        let mut layers = [[NONE; 4]; 2];
        layers[0][0] = mo(1);
        let combos = [combo(0b0110, UNLOCK, 1 << 1)];

        // Act
        let reach = unlock_reachable(&layers, &[], &combos);

        // Assert: reachable from layer 1 while 0 is held
        assert_eq!(reach, Reach::Yes);
    }

    #[test]
    fn compiled_combos_change_nothing_for_the_corne() {
        // Arrange
        let layers = corne();

        // Act
        let reach = unlock_reachable(&layers, &corne::CONDITIONAL_LAYERS, &corne_combos());

        // Assert
        assert_eq!(reach, Reach::Yes);
    }

    #[test]
    fn compiled_corne_keymap_can_be_unlocked() {
        // Arrange
        let layers = corne();

        // Act
        let reach = unlock_reachable(&layers, &corne::CONDITIONAL_LAYERS, &corne_combos());

        // Assert
        assert_eq!(reach, Reach::Yes);
    }

    #[test]
    fn removing_the_unlock_key_locks_out() {
        // Arrange
        let mut layers = corne();
        layers[2][35] = TRANS;

        // Act
        let reach = unlock_reachable(&layers, &corne::CONDITIONAL_LAYERS, &corne_combos());

        // Assert
        assert_eq!(reach, Reach::No);
    }

    #[test]
    fn removing_the_only_way_to_nav_locks_out() {
        // Arrange
        let mut layers = corne();
        layers[0][41] = kp(A);

        // Act
        let reach = unlock_reachable(&layers, &corne::CONDITIONAL_LAYERS, &corne_combos());

        // Assert
        assert_eq!(reach, Reach::No);
    }

    #[test]
    fn unlock_under_the_held_layer_key_locks_out() {
        // Arrange: nav's unlock moved onto the key that holds nav
        let mut layers = corne();
        layers[2][35] = TRANS;
        layers[2][41] = UNLOCK;

        // Act
        let reach = unlock_reachable(&layers, &corne::CONDITIONAL_LAYERS, &corne_combos());

        // Assert
        assert_eq!(reach, Reach::No);
    }

    #[test]
    fn another_way_to_nav_keeps_it_reachable() {
        // Arrange: the nav key moves to another thumb
        let mut layers = corne();
        layers[0][41] = kp(A);
        layers[0][39] = mo(2);

        // Act
        let reach = unlock_reachable(&layers, &corne::CONDITIONAL_LAYERS, &corne_combos());

        // Assert
        assert_eq!(reach, Reach::Yes);
    }

    #[test]
    fn unlock_on_the_base_layer_is_reachable() {
        // Arrange
        let mut layers = corne();
        layers[2][35] = TRANS;
        layers[0][0] = UNLOCK;

        // Act
        let reach = unlock_reachable(&layers, &corne::CONDITIONAL_LAYERS, &corne_combos());

        // Assert
        assert_eq!(reach, Reach::Yes);
    }

    #[test]
    fn unlock_through_a_conditional_layer_is_reachable() {
        // Arrange: only on wm (symbols + nav), on a key no layer key uses
        let mut layers = corne();
        layers[2][35] = TRANS;
        layers[3][11] = UNLOCK;

        // Act
        let reach = unlock_reachable(&layers, &corne::CONDITIONAL_LAYERS, &corne_combos());

        // Assert
        assert_eq!(reach, Reach::Yes);
    }

    #[test]
    fn unlock_reached_through_a_chain_of_layers() {
        // Arrange: base → layer 1 → layer 2, unlock on 2
        let mut layers = [[NONE; 4]; 3];
        layers[0][0] = mo(1);
        layers[1][1] = mo(2);
        layers[2][2] = UNLOCK;

        // Act
        let reach = unlock_reachable(&layers, &[], &[]);

        // Assert
        assert_eq!(reach, Reach::Yes);
    }

    #[test]
    fn layer_key_that_needs_its_own_position_twice_does_not_count() {
        // Arrange: base pos 0 = mo(1); on layer 1, pos 0 = mo(2), unlock on 2
        let mut layers = [[NONE; 4]; 3];
        layers[0][0] = mo(1);
        layers[1][0] = mo(2);
        layers[2][2] = UNLOCK;

        // Act
        let reach = unlock_reachable(&layers, &[], &[]);

        // Assert
        assert_eq!(reach, Reach::No);
    }

    #[test]
    fn a_maze_of_layer_keys_is_too_complex() {
        // Arrange: every key holds some layer, no unlock anywhere
        let mut layers = [[NONE; 42]; 8];
        for (l, layer) in layers.iter_mut().enumerate() {
            for (p, a) in layer.iter_mut().enumerate() {
                *a = mo(((l + p) % 8) as u8);
            }
        }

        // Act
        let reach = unlock_reachable(&layers, &[], &[]);

        // Assert
        assert_eq!(reach, Reach::TooComplex);
    }
}
