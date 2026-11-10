use core_foundation::{
    base::TCFType,
    mach_port::CFMachPortRef,
    runloop::{CFRunLoop, kCFRunLoopCommonModes, kCFRunLoopDefaultMode},
};
use core_graphics::event::{
    CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    CallbackResult, EventField,
};
use input_event::activity;
use std::sync::{Arc, OnceLock};
pub(super) fn start() {
    static START: OnceLock<()> = OnceLock::new();
    START.get_or_init(|| {
        let _ = std::thread::Builder::new()
            .name("human-activity".into())
            .spawn(|| {
                let port: Arc<OnceLock<usize>> = Arc::new(OnceLock::new());
                let cb_port = port.clone();
                let events = vec![
                    CGEventType::KeyDown,
                    CGEventType::KeyUp,
                    CGEventType::FlagsChanged,
                    CGEventType::LeftMouseDown,
                    CGEventType::LeftMouseUp,
                    CGEventType::RightMouseDown,
                    CGEventType::RightMouseUp,
                    CGEventType::OtherMouseDown,
                    CGEventType::OtherMouseUp,
                    CGEventType::MouseMoved,
                    CGEventType::LeftMouseDragged,
                    CGEventType::RightMouseDragged,
                    CGEventType::OtherMouseDragged,
                    CGEventType::ScrollWheel,
                ];
                let Ok(tap) = CGEventTap::new(
                    CGEventTapLocation::Session,
                    CGEventTapPlacement::HeadInsertEventTap,
                    CGEventTapOptions::ListenOnly,
                    events,
                    move |_, kind, event| {
                        if matches!(
                            kind,
                            CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput
                        ) {
                            activity::ready(false);
                            if matches!(kind, CGEventType::TapDisabledByTimeout) {
                                if let Some(p) = cb_port.get() {
                                    unsafe { CGEventTapEnable(*p as CFMachPortRef, true) };
                                    activity::ready(true);
                                }
                            }
                            return CallbackResult::Keep;
                        }
                        if !activity::enabled()
                            || event
                                .get_integer_value_field(EventField::EVENT_SOURCE_UNIX_PROCESS_ID)
                                != 0
                        {
                            return CallbackResult::Keep;
                        }
                        match kind {
                            CGEventType::KeyDown | CGEventType::KeyUp => activity::physical_key(
                                event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE)
                                    as u16,
                                matches!(kind, CGEventType::KeyDown),
                            ),
                            CGEventType::FlagsChanged => {
                                let code = event
                                    .get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE)
                                    as u16;
                                let bit = match code {
                                    55 => 8,
                                    54 => 16,
                                    56 => 2,
                                    60 => 4,
                                    59 => 1,
                                    62 => 0x2000,
                                    58 => 0x20,
                                    61 => 0x40,
                                    63 => 1 << 23,
                                    _ => 0,
                                };
                                if code == 57 {
                                    activity::physical_motion();
                                }
                                if bit != 0 {
                                    activity::physical_key(
                                        code,
                                        event.get_flags().bits() & bit != 0,
                                    );
                                }
                            }
                            CGEventType::LeftMouseDown
                            | CGEventType::RightMouseDown
                            | CGEventType::OtherMouseDown
                            | CGEventType::LeftMouseUp
                            | CGEventType::RightMouseUp
                            | CGEventType::OtherMouseUp => {
                                let n = event
                                    .get_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER)
                                    .clamp(0, 31) as u16;
                                activity::physical_key(
                                    256 + n,
                                    matches!(
                                        kind,
                                        CGEventType::LeftMouseDown
                                            | CGEventType::RightMouseDown
                                            | CGEventType::OtherMouseDown
                                    ),
                                );
                            }
                            _ => activity::physical_motion(),
                        }
                        CallbackResult::Keep
                    },
                ) else {
                    log::warn!("automatic activity observer requires Input Monitoring");
                    return;
                };
                let _ = port.set(tap.mach_port().as_concrete_TypeRef() as usize);
                let Ok(source) = tap.mach_port().create_runloop_source(0) else {
                    return;
                };
                unsafe {
                    CFRunLoop::get_current().add_source(&source, kCFRunLoopCommonModes);
                }
                tap.enable();
                activity::ready(true);
                log::info!("automatic physical activity observer active");
                loop {
                    unsafe {
                        CFRunLoop::run_in_mode(
                            kCFRunLoopDefaultMode,
                            std::time::Duration::from_secs(1),
                            false,
                        );
                    }
                    if activity::enabled() && unsafe { CGPreflightListenEventAccess() } {
                        if !unsafe { CGEventTapIsEnabled(tap.mach_port().as_concrete_TypeRef()) } {
                            tap.enable();
                        }
                        activity::ready(unsafe {
                            CGEventTapIsEnabled(tap.mach_port().as_concrete_TypeRef())
                        });
                    }
                }
            });
    });
}
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventTapIsEnabled(tap: CFMachPortRef) -> bool;
    fn CGPreflightListenEventAccess() -> bool;
}
