//! Local modifier state follows individual hardware-origin transitions only.
//! Quartz's global key table also accepts synthetic input and is not a device
//! inventory. Never use it to reconstruct modifiers for remote pointer input.
use super::LocalPointerModifiers;
use core_foundation::{
    base::TCFType,
    mach_port::CFMachPortRef,
    runloop::{CFRunLoop, kCFRunLoopDefaultMode},
};
use core_graphics::event::{
    CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventType, CallbackResult, EventField,
};
use std::{
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

fn device_bit(code: u16) -> Option<u64> {
    Some(match code {
        59 => 1,
        56 => 2,
        60 => 4,
        55 => 8,
        54 => 0x10,
        58 => 0x20,
        61 => 0x40,
        62 => 0x2000,
        57 => CGEventFlags::CGEventFlagAlphaShift.bits(),
        63 => CGEventFlags::CGEventFlagSecondaryFn.bits(),
        _ => return None,
    })
}

#[derive(Default)]
struct PhysicalModifiers {
    held: AtomicU64,
}
impl PhysicalModifiers {
    fn observe(&self, source_pid: i64, code: u16, flags: u64) {
        // This is input provenance, not a security boundary: virtual HID can
        // present as hardware. Ordinary CGEvent posters (including us) have a PID.
        if source_pid != 0 {
            return;
        }
        let Some(bit) = device_bit(code) else {
            return;
        };
        // Only the side named by this event changes. Aggregate flags and other
        // device bits may have been contaminated by a different event source.
        if flags & bit != 0 {
            self.held.fetch_or(bit, Ordering::Relaxed);
        } else {
            self.held.fetch_and(!bit, Ordering::Relaxed);
        }
    }
    fn clear(&self) {
        self.held.store(0, Ordering::Relaxed);
    }
    fn sample(&self) -> LocalPointerModifiers {
        let bits = self.held.load(Ordering::Relaxed);
        LocalPointerModifiers::from_keys(
            |code| device_bit(code).is_some_and(|bit| bits & bit != 0),
            bits & CGEventFlags::CGEventFlagAlphaShift.bits() != 0,
        )
    }
}

pub(super) struct LocalModifierMonitor {
    state: Arc<PhysicalModifiers>,
    stop: Arc<AtomicBool>,
    runloop: Option<CFRunLoop>,
}
impl LocalModifierMonitor {
    pub(super) fn new() -> Self {
        let state = Arc::new(PhysicalModifiers::default());
        let stop = Arc::new(AtomicBool::new(false));
        let observer = state.clone();
        let stopped = stop.clone();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("local-modifiers".into())
            .spawn(move || {
                let port: Arc<OnceLock<usize>> = Arc::new(OnceLock::new());
                let callback_port = port.clone();
                let callback_state = observer.clone();
                let callback_stop = stopped.clone();
                let tap = CGEventTap::new(
                    CGEventTapLocation::Session,
                    CGEventTapPlacement::HeadInsertEventTap,
                    CGEventTapOptions::ListenOnly,
                    vec![CGEventType::FlagsChanged],
                    move |_, event_type, event| {
                        if callback_stop.load(Ordering::Relaxed) {
                            return CallbackResult::Keep;
                        }
                        if matches!(
                            event_type,
                            CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput
                        ) {
                            // Lost events must not leave a modifier held. Timeout
                            // recovery never changes or consumes the original event.
                            callback_state.clear();
                            if matches!(event_type, CGEventType::TapDisabledByTimeout) {
                                if let Some(port) = callback_port.get() {
                                    unsafe {
                                        CGEventTapEnable(*port as CFMachPortRef, true);
                                    }
                                }
                            }
                        } else if matches!(event_type, CGEventType::FlagsChanged) {
                            callback_state.observe(
                                event.get_integer_value_field(
                                    EventField::EVENT_SOURCE_UNIX_PROCESS_ID,
                                ),
                                event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE)
                                    as u16,
                                event.get_flags().bits(),
                            );
                        }
                        CallbackResult::Keep
                    },
                );
                let Ok(tap) = tap else {
                    let _ = ready_tx.send(None);
                    return;
                };
                let _ = port.set(tap.mach_port().as_concrete_TypeRef() as usize);
                let Ok(source) = tap.mach_port().create_runloop_source(0) else {
                    let _ = ready_tx.send(None);
                    return;
                };
                let runloop = CFRunLoop::get_current();
                unsafe {
                    runloop.add_source(&source, kCFRunLoopDefaultMode);
                }
                tap.enable();
                if ready_tx.send(Some(runloop.clone())).is_err() {
                    return;
                }
                // The timeout bounds teardown even if stop races run-loop entry.
                // No input is queued, rewritten, logged or sent by this observer.
                while !stopped.load(Ordering::Relaxed) {
                    unsafe {
                        CFRunLoop::run_in_mode(
                            kCFRunLoopDefaultMode,
                            Duration::from_secs(1),
                            false,
                        );
                    }
                }
                observer.clear();
            });
        let runloop = match worker {
            Ok(_) => ready_rx.recv_timeout(Duration::from_secs(1)).ok().flatten(),
            Err(_) => None,
        };
        if runloop.is_none() {
            stop.store(true, Ordering::Relaxed);
            log::warn!("local modifier observer unavailable; local pointer chords remain neutral");
        } else {
            log::info!(
                "local modifier observer active (passive, hardware-origin transitions only)"
            );
        }
        Self {
            state,
            stop,
            runloop,
        }
    }
    pub(super) fn sample(&self) -> LocalPointerModifiers {
        self.state.sample()
    }
}
impl Drop for LocalModifierMonitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.state.clear();
        if let Some(runloop) = &self.runloop {
            runloop.stop();
        }
    }
}
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn synthetic_command_and_poisoned_aggregate_cannot_modify_local_pointer_state() {
        let s = PhysicalModifiers::default();
        s.observe(123, 55, 0x100008); // programmatic paste
        assert!(!s.sample().command);
        s.observe(0, 56, 0x12000a); // real Shift, polluted Command bits
        let m = s.sample();
        assert!(m.shift);
        assert!(!m.command);
        s.observe(0, 56, 0x100008); // release Shift while aggregate Cmd lingers
        let m = s.sample();
        assert!(!m.shift && !m.command);
    }
    #[test]
    fn synthetic_release_cannot_cancel_a_real_cross_device_chord() {
        let s = PhysicalModifiers::default();
        s.observe(0, 55, 0x100008);
        s.observe(456, 55, 0);
        assert!(s.sample().command);
        s.observe(0, 55, 0);
        assert!(!s.sample().command);
    }
    #[test]
    fn both_sides_remain_independent_through_press_and_release() {
        for (left, right, field) in [(55, 54, 0), (56, 60, 1), (59, 62, 2), (58, 61, 3)] {
            let s = PhysicalModifiers::default();
            let sample = |s: &PhysicalModifiers| {
                let m = s.sample();
                [m.command, m.shift, m.control, m.option][field]
            };
            s.observe(0, left, device_bit(left).unwrap());
            s.observe(0, right, device_bit(right).unwrap());
            s.observe(0, left, device_bit(right).unwrap());
            assert!(sample(&s));
            s.observe(0, right, 0);
            assert!(!sample(&s));
        }
    }
    #[test]
    fn lost_observation_clears_all_held_modifiers() {
        let s = PhysicalModifiers::default();
        for code in [55, 56, 59, 58, 63, 57] {
            s.observe(0, code, device_bit(code).unwrap());
        }
        s.clear();
        let m = s.sample();
        assert!(!m.command && !m.shift && !m.control && !m.option && !m.function && !m.caps_lock);
    }
    #[test]
    fn ordinary_keys_and_unknown_origins_do_not_change_state() {
        let s = PhysicalModifiers::default();
        for code in 0..128 {
            s.observe(99, code, u64::MAX);
        }
        s.observe(0, 8, u64::MAX);
        assert_eq!(s.held.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn caps_and_function_only_follow_their_own_physical_transition() {
        let s = PhysicalModifiers::default();
        s.observe(0, 57, 1 << 16);
        s.observe(0, 63, 1 << 23);
        s.observe(0, 56, 2);
        let m = s.sample();
        assert!(m.caps_lock && m.function && m.shift);
        s.observe(0, 57, 0);
        s.observe(0, 63, 0);
        let m = s.sample();
        assert!(!m.caps_lock && !m.function && m.shift);
    }
}
