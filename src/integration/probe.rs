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
    pub fn new(mut probe: impl FnMut() -> T + Send + 'static) -> std::io::Result<Self> {
        let (request, requests) = sync_channel(1);
        let (results, result) = sync_channel(1);
        thread::Builder::new()
            .name("desktop-metadata".into())
            .spawn(move || {
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
