//! Plan modifier transitions without publishing native input.
use super::{XMods, to_cgevent_flags};
use core_graphics::event::{CGEventFlags, CGKeyCode};
use input_event::scancode::Linux;
use std::{cell::Cell, collections::HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum KeyAction {
    Modifier {
        code: CGKeyCode,
        flags: CGEventFlags,
    },
    Repeat {
        code: CGKeyCode,
        state: u8,
    },
}

#[derive(Clone, Copy)]
struct Definition {
    key: u32,
    code: CGKeyCode,
    mask: XMods,
    device: u64,
    momentary: bool,
}
const DEFINITIONS: [Definition; 9] = [
    Definition {
        key: Linux::KeyLeftShift as u32,
        code: 56,
        mask: XMods::ShiftMask,
        device: 2,
        momentary: true,
    },
    Definition {
        key: Linux::KeyRightShift as u32,
        code: 60,
        mask: XMods::ShiftMask,
        device: 4,
        momentary: true,
    },
    Definition {
        key: Linux::KeyLeftCtrl as u32,
        code: 59,
        mask: XMods::ControlMask,
        device: 1,
        momentary: true,
    },
    Definition {
        key: Linux::KeyRightCtrl as u32,
        code: 62,
        mask: XMods::ControlMask,
        device: 0x2000,
        momentary: true,
    },
    Definition {
        key: Linux::KeyLeftAlt as u32,
        code: 58,
        mask: XMods::Mod1Mask,
        device: 0x20,
        momentary: true,
    },
    Definition {
        key: Linux::KeyRightalt as u32,
        code: 61,
        mask: XMods::Mod1Mask,
        device: 0x40,
        momentary: true,
    },
    Definition {
        key: Linux::KeyLeftMeta as u32,
        code: 55,
        mask: XMods::Mod4Mask,
        device: 8,
        momentary: true,
    },
    Definition {
        key: Linux::KeyRightmeta as u32,
        code: 54,
        mask: XMods::Mod4Mask,
        device: 0x10,
        momentary: true,
    },
    Definition {
        key: Linux::KeyCapsLock as u32,
        code: 57,
        mask: XMods::LockMask,
        device: 0,
        momentary: false,
    },
];

pub(super) fn raw_actions(
    current: &Cell<XMods>,
    held: &mut HashSet<u32>,
    key: u32,
    code: CGKeyCode,
    state: u8,
) -> Vec<KeyAction> {
    let mut actions = Vec::with_capacity(2);
    if let Some(def) = DEFINITIONS.iter().find(|d| d.key == key) {
        if state == 1 {
            held.insert(key);
        } else {
            held.remove(&key);
        }
        let mut mods = current.get();
        if DEFINITIONS
            .iter()
            .any(|d| d.mask == def.mask && held.contains(&d.key))
        {
            mods.insert(def.mask);
        } else {
            mods.remove(def.mask);
        }
        current.set(mods);
        actions.push(KeyAction::Modifier {
            code: def.code,
            flags: flags(mods, held),
        });
        // Modifier keys change state once. Sending them through the repeating
        // ordinary-key task can release the wrong key and strand native flags.
        if def.momentary {
            return actions;
        }
    }
    actions.push(KeyAction::Repeat { code, state });
    actions
}

fn flags(mods: XMods, held: &HashSet<u32>) -> CGEventFlags {
    let device = DEFINITIONS
        .iter()
        .filter(|d| held.contains(&d.key))
        .fold(0, |bits, d| bits | d.device);
    CGEventFlags::from_bits_retain(to_cgevent_flags(mods).bits() | device)
}

pub(super) fn absolute_actions(
    current: &Cell<XMods>,
    held: &mut HashSet<u32>,
    depressed: u32,
) -> Vec<KeyAction> {
    let target = XMods::from_bits(depressed).unwrap_or_default();
    let mut actions = Vec::new();
    // Release known sides individually. Each native FlagsChanged must name
    // the actual key whose state changed, even when another side stays down.
    for def in &DEFINITIONS {
        if held.contains(&def.key) && !target.contains(def.mask) {
            actions.extend(
                raw_actions(current, held, def.key, def.code, 0)
                    .into_iter()
                    .filter(|a| matches!(a, KeyAction::Modifier { .. })),
            );
        }
    }
    for def in &DEFINITIONS {
        if target.contains(def.mask)
            && !DEFINITIONS
                .iter()
                .any(|d| d.mask == def.mask && held.contains(&d.key))
        {
            actions.extend(
                raw_actions(current, held, def.key, def.code, 1)
                    .into_iter()
                    .filter(|a| matches!(a, KeyAction::Modifier { .. })),
            );
        }
    }
    current.set(target);
    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    fn act(
        current: &Cell<XMods>,
        held: &mut HashSet<u32>,
        key: Linux,
        state: u8,
    ) -> Vec<KeyAction> {
        let code = DEFINITIONS
            .iter()
            .find(|d| d.key == key as u32)
            .map_or(8, |d| d.code);
        raw_actions(current, held, key as u32, code, state)
    }
    #[test]
    fn modifier_release_during_a_regular_key_never_cancels_its_repeat() {
        let current = Cell::new(XMods::empty());
        let mut held = HashSet::new();
        act(&current, &mut held, Linux::KeyLeftMeta, 1);
        assert_eq!(
            act(&current, &mut held, Linux::KeyC, 1),
            vec![KeyAction::Repeat { code: 8, state: 1 }]
        );
        assert_eq!(
            act(&current, &mut held, Linux::KeyLeftMeta, 0),
            vec![KeyAction::Modifier {
                code: 55,
                flags: CGEventFlags::empty()
            }]
        );
        assert_eq!(
            act(&current, &mut held, Linux::KeyC, 0),
            vec![KeyAction::Repeat { code: 8, state: 0 }]
        );
    }
    #[test]
    fn every_momentary_modifier_has_its_own_down_and_up_without_repeat() {
        for def in DEFINITIONS.iter().filter(|d| d.momentary) {
            let current = Cell::new(XMods::empty());
            let mut held = HashSet::new();
            let down = raw_actions(&current, &mut held, def.key, def.code, 1);
            assert_eq!(
                down,
                vec![KeyAction::Modifier {
                    code: def.code,
                    flags: CGEventFlags::from_bits_retain(
                        to_cgevent_flags(def.mask).bits() | def.device
                    )
                }]
            );
            assert_eq!(
                raw_actions(&current, &mut held, def.key, def.code, 0),
                vec![KeyAction::Modifier {
                    code: def.code,
                    flags: CGEventFlags::empty()
                }]
            );
        }
    }
    #[test]
    fn releasing_one_side_keeps_the_other_side_and_its_device_flag() {
        let current = Cell::new(XMods::empty());
        let mut held = HashSet::new();
        act(&current, &mut held, Linux::KeyLeftMeta, 1);
        act(&current, &mut held, Linux::KeyRightmeta, 1);
        assert_eq!(
            act(&current, &mut held, Linux::KeyLeftMeta, 0),
            vec![KeyAction::Modifier {
                code: 55,
                flags: CGEventFlags::from_bits_retain(
                    CGEventFlags::CGEventFlagCommand.bits() | 0x10
                )
            }]
        );
        assert_eq!(
            act(&current, &mut held, Linux::KeyRightmeta, 0),
            vec![KeyAction::Modifier {
                code: 54,
                flags: CGEventFlags::empty()
            }]
        );
    }
    #[test]
    fn absolute_release_releases_the_known_right_side_with_its_identity() {
        let current = Cell::new(XMods::empty());
        let mut held = HashSet::new();
        act(&current, &mut held, Linux::KeyRightmeta, 1);
        assert_eq!(
            absolute_actions(&current, &mut held, 0),
            vec![KeyAction::Modifier {
                code: 54,
                flags: CGEventFlags::empty()
            }]
        );
        assert!(held.is_empty());
    }
    #[test]
    fn a_thousand_chords_leave_no_modifier_or_device_bits_held() {
        for i in 0..1000 {
            let current = Cell::new(XMods::empty());
            let mut held = HashSet::new();
            let d = &DEFINITIONS[i % 8];
            let down = raw_actions(&current, &mut held, d.key, d.code, 1);
            assert!(!down.iter().any(|a| matches!(a, KeyAction::Repeat { .. })));
            act(&current, &mut held, Linux::KeyC, 1);
            let up = raw_actions(&current, &mut held, d.key, d.code, 0);
            assert_eq!(
                up,
                vec![KeyAction::Modifier {
                    code: d.code,
                    flags: CGEventFlags::empty()
                }]
            );
            act(&current, &mut held, Linux::KeyC, 0);
            assert!(current.get().is_empty() && held.is_empty());
        }
    }
}
