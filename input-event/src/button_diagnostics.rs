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
}
const EMPTY: Entry = Entry {
    at_ms: 0,
    stage: Stage::Physical,
    detail: Detail::Snapshot { serial: 0, mask: 0 },
};

struct History {
    entries: [Entry; CAPACITY],
    next: usize,
    len: usize,
}
impl History {
    const fn new() -> Self {
        Self {
            entries: [EMPTY; CAPACITY],
            next: 0,
            len: 0,
        }
    }
    fn push(&mut self, entry: Entry) {
        self.entries[self.next] = entry;
        self.next = (self.next + 1) % CAPACITY;
        self.len = (self.len + 1).min(CAPACITY);
    }
    fn ordered(&self) -> Vec<Entry> {
        let first = (self.next + CAPACITY - self.len) % CAPACITY;
        (0..self.len)
            .map(|i| self.entries[(first + i) % CAPACITY])
            .collect()
    }
}
static HISTORY: Mutex<History> = Mutex::new(History::new());
static DROPPED: AtomicU64 = AtomicU64::new(0);

#[derive(Serialize)]
pub struct Snapshot {
    pub capacity: usize,
    pub dropped_records: u64,
    pub entries: Vec<Entry>,
}

fn record_to(history: &Mutex<History>, dropped: &AtomicU64, entry: Entry) {
    if let Ok(mut history) = history.try_lock() {
        history.push(entry);
    } else {
        dropped.fetch_add(1, Ordering::Relaxed);
    }
}
fn record(stage: Stage, detail: Detail) {
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
