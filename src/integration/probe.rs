//! Slow desktop metadata must never run on the single-threaded input executor.
use std::{
    sync::mpsc::{Receiver, SyncSender, sync_channel},
    thread,
    time::{Duration, Instant},
};

pub(super) struct BackgroundProbe<T> {
    request: SyncSender<()>,
    result: Receiver<(Instant, T)>,
    pending: bool,
    latest: Option<(Instant, T)>,
}

impl<T: Clone + Send + 'static> BackgroundProbe<T> {
    pub fn new(probe: impl FnMut() -> T + Send + 'static) -> std::io::Result<Self> {
        Self::named("desktop-metadata", probe)
    }

    pub fn named(
        name: &str,
        mut probe: impl FnMut() -> T + Send + 'static,
    ) -> std::io::Result<Self> {
        let (request, requests) = sync_channel(1);
        let (results, result) = sync_channel(1);
        thread::Builder::new().name(name.into()).spawn(move || {
            while requests.recv().is_ok() {
                // Age includes time spent inside a stalled OS call.
                let started = Instant::now();
                if results.send((started, probe())).is_err() {
                    break;
                }
            }
        })?;
        Ok(Self {
            request,
            result,
            pending: false,
            latest: None,
        })
    }

    pub fn poll(&mut self, max_age: Duration) -> Option<T> {
        if let Ok(sample) = self.result.try_recv() {
            self.latest = Some(sample);
            self.pending = false;
        }
        // At most one outstanding call, even if the OS hangs indefinitely.
        if !self.pending && self.request.try_send(()).is_ok() {
            self.pending = true;
        }
        self.latest
            .as_ref()
            .filter(|(at, _)| at.elapsed() < max_age)
            .map(|(_, value)| value.clone())
    }
}

// Keep status publication off input, with bounded work and no blocking send.
pub(super) struct BackgroundWriter<T> {
    request: SyncSender<T>,
}
impl<T: Send + 'static> BackgroundWriter<T> {
    pub fn new(name: &str, mut write: impl FnMut(T) + Send + 'static) -> std::io::Result<Self> {
        let (request, values) = sync_channel(1);
        thread::Builder::new().name(name.into()).spawn(move || {
            while let Ok(value) = values.recv() {
                write(value);
            }
        })?;
        Ok(Self { request })
    }
    pub fn submit(&self, value: T) -> bool {
        self.request.try_send(value).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocked_probe_never_blocks_input_or_queues_more_probes() {
        let (entered_tx, entered_rx) = sync_channel(1);
        let (release_tx, release_rx) = sync_channel(1);
        let mut probe = BackgroundProbe::new(move || {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            42
        })
        .unwrap();
        assert_eq!(probe.poll(Duration::from_secs(2)), None);
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let before = Instant::now();
        for _ in 0..10_000 {
            assert_eq!(probe.poll(Duration::from_secs(2)), None);
        }
        assert!(before.elapsed() < Duration::from_millis(250));
        assert!(entered_rx.try_recv().is_err());
        release_tx.send(()).unwrap();
        // Drop receivers without joining a worker potentially blocked in the OS.
    }

    #[test]
    fn blocked_status_write_does_not_block_input_or_grow_a_backlog() {
        let (entered, waiting) = sync_channel(1);
        let (release, blocked) = sync_channel::<()>(1);
        let writer = BackgroundWriter::new("test-status", move |_: u8| {
            let _ = entered.send(());
            let _ = blocked.recv();
        })
        .unwrap();
        assert!(writer.submit(1));
        waiting.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(writer.submit(2));
        let start = Instant::now();
        for _ in 0..10_000 {
            assert!(!writer.submit(3));
        }
        assert!(start.elapsed() < Duration::from_millis(250));
        release.send(()).unwrap();
        // Disconnection releases the fake writer's second call as well.
        drop(release);
    }

    #[test]
    fn expired_metadata_is_not_reused_as_current() {
        let (_release, wait) = sync_channel::<()>(1);
        let mut probe = BackgroundProbe::new(move || {
            let _ = wait.recv();
            42
        })
        .unwrap();
        probe.latest = Some((Instant::now() - Duration::from_secs(3), 42));
        assert_eq!(probe.poll(Duration::from_secs(2)), None);
        probe.latest = Some((Instant::now(), 24));
        assert_eq!(probe.poll(Duration::from_secs(2)), Some(24));
    }
}
