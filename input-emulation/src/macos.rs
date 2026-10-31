use super::{Emulation, EmulationHandle, error::EmulationError};
use async_trait::async_trait;
use bitflags::bitflags;
use core_graphics::base::CGFloat;
use core_graphics::display::{
    CGDirectDisplayID, CGDisplayBounds, CGGetDisplaysWithRect, CGPoint, CGRect, CGSize,
};
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, CGKeyCode, CGMouseButton, EventField,
    ScrollEventUnit,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use input_event::{
    BTN_BACK, BTN_FORWARD, BTN_LEFT, BTN_MIDDLE, BTN_RIGHT, Event, KeyboardEvent, PointerEvent,
    scancode,
};
use keycode::{KeyMap, KeyMapping};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};
use tokio::{sync::oneshot, task::JoinHandle};

use super::error::MacOSEmulationCreationError;

const DEFAULT_REPEAT_DELAY: Duration = Duration::from_millis(500);
const DEFAULT_REPEAT_INTERVAL: Duration = Duration::from_millis(32);
const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(500);

pub(crate) struct MacOSEmulation {
    /// global event source for all events
    event_source: CGEventSource,
    /// task handle for key repeats
    repeat_task: Option<JoinHandle<()>>,
    /// current state of the mouse buttons (tracked by evdev button code)
    pressed_buttons: HashSet<u32>,
    clicks: ClickTracker,
    /// current modifier state
    modifier_state: Rc<Cell<XMods>>,
    /// Persistent cancellation for this key only, including before first poll.
    repeat_cancel: Option<oneshot::Sender<()>>,
}

#[derive(Default)]
struct ClickTracker {
    previous: Option<(u32, CGPoint, Instant, i64)>,
    held: HashMap<u32, i64>,
}
impl ClickTracker {
    fn moved(&mut self, position: CGPoint) {
        if self.previous.is_some_and(|(_, p, _, _)| {
            (p.x - position.x).abs() > 4. || (p.y - position.y).abs() > 4.
        }) {
            self.previous = None;
        }
    }
    fn transition(&mut self, button: u32, state: u32, position: CGPoint, now: Instant) -> i64 {
        if state == 0 {
            return self.held.remove(&button).unwrap_or(1);
        }
        if let Some(count) = self.held.get(&button) {
            return *count;
        }
        let count = self
            .previous
            .filter(|(b, p, t, _)| {
                *b == button
                    && now.duration_since(*t) < DOUBLE_CLICK_INTERVAL
                    && (p.x - position.x).abs() <= 4.
                    && (p.y - position.y).abs() <= 4.
            })
            .map_or(1, |(_, _, _, n)| n.saturating_add(1));
        self.previous = Some((button, position, now, count));
        self.held.insert(button, count);
        count
    }
}

/// Maps an evdev button code to the CGEventType used for drag events.
fn drag_event_type(button: u32) -> CGEventType {
    match button {
        BTN_LEFT => CGEventType::LeftMouseDragged,
        BTN_RIGHT => CGEventType::RightMouseDragged,
        // middle, back, forward, and any other button all use OtherMouseDragged
        _ => CGEventType::OtherMouseDragged,
    }
}

// Protocol axes use positive down/right; Quartz wheel deltas use positive
// up/left. This conversion applies only to received input, never local devices
// or the user's macOS natural-scrolling preference.
fn quartz_scroll_wheels(axis: u8, value: i32) -> Option<(u32, i32, i32, i32)> {
    let delta = value.saturating_neg();
    match axis {
        0 => Some((1, delta, 0, 0)),
        1 => Some((2, 0, delta, 0)),
        _ => None,
    }
}

unsafe impl Send for MacOSEmulation {}

impl MacOSEmulation {
    pub(crate) fn new() -> Result<Self, MacOSEmulationCreationError> {
        request_macos_emulation_permissions()?;

        let event_source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
            .map_err(|_| MacOSEmulationCreationError::EventSourceCreation)?;
        Ok(Self {
            event_source,
            pressed_buttons: HashSet::new(),
            clicks: ClickTracker::default(),
            repeat_task: None,
            repeat_cancel: None,
            modifier_state: Rc::new(Cell::new(XMods::empty())),
        })
    }

    fn get_mouse_location(&self) -> Option<CGPoint> {
        let event: CGEvent = CGEvent::new(self.event_source.clone()).ok()?;
        Some(event.location())
    }

    async fn spawn_repeat_task(&mut self, key: u16) {
        // there can only be one repeating key and it's
        // always the last to be pressed
        self.cancel_repeat_task().await;
        // initial key event
        key_event(self.event_source.clone(), key, 1, self.modifier_state.get());
        // repeat task
        let event_source = self.event_source.clone();
        let (cancel, cancelled) = oneshot::channel();
        self.repeat_cancel = Some(cancel);
        let modifiers = self.modifier_state.clone();
        let repeat_task = tokio::task::spawn_local(repeat_key_events(
            cancelled,
            DEFAULT_REPEAT_DELAY,
            DEFAULT_REPEAT_INTERVAL,
            move |state| {
                key_event(event_source.clone(), key, state, modifiers.get());
            },
        ));
        self.repeat_task = Some(repeat_task);
    }

    async fn cancel_repeat_task(&mut self) {
        if let Some(cancel) = self.repeat_cancel.take() {
            let _ = cancel.send(());
        }
        if let Some(task) = self.repeat_task.take() {
            let _ = task.await;
        }
    }
}

// Run the repeat loop separately so cancellation timing can be tested without
// creating or posting any native input events.
async fn repeat_key_events(
    mut cancelled: oneshot::Receiver<()>,
    delay: Duration,
    interval: Duration,
    mut emit: impl FnMut(u8),
) {
    let mut wait = delay;
    loop {
        tokio::select! {
            // A release already sent must win even if the timer is also ready.
            biased;
            _ = &mut cancelled => break,
            _ = tokio::time::sleep(wait) => emit(1),
        }
        wait = interval;
    }
    // Preserve the unconditional key-up requested in @feschber's review.
    // The captured code is a Mac CGKeyCode: never translate it as a Linux
    // scancode or change modifiers here; the real key release owns that state.
    emit(0);
}

fn request_macos_emulation_permissions() -> Result<(), MacOSEmulationCreationError> {
    // Request both permissions up front so the user sees both TCC prompts
    // on the first launch. See the matching comment in input-capture/src/
    // macos.rs::request_macos_capture_permissions for the rationale.
    let accessibility = request_accessibility_permission();
    let input_control = request_input_control_permission();

    if !accessibility {
        return Err(MacOSEmulationCreationError::AccessibilityPermission);
    }
    if !input_control {
        return Err(MacOSEmulationCreationError::InputControlPermission);
    }
    Ok(())
}

fn request_accessibility_permission() -> bool {
    // Silent check. The GUI owns the one-time user-visible prompt at
    // startup (see lan_mouse_gtk::macos_privacy).
    unsafe { AXIsProcessTrusted() }
}

fn request_input_control_permission() -> bool {
    unsafe { CGPreflightPostEventAccess() }
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightPostEventAccess() -> bool;
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

/// Mac virtual key codes for the four arrow keys.
const MAC_KEY_LEFT: u16 = 0x7B;
const MAC_KEY_RIGHT: u16 = 0x7C;
const MAC_KEY_DOWN: u16 = 0x7D;
const MAC_KEY_UP: u16 = 0x7E;

fn is_arrow_key(key: u16) -> bool {
    matches!(
        key,
        MAC_KEY_LEFT | MAC_KEY_RIGHT | MAC_KEY_DOWN | MAC_KEY_UP
    )
}

fn key_event(event_source: CGEventSource, key: u16, state: u8, modifiers: XMods) {
    let event = match CGEvent::new_keyboard_event(event_source, key, state != 0) {
        Ok(e) => e,
        Err(_) => {
            log::warn!("unable to create key event");
            return;
        }
    };
    let mut flags = to_cgevent_flags(modifiers);
    // Hardware-generated arrow keys on macOS carry NumericPad + SecondaryFn.
    // CGEventTap-based hotkey matchers (e.g. tiling window managers) check
    // these flags to recognize navigation keys; without them synthesized
    // arrow chords fall through to the focused app.
    if is_arrow_key(key) {
        flags |= CGEventFlags::CGEventFlagNumericPad | CGEventFlags::CGEventFlagSecondaryFn;
    }
    event.set_flags(flags);
    event.post(CGEventTapLocation::HID);
    log::trace!("key event: {key} {state}");
}

fn modifier_event(event_source: CGEventSource, depressed: XMods) {
    let Ok(event) = CGEvent::new(event_source) else {
        log::warn!("could not create CGEvent");
        return;
    };
    let flags = to_cgevent_flags(depressed);
    event.set_type(CGEventType::FlagsChanged);
    event.set_flags(flags);
    event.post(CGEventTapLocation::HID);
    log::trace!("modifiers updated: {depressed:?}");
}

fn get_display_at_point(x: CGFloat, y: CGFloat) -> Option<CGDirectDisplayID> {
    let mut displays: [CGDirectDisplayID; 16] = [0; 16];
    let mut display_count: u32 = 0;
    let rect = CGRect::new(&CGPoint::new(x, y), &CGSize::new(0.0, 0.0));

    let error = unsafe {
        CGGetDisplaysWithRect(
            rect,
            1,
            displays.as_mut_ptr(),
            &mut display_count as *mut u32,
        )
    };

    if error != 0 {
        log::warn!("error getting displays at point ({x}, {y}): {error}");
        return Option::None;
    }

    if display_count == 0 {
        log::debug!("no displays found at point ({x}, {y})");
        return Option::None;
    }

    displays.first().copied()
}

fn get_display_bounds(display: CGDirectDisplayID) -> (CGFloat, CGFloat, CGFloat, CGFloat) {
    unsafe {
        let bounds = CGDisplayBounds(display);
        let min_x = bounds.origin.x;
        let max_x = bounds.origin.x + bounds.size.width;
        let min_y = bounds.origin.y;
        let max_y = bounds.origin.y + bounds.size.height;
        (min_x as f64, min_y as f64, max_x as f64, max_y as f64)
    }
}

fn clamp_to_screen_space(
    current_x: CGFloat,
    current_y: CGFloat,
    dx: CGFloat,
    dy: CGFloat,
) -> (CGFloat, CGFloat) {
    // Check which display the mouse is currently on
    // Determine what the location of the mouse would be after applying the move
    // Get the display at the new location
    // If the point is not on a display
    //   Clamp the mouse to the current display
    // Else If the point is on a display
    //   Clamp the mouse to the new display
    let current_display = match get_display_at_point(current_x, current_y) {
        Some(display) => display,
        None => {
            log::warn!("could not get current display!");
            return (current_x, current_y);
        }
    };

    let new_x = current_x + dx;
    let new_y = current_y + dy;

    let final_display = get_display_at_point(new_x, new_y).unwrap_or(current_display);
    let (min_x, min_y, max_x, max_y) = get_display_bounds(final_display);

    (
        new_x.clamp(min_x, max_x - 1.),
        new_y.clamp(min_y, max_y - 1.),
    )
}

#[async_trait]
impl Emulation for MacOSEmulation {
    async fn consume(
        &mut self,
        event: Event,
        _handle: EmulationHandle,
    ) -> Result<(), EmulationError> {
        log::trace!("{event:?}");
        match event {
            Event::Pointer(pointer_event) => {
                match pointer_event {
                    PointerEvent::Motion { time: _, dx, dy } => {
                        let mut mouse_location = match self.get_mouse_location() {
                            Some(l) => l,
                            None => {
                                log::warn!("could not get mouse location!");
                                return Ok(());
                            }
                        };

                        let (new_mouse_x, new_mouse_y) =
                            clamp_to_screen_space(mouse_location.x, mouse_location.y, dx, dy);

                        mouse_location.x = new_mouse_x;
                        mouse_location.y = new_mouse_y;

                        // If any button is held, emit a drag event for it;
                        // otherwise emit a normal mouse-moved event.
                        let event_type = self
                            .pressed_buttons
                            .iter()
                            .min()
                            .map(|&btn| drag_event_type(btn))
                            .unwrap_or(CGEventType::MouseMoved);
                        self.clicks.moved(mouse_location);
                        let mouse_button = match event_type {
                            CGEventType::RightMouseDragged => CGMouseButton::Right,
                            CGEventType::OtherMouseDragged => CGMouseButton::Center,
                            _ => CGMouseButton::Left,
                        };
                        let event = match CGEvent::new_mouse_event(
                            self.event_source.clone(),
                            event_type,
                            mouse_location,
                            mouse_button,
                        ) {
                            Ok(e) => e,
                            Err(_) => {
                                log::warn!("mouse event creation failed!");
                                return Ok(());
                            }
                        };
                        event.set_integer_value_field(EventField::MOUSE_EVENT_DELTA_X, dx as i64);
                        event.set_integer_value_field(EventField::MOUSE_EVENT_DELTA_Y, dy as i64);
                        event.post(CGEventTapLocation::HID);
                    }
                    PointerEvent::Button {
                        time: _,
                        button,
                        state,
                    } => {
                        // button number for OtherMouse events (3 = back, 4 = forward, etc.)
                        let cg_button_number: Option<i64> = match button {
                            BTN_BACK => Some(3),
                            BTN_FORWARD => Some(4),
                            _ => None,
                        };
                        let (event_type, mouse_button) = match (button, state) {
                            (BTN_LEFT, 1) => (CGEventType::LeftMouseDown, CGMouseButton::Left),
                            (BTN_LEFT, 0) => (CGEventType::LeftMouseUp, CGMouseButton::Left),
                            (BTN_RIGHT, 1) => (CGEventType::RightMouseDown, CGMouseButton::Right),
                            (BTN_RIGHT, 0) => (CGEventType::RightMouseUp, CGMouseButton::Right),
                            (BTN_MIDDLE, 1) => (CGEventType::OtherMouseDown, CGMouseButton::Center),
                            (BTN_MIDDLE, 0) => (CGEventType::OtherMouseUp, CGMouseButton::Center),
                            (BTN_BACK, 1) | (BTN_FORWARD, 1) => {
                                (CGEventType::OtherMouseDown, CGMouseButton::Center)
                            }
                            (BTN_BACK, 0) | (BTN_FORWARD, 0) => {
                                (CGEventType::OtherMouseUp, CGMouseButton::Center)
                            }
                            _ => {
                                log::warn!("invalid button event: {button},{state}");
                                return Ok(());
                            }
                        };
                        // store button state using the evdev button code so
                        // back, forward, and middle are tracked independently
                        if state == 1 {
                            self.pressed_buttons.insert(button);
                        } else {
                            self.pressed_buttons.remove(&button);
                        }

                        let Some(location) = self.get_mouse_location() else {
                            return Ok(());
                        };
                        let click_state =
                            self.clicks
                                .transition(button, state, location, Instant::now());
                        let event = match CGEvent::new_mouse_event(
                            self.event_source.clone(),
                            event_type,
                            location,
                            mouse_button,
                        ) {
                            Ok(e) => e,
                            Err(()) => {
                                log::warn!("mouse event creation failed!");
                                return Ok(());
                            }
                        };
                        event.set_integer_value_field(
                            EventField::MOUSE_EVENT_CLICK_STATE,
                            click_state,
                        );
                        // Set the button number for extra buttons (back=3, forward=4)
                        if let Some(btn_num) = cg_button_number {
                            event.set_integer_value_field(
                                EventField::MOUSE_EVENT_BUTTON_NUMBER,
                                btn_num,
                            );
                        }
                        event.post(CGEventTapLocation::HID);
                    }
                    PointerEvent::Axis {
                        time: _,
                        axis,
                        value,
                    } => {
                        let value = value as i32;
                        let Some((count, wheel1, wheel2, wheel3)) =
                            quartz_scroll_wheels(axis, value)
                        else {
                            log::warn!("invalid scroll event: {axis}, {value}");
                            return Ok(());
                        };
                        let event = match CGEvent::new_scroll_event(
                            self.event_source.clone(),
                            ScrollEventUnit::PIXEL,
                            count,
                            wheel1,
                            wheel2,
                            wheel3,
                        ) {
                            Ok(e) => e,
                            Err(()) => {
                                log::warn!("scroll event creation failed!");
                                return Ok(());
                            }
                        };
                        event.post(CGEventTapLocation::HID);
                    }
                    PointerEvent::AxisDiscrete120 { axis, value } => {
                        const LINES_PER_STEP: i32 = 3;
                        let Some((count, wheel1, wheel2, wheel3)) =
                            quartz_scroll_wheels(axis, value / (120 / LINES_PER_STEP))
                        else {
                            log::warn!("invalid scroll event: {axis}, {value}");
                            return Ok(());
                        };
                        let event = match CGEvent::new_scroll_event(
                            self.event_source.clone(),
                            ScrollEventUnit::LINE,
                            count,
                            wheel1,
                            wheel2,
                            wheel3,
                        ) {
                            Ok(e) => e,
                            Err(()) => {
                                log::warn!("scroll event creation failed!");
                                return Ok(());
                            }
                        };
                        event.post(CGEventTapLocation::HID);
                    }
                }
            }
            Event::Keyboard(keyboard_event) => match keyboard_event {
                KeyboardEvent::Key {
                    time: _,
                    key,
                    state,
                } => {
                    // Fallback table for Linux evdev keys that have no macOS CGKeyCode in
                    // the keycode crate's Chromium-derived table. F13/F14/F15 are the
                    // conventional macOS stand-ins for Print Screen / Scroll Lock / Pause,
                    // which do not exist on Apple keyboards.
                    let code: CGKeyCode = match scancode::Linux::try_from(key) {
                        Ok(scancode::Linux::KeySysrq) => 0x69,      // F13
                        Ok(scancode::Linux::KeyScrollLock) => 0x6B, // F14
                        Ok(scancode::Linux::KeyPause) => 0x71,      // F15
                        _ => match KeyMap::from_key_mapping(KeyMapping::Evdev(key as u16)) {
                            Ok(k) => k.mac as CGKeyCode,
                            Err(_) => {
                                log::warn!("unable to map key event");
                                return Ok(());
                            }
                        },
                    };
                    let is_modifier = update_modifiers(&self.modifier_state, key, state);
                    if is_modifier {
                        modifier_event(self.event_source.clone(), self.modifier_state.get());
                    }
                    match state {
                        // pressed
                        1 => self.spawn_repeat_task(code).await,
                        _ => self.cancel_repeat_task().await,
                    }
                }
                KeyboardEvent::Modifiers {
                    depressed,
                    latched,
                    locked,
                    group,
                } => {
                    set_modifiers(&self.modifier_state, depressed, latched, locked, group);
                    modifier_event(self.event_source.clone(), self.modifier_state.get());
                }
            },
        }
        // FIXME
        Ok(())
    }

    async fn create(&mut self, _handle: EmulationHandle) {}

    async fn destroy(&mut self, _handle: EmulationHandle) {}

    async fn terminate(&mut self) {
        self.cancel_repeat_task().await;
    }
}

fn update_modifiers(modifiers: &Cell<XMods>, key: u32, state: u8) -> bool {
    if let Ok(key) = scancode::Linux::try_from(key) {
        let mask = match key {
            scancode::Linux::KeyLeftShift | scancode::Linux::KeyRightShift => XMods::ShiftMask,
            scancode::Linux::KeyCapsLock => XMods::LockMask,
            scancode::Linux::KeyLeftCtrl | scancode::Linux::KeyRightCtrl => XMods::ControlMask,
            scancode::Linux::KeyLeftAlt | scancode::Linux::KeyRightalt => XMods::Mod1Mask,
            scancode::Linux::KeyLeftMeta | scancode::Linux::KeyRightmeta => XMods::Mod4Mask,
            _ => XMods::empty(),
        };
        // unchanged
        if mask.is_empty() {
            return false;
        }
        let mut mods = modifiers.get();
        match state {
            1 => mods.insert(mask),
            _ => mods.remove(mask),
        }
        modifiers.set(mods);
        true
    } else {
        false
    }
}

fn set_modifiers(
    active_modifiers: &Cell<XMods>,
    depressed: u32,
    latched: u32,
    locked: u32,
    group: u32,
) {
    let depressed = XMods::from_bits(depressed).unwrap_or_default();
    let _latched = XMods::from_bits(latched).unwrap_or_default();
    let _locked = XMods::from_bits(locked).unwrap_or_default();
    let _group = XMods::from_bits(group).unwrap_or_default();

    // we only care about the depressed modifiers for now
    active_modifiers.replace(depressed);
}

fn to_cgevent_flags(depressed: XMods) -> CGEventFlags {
    let mut flags = CGEventFlags::empty();
    if depressed.contains(XMods::ShiftMask) {
        flags |= CGEventFlags::CGEventFlagShift;
    }
    if depressed.contains(XMods::LockMask) {
        flags |= CGEventFlags::CGEventFlagAlphaShift;
    }
    if depressed.contains(XMods::ControlMask) {
        flags |= CGEventFlags::CGEventFlagControl;
    }
    if depressed.contains(XMods::Mod1Mask) {
        flags |= CGEventFlags::CGEventFlagAlternate;
    }
    if depressed.contains(XMods::Mod4Mask) {
        flags |= CGEventFlags::CGEventFlagCommand;
    }
    flags
}

// From X11/X.h
bitflags! {
    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
    struct XMods: u32 {
        const ShiftMask = (1<<0);
        const LockMask = (1<<1);
        const ControlMask = (1<<2);
        const Mod1Mask = (1<<3);
        const Mod2Mask = (1<<4);
        const Mod3Mask = (1<<5);
        const Mod4Mask = (1<<6);
        const Mod5Mask = (1<<7);
    }
}

#[cfg(test)]
mod scroll_direction_tests {
    use super::*;

    // A Windows wheel-up delta (+120) is sent as -120. Verify the actual
    // Quartz event encodes up (+3 lines), and down follows the reverse path.
    // Events are inspected in memory and never posted to the user's desktop.
    #[test]
    fn windows_vertical_wheel_preserves_direction_in_quartz_event() {
        for windows_delta in [-120, 120] {
            let protocol_delta = -windows_delta;
            let (count, y, x, z) = quartz_scroll_wheels(0, protocol_delta / 40).unwrap();
            let event = CGEvent::new_scroll_event(
                CGEventSource::new(CGEventSourceStateID::CombinedSessionState).unwrap(),
                ScrollEventUnit::LINE,
                count,
                y,
                x,
                z,
            )
            .unwrap();
            assert_eq!(
                event.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_DELTA_AXIS_1),
                i64::from(windows_delta / 40)
            );
            assert_eq!(
                event.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_DELTA_AXIS_2),
                0
            );
        }
    }

    #[test]
    fn horizontal_and_pixel_scroll_use_the_same_protocol_direction() {
        for axis in [0, 1] {
            for delta in [-24, 0, 24] {
                let (count, y, x, z) = quartz_scroll_wheels(axis, delta).unwrap();
                let event = CGEvent::new_scroll_event(
                    CGEventSource::new(CGEventSourceStateID::CombinedSessionState).unwrap(),
                    ScrollEventUnit::PIXEL,
                    count,
                    y,
                    x,
                    z,
                )
                .unwrap();
                let field = if axis == 0 {
                    EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_1
                } else {
                    EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_2
                };
                assert_eq!(event.get_integer_value_field(field), -i64::from(delta));
            }
        }
    }

    #[test]
    fn invalid_axis_is_rejected_and_extreme_delta_does_not_overflow() {
        assert_eq!(quartz_scroll_wheels(2, 120), None);
        assert_eq!(quartz_scroll_wheels(0, i32::MIN), Some((1, i32::MAX, 0, 0)));
    }
}

#[cfg(test)]
mod repeat_cancellation_tests {
    use super::*;
    use std::cell::RefCell;

    #[tokio::test]
    async fn repeat_cancellation_before_first_poll_never_hangs() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (cancel, cancelled) = oneshot::channel();
                let states = Rc::new(RefCell::new(Vec::new()));
                let recorded = states.clone();
                let task = tokio::task::spawn_local(repeat_key_events(
                    cancelled,
                    DEFAULT_REPEAT_DELAY,
                    DEFAULT_REPEAT_INTERVAL,
                    move |state| {
                        recorded.borrow_mut().push(state);
                    },
                ));
                // Reproduces the fast down/up race before the repeat task runs.
                cancel.send(()).unwrap();
                let completed = tokio::time::timeout(Duration::from_millis(100), task).await;
                assert!(
                    completed.is_ok(),
                    "key release lost its cancellation signal"
                );
                assert_eq!(&*states.borrow(), &[0]);
            })
            .await;
    }

    #[tokio::test]
    async fn active_repeat_cancels_and_releases_exactly_once() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (cancel, cancelled) = oneshot::channel();
                let (began, started) = oneshot::channel();
                let mut began = Some(began);
                let states = Rc::new(RefCell::new(Vec::new()));
                let recorded = states.clone();
                let task = tokio::task::spawn_local(repeat_key_events(
                    cancelled,
                    Duration::ZERO,
                    Duration::from_secs(60),
                    move |state| {
                        recorded.borrow_mut().push(state);
                        if state == 1 {
                            if let Some(began) = began.take() {
                                let _ = began.send(());
                            }
                        }
                    },
                ));
                tokio::time::timeout(Duration::from_secs(2), started)
                    .await
                    .unwrap()
                    .unwrap();
                cancel.send(()).unwrap();
                tokio::time::timeout(Duration::from_millis(100), task)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(&*states.borrow(), &[1, 0]);
            })
            .await;
    }

    #[tokio::test]
    async fn fast_key_pairs_do_not_share_or_lose_cancellation() {
        for _ in 0..1000 {
            let (cancel, cancelled) = oneshot::channel();
            let mut states = Vec::new();
            cancel.send(()).unwrap();
            repeat_key_events(
                cancelled,
                DEFAULT_REPEAT_DELAY,
                DEFAULT_REPEAT_INTERVAL,
                |state| states.push(state),
            )
            .await;
            assert_eq!(states, vec![0]);
        }
    }

    #[tokio::test]
    async fn dropping_cancellation_owner_still_releases_key() {
        let (cancel, cancelled) = oneshot::channel();
        drop(cancel);
        let mut states = Vec::new();
        repeat_key_events(
            cancelled,
            DEFAULT_REPEAT_DELAY,
            DEFAULT_REPEAT_INTERVAL,
            |state| states.push(state),
        )
        .await;
        assert_eq!(states, vec![0]);
    }
}

#[cfg(test)]
mod click_tracking_tests {
    use super::*;
    #[test]
    fn click_release_keeps_count_during_motion_and_double_click_requires_proximity() {
        let mut clicks = ClickTracker::default();
        let now = Instant::now();
        let p = CGPoint::new(10., 10.);
        assert_eq!(clicks.transition(BTN_LEFT, 1, p, now), 1);
        clicks.moved(CGPoint::new(11., 10.));
        assert_eq!(clicks.transition(BTN_LEFT, 0, p, now), 1);
        assert_eq!(
            clicks.transition(BTN_LEFT, 1, p, now + Duration::from_millis(100)),
            2
        );
        clicks.moved(CGPoint::new(300., 200.));
        assert_eq!(
            clicks.transition(BTN_LEFT, 0, CGPoint::new(300., 200.), now),
            2
        );
        assert_eq!(
            clicks.transition(
                BTN_LEFT,
                1,
                CGPoint::new(300., 200.),
                now + Duration::from_millis(200)
            ),
            1
        );
    }
    #[test]
    fn rapid_clicks_at_different_locations_and_duplicate_down_are_single_clicks() {
        let mut c = ClickTracker::default();
        let now = Instant::now();
        let p = CGPoint::new(1., 1.);
        assert_eq!(c.transition(BTN_LEFT, 1, p, now), 1);
        assert_eq!(c.transition(BTN_LEFT, 1, p, now), 1);
        assert_eq!(c.transition(BTN_LEFT, 0, p, now), 1);
        assert_eq!(
            c.transition(
                BTN_LEFT,
                1,
                CGPoint::new(200., 200.),
                now + Duration::from_millis(10)
            ),
            1
        );
        assert_eq!(
            c.transition(BTN_RIGHT, 1, p, now + Duration::from_millis(20)),
            1
        );
        assert_eq!(c.transition(BTN_LEFT, 0, p, now), 1);
    }
}
