//! Bounded button-only diagnostics. Never records keys, positions or app names.
//! Hooks use try_lock and a preallocated ring: diagnostics may drop a record,
//! but must never wait for a reader or change input behavior.
use serde::Serialize;
use std::{
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

pub const CAPACITY: usize = 256;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Physical,
    Captured,
    Repair,
    Cleanup,
    Send,
    Receive,
    Emulated,
    Suppressed,
    Posted,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Detail {
    Transition {
        button: u8,
        down: bool,
        click_count: Option<i64>,
    },
    Snapshot {
        serial: u32,
        mask: u8,
    },
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct Entry {
    pub at_ms: u64,
    pub stage: Stage,
    pub detail: Detail,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_flags: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_modifier_flags: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_modifier_flags: Option<u64>,
}
const EMPTY: Entry = Entry {
    at_ms: 0,
    stage: Stage::Physical,
    detail: Detail::Snapshot { serial: 0, mask: 0 },
    modifier_flags: None,
    remote_modifier_flags: None,
    local_modifier_flags: None,
};

struct History {
    entries: [Entry; CAPACITY],
    next: usize,
    len: usize,
    // Separate button histories survive the periodic snapshot traffic.
    posted: [Entry; CAPACITY],
    posted_next: usize,
    posted_len: usize,
    modified: [Entry; 64],
    modified_next: usize,
    modified_len: usize,
}
impl History {
    const fn new() -> Self {
        Self {
            entries: [EMPTY; CAPACITY],
            next: 0,
            len: 0,
            posted: [EMPTY; CAPACITY],
            posted_next: 0,
            posted_len: 0,
            modified: [EMPTY; 64],
            modified_next: 0,
            modified_len: 0,
        }
    }
    fn push(&mut self, entry: Entry) {
        self.entries[self.next] = entry;
        self.next = (self.next + 1) % CAPACITY;
        self.len = (self.len + 1).min(CAPACITY);
        if entry.stage == Stage::Posted {
            self.posted[self.posted_next] = entry;
            self.posted_next = (self.posted_next + 1) % CAPACITY;
            self.posted_len = (self.posted_len + 1).min(CAPACITY);
            if entry.modifier_flags.is_some_and(|f| f != 0) {
                self.modified[self.modified_next] = entry;
                self.modified_next = (self.modified_next + 1) % 64;
                self.modified_len = (self.modified_len + 1).min(64);
            }
        }
    }
    fn ordered(&self) -> Vec<Entry> {
        ordered_ring(&self.entries, self.next, self.len)
    }
}
fn ordered_ring<const N: usize>(entries: &[Entry; N], next: usize, len: usize) -> Vec<Entry> {
    let first = (next + N - len) % N;
    (0..len).map(|i| entries[(first + i) % N]).collect()
}
static HISTORY: Mutex<History> = Mutex::new(History::new());
static DROPPED: AtomicU64 = AtomicU64::new(0);

#[derive(Serialize)]
pub struct Snapshot {
    pub capacity: usize,
    pub dropped_records: u64,
    pub entries: Vec<Entry>,
    pub posted_entries: Vec<Entry>,
    pub modified_posted_entries: Vec<Entry>,
}

fn record_to(history: &Mutex<History>, dropped: &AtomicU64, entry: Entry) {
    if let Ok(mut history) = history.try_lock() {
        history.push(entry);
    } else {
        dropped.fetch_add(1, Ordering::Relaxed);
    }
}
fn record(stage: Stage, detail: Detail) {
    record_with_flags(stage, detail, None);
}
fn record_with_flags(stage: Stage, detail: Detail, modifier_flags: Option<u64>) {
    record_with_sources(stage, detail, modifier_flags, None);
}
fn record_with_sources(
    stage: Stage,
    detail: Detail,
    modifier_flags: Option<u64>,
    sources: Option<(u64, u64)>,
) {
    let at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    record_to(
        &HISTORY,
        &DROPPED,
        Entry {
            at_ms,
            stage,
            detail,
            modifier_flags,
            remote_modifier_flags: sources.map(|s| s.0 & 0x00ff_20ff),
            local_modifier_flags: sources.map(|s| s.1 & 0x00ff_20ff),
        },
    );
}
fn transition_detail(button: u32, state: u32, click_count: Option<i64>) -> Option<Detail> {
    let index = button.checked_sub(crate::BTN_LEFT).filter(|i| *i < 5)?;
    (state <= 1).then_some(Detail::Transition {
        button: index as u8,
        down: state == 1,
        click_count,
    })
}
pub fn transition(stage: Stage, button: u32, state: u32, click_count: Option<i64>) {
    if let Some(detail) = transition_detail(button, state, click_count) {
        record(stage, detail);
    }
}
/// Pointer modifiers only: no key codes or typed content are recorded.
pub fn posted_transition(button: u32, state: u32, click_count: i64, flags: u64) {
    if let Some(detail) = transition_detail(button, state, Some(click_count)) {
        record_with_flags(Stage::Posted, detail, Some(flags & 0x00ff_20ff));
    }
}
/// Attribute pointer modifiers to remote state and the local key-state sample.
/// No ordinary key events, text, positions or application names are collected.
pub fn posted_transition_with_sources(
    button: u32,
    state: u32,
    click_count: i64,
    flags: u64,
    sources: (u64, u64),
) {
    if let Some(detail) = transition_detail(button, state, Some(click_count)) {
        record_with_sources(
            Stage::Posted,
            detail,
            Some(flags & 0x00ff_20ff),
            Some(sources),
        );
    }
}
pub fn snapshot_event(stage: Stage, serial: u32, mask: u8) {
    if mask & !31 == 0 {
        record(stage, Detail::Snapshot { serial, mask });
    }
}
pub fn snapshot() -> Option<Snapshot> {
    let history = HISTORY.try_lock().ok()?;
    Some(Snapshot {
        capacity: CAPACITY,
        dropped_records: DROPPED.load(Ordering::Relaxed),
        entries: history.ordered(),
        posted_entries: ordered_ring(&history.posted, history.posted_next, history.posted_len),
        modified_posted_entries: ordered_ring(
            &history.modified,
            history.modified_next,
            history.modified_len,
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ring_retains_only_the_latest_records_in_order() {
        let mut history = History::new();
        for n in 0..CAPACITY * 3 {
            history.push(Entry {
                at_ms: n as u64,
                ..EMPTY
            });
        }
        let entries = history.ordered();
        assert_eq!(entries.len(), CAPACITY);
        assert_eq!(entries.first().unwrap().at_ms, (CAPACITY * 2) as u64);
        assert_eq!(entries.last().unwrap().at_ms, (CAPACITY * 3 - 1) as u64);
    }
    #[test]
    fn modified_click_history_survives_snapshot_and_ordinary_click_traffic() {
        let mut history = History::new();
        let modified = Entry {
            at_ms: 42,
            stage: Stage::Posted,
            modifier_flags: Some(0x100000),
            remote_modifier_flags: Some(0),
            local_modifier_flags: Some(0x100000),
            ..EMPTY
        };
        history.push(modified);
        for n in 0..CAPACITY * 3 {
            history.push(Entry {
                at_ms: n as u64 + 100,
                ..EMPTY
            });
        }
        assert_eq!(
            ordered_ring(&history.posted, history.posted_next, history.posted_len),
            vec![modified]
        );
        for _ in 0..CAPACITY * 3 {
            history.push(Entry {
                stage: Stage::Posted,
                modifier_flags: Some(0),
                ..EMPTY
            });
        }
        assert_eq!(history.posted_len, CAPACITY);
        assert_eq!(
            ordered_ring(
                &history.modified,
                history.modified_next,
                history.modified_len
            ),
            vec![modified]
        );
    }

    #[test]
    fn contended_diagnostic_reader_cannot_block_the_hook() {
        let history = Mutex::new(History::new());
        let dropped = AtomicU64::new(0);
        let guard = history.lock().unwrap();
        for _ in 0..10_000 {
            record_to(&history, &dropped, EMPTY);
        }
        assert_eq!(dropped.load(Ordering::Relaxed), 10_000);
        assert_eq!(guard.len, 0);
        drop(guard);
        record_to(&history, &dropped, EMPTY);
        assert_eq!(history.lock().unwrap().len, 1);
    }
    #[test]
    fn diagnostic_schema_only_accepts_mouse_buttons_and_binary_transitions() {
        assert!(transition_detail(crate::BTN_LEFT, 1, Some(2)).is_some());
        assert!(transition_detail(crate::BTN_FORWARD, 0, None).is_some());
        assert!(transition_detail(30, 1, None).is_none());
        assert!(transition_detail(crate::BTN_LEFT, 2, None).is_none());
        assert!(transition_detail(crate::BTN_FORWARD + 1, 0, None).is_none());
    }
}
