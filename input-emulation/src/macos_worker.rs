//! Keep synchronous Quartz/WindowServer work off the network executor.
//! The original backend, real cursor reads and event order remain unchanged.
use super::error::MacOSEmulationCreationError;
use super::{Emulation, EmulationError, EmulationHandle, NativePerformance};
use async_trait::async_trait;
use input_event::Event;
use std::{
    io,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};
use tokio::sync::{mpsc, oneshot};

static REQUESTS: AtomicU64 = AtomicU64::new(0);
static MAX_NATIVE_US: AtomicU64 = AtomicU64::new(0);
static MAX_ROUND_TRIP_US: AtomicU64 = AtomicU64::new(0);

pub(super) fn performance() -> NativePerformance {
    NativePerformance {
        requests: REQUESTS.load(Ordering::Relaxed),
        max_native_us: MAX_NATIVE_US.load(Ordering::Relaxed),
        max_round_trip_us: MAX_ROUND_TRIP_US.load(Ordering::Relaxed),
    }
}

enum Operation {
    Input(Event, EmulationHandle),
    Create(EmulationHandle),
    Destroy(EmulationHandle),
    Terminate,
}
struct Request {
    operation: Operation,
    reply: oneshot::Sender<Result<(), EmulationError>>,
}

pub(super) struct MacOSInputWorker {
    tx: mpsc::Sender<Request>,
}

impl MacOSInputWorker {
    pub async fn new() -> Result<Self, MacOSEmulationCreationError> {
        Self::start(|| Ok(Box::new(super::macos::MacOSEmulation::new()?))).await
    }

    async fn start(
        factory: impl FnOnce() -> Result<Box<dyn Emulation>, MacOSEmulationCreationError>
        + Send
        + 'static,
    ) -> Result<Self, MacOSEmulationCreationError> {
        let (tx, mut rx) = mpsc::channel::<Request>(1);
        let (ready, startup) = oneshot::channel();
        std::thread::Builder::new()
            .name("macos-input".into())
            .spawn(move || {
                // This small, normally sleeping thread handles interactive input,
                // not rendering, file transfer or metadata polling.
                unsafe extern "C" {
                    fn pthread_set_qos_class_self_np(class: u32, priority: i32) -> i32;
                }
                let qos = unsafe { pthread_set_qos_class_self_np(0x21, 0) };
                if qos != 0 {
                    log::debug!("input worker QoS unavailable: {qos}");
                }
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = ready.send(Err(error.into()));
                        return;
                    }
                };
                tokio::task::LocalSet::new().block_on(&runtime, async move {
                    let mut backend = match factory() {
                        Ok(backend) => backend,
                        Err(error) => {
                            let _ = ready.send(Err(error));
                            return;
                        }
                    };
                    if ready.send(Ok(())).is_err() {
                        backend.terminate().await;
                        return;
                    }
                    while let Some(request) = rx.recv().await {
                        let started = Instant::now();
                        let result = match request.operation {
                            Operation::Input(event, handle) => {
                                let result = backend.consume(event, handle).await;
                                REQUESTS.fetch_add(1, Ordering::Relaxed);
                                MAX_NATIVE_US.fetch_max(
                                    started.elapsed().as_micros() as u64,
                                    Ordering::Relaxed,
                                );
                                result
                            }
                            Operation::Create(handle) => {
                                backend.create(handle).await;
                                Ok(())
                            }
                            Operation::Destroy(handle) => {
                                backend.destroy(handle).await;
                                Ok(())
                            }
                            Operation::Terminate => {
                                backend.terminate().await;
                                let _ = request.reply.send(Ok(()));
                                return;
                            }
                        };
                        let _ = request.reply.send(result);
                    }
                    backend.terminate().await;
                });
            })?;
        startup
            .await
            .map_err(|_| io::Error::other("input worker exited during startup"))??;
        Ok(Self { tx })
    }

    async fn request(&self, operation: Operation) -> Result<(), EmulationError> {
        let measure = matches!(operation, Operation::Input(..));
        let started = Instant::now();
        let (reply, completed) = oneshot::channel();
        self.tx
            .send(Request { operation, reply })
            .await
            .map_err(|_| EmulationError::EndOfStream)?;
        let result = completed.await.map_err(|_| EmulationError::EndOfStream)?;
        if measure {
            MAX_ROUND_TRIP_US.fetch_max(started.elapsed().as_micros() as u64, Ordering::Relaxed);
        }
        result
    }
}

#[async_trait]
impl Emulation for MacOSInputWorker {
    async fn consume(
        &mut self,
        event: Event,
        handle: EmulationHandle,
    ) -> Result<(), EmulationError> {
        self.request(Operation::Input(event, handle)).await
    }
    async fn create(&mut self, handle: EmulationHandle) {
        if let Err(error) = self.request(Operation::Create(handle)).await {
            log::warn!("input worker create: {error}");
        }
    }
    async fn destroy(&mut self, handle: EmulationHandle) {
        if let Err(error) = self.request(Operation::Destroy(handle)).await {
            log::warn!("input worker destroy: {error}");
        }
    }
    async fn terminate(&mut self) {
        if let Err(error) = self.request(Operation::Terminate).await {
            log::debug!("input worker terminate: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use input_event::KeyboardEvent;
    use std::sync::{Arc, Mutex, mpsc as sync};
    use std::time::Duration;

    struct BlockedBackend {
        entered: sync::Sender<()>,
        release: sync::Receiver<()>,
        events: Arc<Mutex<Vec<&'static str>>>,
    }
    #[async_trait]
    impl Emulation for BlockedBackend {
        async fn consume(
            &mut self,
            event: Event,
            _: EmulationHandle,
        ) -> Result<(), EmulationError> {
            if matches!(event, Event::Keyboard(KeyboardEvent::Key { state: 1, .. })) {
                self.entered.send(()).unwrap();
                self.release.recv_timeout(Duration::from_secs(3)).unwrap();
                self.events.lock().unwrap().push("down");
            } else {
                self.events.lock().unwrap().push("up");
            }
            Ok(())
        }
        async fn create(&mut self, _: EmulationHandle) {
            self.events.lock().unwrap().push("create");
        }
        async fn destroy(&mut self, _: EmulationHandle) {
            self.events.lock().unwrap().push("destroy");
        }
        async fn terminate(&mut self) {
            self.events.lock().unwrap().push("stop");
        }
    }

    #[tokio::test]
    async fn blocked_native_call_keeps_network_executor_responsive_and_preserves_order() {
        let (entered, waiting) = sync::channel();
        let (release, blocked) = sync::channel();
        let events = Arc::new(Mutex::new(Vec::new()));
        let recorded = events.clone();
        let main_thread = std::thread::current().id();
        let mut worker = MacOSInputWorker::start(move || {
            assert_ne!(std::thread::current().id(), main_thread);
            Ok(Box::new(BlockedBackend {
                entered,
                release: blocked,
                events: recorded,
            }))
        })
        .await
        .unwrap();
        let task = tokio::spawn(async move {
            worker.create(1).await;
            for state in [1, 0] {
                worker
                    .consume(
                        Event::Keyboard(KeyboardEvent::Key {
                            time: 0,
                            key: 30,
                            state,
                        }),
                        1,
                    )
                    .await
                    .unwrap();
            }
            worker.destroy(1).await;
            worker.terminate().await;
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while waiting.try_recv().is_err() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        // The native backend is deliberately blocked, but unrelated timer and
        // network futures on the caller's single-threaded runtime still run.
        tokio::time::timeout(
            Duration::from_millis(100),
            tokio::time::sleep(Duration::from_millis(5)),
        )
        .await
        .unwrap();
        assert_eq!(&*events.lock().unwrap(), &["create"]);
        release.send(()).unwrap();
        task.await.unwrap();
        assert_eq!(
            &*events.lock().unwrap(),
            &["create", "down", "up", "destroy", "stop"]
        );
    }

    #[tokio::test]
    async fn native_startup_failure_is_reported_without_enabling_emulation() {
        let result =
            MacOSInputWorker::start(|| Err(MacOSEmulationCreationError::EventSourceCreation)).await;
        assert!(matches!(
            result,
            Err(MacOSEmulationCreationError::EventSourceCreation)
        ));
    }
}
