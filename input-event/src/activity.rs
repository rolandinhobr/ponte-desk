//! In-process physical-activity metadata. No text, coordinates or key history.
use std::sync::{
    OnceLock,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Instant;
static ENABLED: AtomicBool = AtomicBool::new(false);
static CLAIM: AtomicBool = AtomicBool::new(false);
static TAKEOVER: AtomicBool = AtomicBool::new(false);
static READY: AtomicBool = AtomicBool::new(false);
static SERIAL: AtomicU64 = AtomicU64::new(0);
static CLAIM_SERIAL: AtomicU64 = AtomicU64::new(0);
static LAST: AtomicU64 = AtomicU64::new(0);
static LAST_CLAIM: AtomicU64 = AtomicU64::new(0);
static HELD: [AtomicU64; 16] = [const { AtomicU64::new(0) }; 16];
fn now() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}
pub fn configure(enabled: bool, claim: bool, takeover: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
    CLAIM.store(enabled && claim, Ordering::Relaxed);
    TAKEOVER.store(enabled && takeover, Ordering::Relaxed);
    if !enabled {
        clear_held();
    }
}
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}
pub fn mac_takeover() -> bool {
    TAKEOVER.load(Ordering::Relaxed)
}
pub fn ready(value: bool) {
    READY.store(value, Ordering::Relaxed);
    if !value {
        clear_held();
    }
}
pub fn clear_held() {
    for held in &HELD {
        held.store(0, Ordering::Relaxed);
    }
}
pub fn physical_motion() {
    record(true);
}
pub fn physical_key(code: u16, down: bool) {
    key(code, down, 0);
}
pub fn captured_key(code: u16, down: bool) {
    key(code, down, 8);
}
pub fn capture_stopped() {
    for held in &HELD[8..] {
        held.store(0, Ordering::Relaxed);
    }
}
fn key(code: u16, down: bool, bank: usize) {
    if !enabled() || code >= 512 {
        return;
    }
    if let Some(word) = HELD.get(bank + usize::from(code) / 64) {
        let bit = 1u64 << (code % 64);
        if down {
            word.fetch_or(bit, Ordering::Relaxed);
        } else {
            word.fetch_and(!bit, Ordering::Relaxed);
        }
    }
    record(down);
}
fn record(intent: bool) {
    if !enabled() {
        return;
    }
    let at = now();
    LAST.store(at, Ordering::Relaxed);
    SERIAL.fetch_add(1, Ordering::Relaxed);
    if intent && CLAIM.load(Ordering::Relaxed) {
        LAST_CLAIM.store(at, Ordering::Relaxed);
        CLAIM_SERIAL.fetch_add(1, Ordering::Relaxed);
    }
}
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Sample {
    pub ready: bool,
    pub serial: u64,
    pub claim_serial: u64,
    pub age_ms: Option<u64>,
    pub claim_age_ms: Option<u64>,
    pub held: bool,
}
pub fn sample() -> Sample {
    let at = now();
    let age = |v: u64| (v != 0).then(|| at.saturating_sub(v));
    Sample {
        ready: READY.load(Ordering::Relaxed),
        serial: SERIAL.load(Ordering::Relaxed),
        claim_serial: CLAIM_SERIAL.load(Ordering::Relaxed),
        age_ms: age(LAST.load(Ordering::Relaxed)),
        claim_age_ms: age(LAST_CLAIM.load(Ordering::Relaxed)),
        held: HELD.iter().any(|h| h.load(Ordering::Relaxed) != 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn physical_metadata_preserves_held_input_and_donor_never_claims() {
        configure(true, false, false);
        ready(true);
        let before = sample();
        physical_key(42, true);
        physical_key(42, true);
        physical_key(256, true);
        let down = sample();
        assert!(down.held);
        assert_eq!(down.claim_serial, before.claim_serial);
        physical_key(42, false);
        assert!(sample().held);
        physical_key(256, false);
        assert!(!sample().held);
        physical_key(55, true);
        captured_key(55, true);
        physical_key(55, false);
        assert!(
            sample().held,
            "a delayed raw release cannot cancel capture's newer press"
        );
        capture_stopped();
        assert!(!sample().held);
        configure(true, true, true);
        physical_motion();
        assert!(sample().claim_serial > before.claim_serial);
        assert!(mac_takeover());
        configure(false, false, false);
        physical_key(55, true);
        assert!(!sample().held);
        assert!(!mac_takeover());
    }
}
