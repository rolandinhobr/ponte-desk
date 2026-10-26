//! Only physical input is exported. Software input stays on its source host.
use std::sync::atomic::{AtomicU64, Ordering};

/// Tag used by our Windows SendInput calls, never attributed to an agent.
pub const EMULATION_TAG: usize = 0x4c4d_4252;
static IGNORED_SOFTWARE: AtomicU64 = AtomicU64::new(0);

pub fn ignored_software_events() -> u64 {
    IGNORED_SOFTWARE.load(Ordering::Relaxed)
}

pub(crate) fn record_software_event() {
    IGNORED_SOFTWARE.fetch_add(1, Ordering::Relaxed);
}

pub fn windows_mouse_is_software(flags: u32) -> bool {
    flags & 0x01 != 0
}

pub fn windows_key_is_software(flags: u32) -> bool {
    flags & 0x10 != 0
}

pub fn mac_event_is_software(source_pid: i64) -> bool {
    source_pid != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn physical_input_and_extended_keys_remain_shareable() {
        assert!(!windows_mouse_is_software(0));
        assert!(!windows_key_is_software(1));
        assert!(!mac_event_is_software(0));
    }
    #[test]
    fn injected_input_never_enters_the_network_capture() {
        assert!(windows_mouse_is_software(1));
        assert!(windows_mouse_is_software(3));
        assert!(windows_key_is_software(0x10));
        assert!(windows_key_is_software(0x91));
        assert!(mac_event_is_software(123));
    }
}
