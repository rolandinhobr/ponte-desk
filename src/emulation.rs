use crate::config::local_commit;
use crate::listen::{LanMouseListener, ListenEvent, ListenerCreationError};
use futures::{FutureExt, StreamExt};
use input_emulation::{EmulationHandle, InputEmulation, InputEmulationError};
use input_event::{Event, PointerEvent};
use lan_mouse_proto::{Position, ProtoEvent};
use local_channel::mpsc::{Receiver, Sender, channel};
use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    net::SocketAddr,
    rc::Rc,
    time::{Duration, Instant},
};
use tokio::{
    select,
    task::{JoinHandle, spawn_local},
};

/// emulation handling events received from a listener
pub(crate) struct Emulation {
    task: JoinHandle<()>,
    request_tx: Sender<EmulationRequest>,
    event_rx: Receiver<EmulationEvent>,
}

pub(crate) enum EmulationEvent {
    Connected {
        addr: SocketAddr,
        fingerprint: String,
    },
    ConnectionAttempt {
        fingerprint: String,
    },
    /// new connection
    Entered {
        /// address of the connection
        addr: SocketAddr,
        /// position of the connection
        pos: lan_mouse_ipc::Position,
        /// certificate fingerprint of the connection
        fingerprint: String,
    },
    /// connection closed
    Disconnected {
        addr: SocketAddr,
    },
    /// the port of the listener has changed
    PortChanged(Result<u16, ListenerCreationError>),
    /// emulation was disabled
    EmulationDisabled,
    /// emulation was enabled
    EmulationEnabled,
    /// capture should be released
    ReleaseNotify,
    /// peer sent us a Hello with its build commit hash. Used to
    /// populate `client_manager.peer_commit` from the listen side
    /// too — without this, peer-version visibility silently fails
    /// whenever the outgoing connection in the *other* direction is
    /// broken (one-way setups, asymmetric NAT, peer's TCP listener
    /// down). The connect-side path stays as the primary source;
    /// this is the defensive fallback.
    PeerHello {
        addr: SocketAddr,
        commit: [u8; 8],
    },
}

enum EmulationRequest {
    Reenable,
    Suspend(bool),
    Release(SocketAddr),
    ChangePort(u16),
    Terminate,
}

impl Emulation {
    pub(crate) fn new(
        backend: Option<input_emulation::Backend>,
        listener: LanMouseListener,
        start_suspended: bool,
    ) -> Self {
        let emulation_proxy = EmulationProxy::new(backend, start_suspended);
        let (request_tx, request_rx) = channel();
        let (event_tx, event_rx) = channel();
        let emulation_task = ListenTask {
            listener,
            emulation_proxy,
            request_rx,
            event_tx,
        };
        let task = spawn_local(emulation_task.run());
        Self {
            task,
            request_tx,
            event_rx,
        }
    }

    pub(crate) fn send_leave_event(&self, addr: SocketAddr) {
        self.request_tx
            .send(EmulationRequest::Release(addr))
            .expect("channel closed");
    }

    pub(crate) fn reenable(&self) {
        self.request_tx
            .send(EmulationRequest::Reenable)
            .expect("channel closed");
    }

    pub(crate) fn suspend(&self, suspended: bool) {
        self.request_tx
            .send(EmulationRequest::Suspend(suspended))
            .expect("channel closed");
    }

    pub(crate) fn request_port_change(&self, port: u16) {
        self.request_tx
            .send(EmulationRequest::ChangePort(port))
            .expect("channel closed")
    }

    pub(crate) async fn event(&mut self) -> EmulationEvent {
        self.event_rx.recv().await.expect("channel closed")
    }

    /// wait for termination
    pub(crate) async fn terminate(&mut self) {
        log::debug!("terminating emulation");
        self.request_tx
            .send(EmulationRequest::Terminate)
            .expect("channel closed");
        if let Err(e) = (&mut self.task).await {
            log::warn!("{e}");
        }
    }
}

struct ListenTask {
    listener: LanMouseListener,
    emulation_proxy: EmulationProxy,
    request_rx: Receiver<EmulationRequest>,
    event_tx: Sender<EmulationEvent>,
}

impl ListenTask {
    async fn run(mut self) {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        let mut peers = PeerInputSessions::default();
        let mut rejected_connections = HashMap::new();
        loop {
            select! {
                e = self.listener.next() => {match e {
                    Some(ListenEvent::Msg { event, addr }) => {
                        log::trace!("{event} <-<-<-<-<- {addr}");
                        peers.responses.insert(addr, Instant::now());
                        match event {
                            ProtoEvent::Enter(pos) => {
                                if !self.emulation_proxy.emulation_active.get() {
                                    self.listener.reply(addr, ProtoEvent::Leave(0)).await;
                                    continue;
                                }
                                if let Some(fingerprint) = self.listener.get_certificate_fingerprint(addr).await {
                                    peers.button_sources.insert(addr);
                                    log::info!("releasing capture: {addr} entered this device");
                                    self.event_tx.send(EmulationEvent::ReleaseNotify).expect("channel closed");
                                    self.listener.reply(addr, ProtoEvent::Ack(0)).await;
                                    self.event_tx.send(EmulationEvent::Entered{addr, pos: to_ipc_pos(pos), fingerprint}).expect("channel closed");
                                }
                            }
                            ProtoEvent::Leave(_) => {
                                // Retain the sequence tombstone: delayed state must
                                // not recreate a held button after the route left.
                                peers.button_sources.remove(&addr);
                                self.emulation_proxy.remove(addr);
                                self.listener.reply(addr, ProtoEvent::Ack(0)).await;
                            }
                            ProtoEvent::ButtonSync => self.listener.reply(addr, ProtoEvent::ButtonSync).await,
                            ProtoEvent::PointerButtons {serial,mask} => {
                                let first=!peers.button_serials.contains_key(&addr);
                                if self.emulation_proxy.emulation_active.get() && peers.accept_buttons(addr,serial,mask) {
                                    if first {log::info!("receiving synchronized pointer button state from {addr}");}
                                    self.emulation_proxy.sync_buttons(mask,addr);
                                }
                            }
                            ProtoEvent::Input(event) => {
                                // Once state sync starts, delayed legacy transitions must
                                // not re-press a button released by a newer snapshot.
                                if !peers.button_serials.contains_key(&addr) || !matches!(event,Event::Pointer(PointerEvent::Button {button,..}) if (input_event::BTN_LEFT..=input_event::BTN_FORWARD).contains(&button)) {
                                    self.emulation_proxy.consume(event, addr);
                                }
                            },
                            ProtoEvent::Ping => self.listener.reply(addr, ProtoEvent::Pong(self.emulation_proxy.emulation_active.get())).await,
                            // Peer's version handshake. Echo our own
                            // commit back so the peer's connect-side
                            // receive_loop populates its `peer_commit`,
                            // AND publish a PeerHello upward so our
                            // service can populate ours from the listen
                            // side too — the connect side is the primary
                            // path, but if the outbound direction is
                            // broken (one-way setup, NAT, peer's TCP
                            // listener down) the version display would
                            // otherwise silently say "unknown" while
                            // the peer is in fact happily talking to us.
                            ProtoEvent::Hello { commit } => {
                                self.listener.reply(addr, ProtoEvent::Hello { commit: local_commit() }).await;
                                self.event_tx.send(EmulationEvent::PeerHello { addr, commit }).expect("channel closed");
                            }
                            _ => {}
                        }
                    }
                    Some(ListenEvent::Accept { addr, fingerprint }) => {
                        peers.new_connection(addr);
                        self.event_tx.send(EmulationEvent::Connected { addr, fingerprint }).expect("channel closed");
                    }
                    Some(ListenEvent::Rejected { fingerprint }) => {
                        if rejected_connections.insert(fingerprint.clone(), Instant::now())
                            .is_none_or(|i| i.elapsed() >= Duration::from_secs(2)) {
                                self.event_tx.send(EmulationEvent::ConnectionAttempt { fingerprint }).expect("channel closed");
                            }
                    }
                    None => break
                }}
                event = self.emulation_proxy.event() => {
                    self.event_tx.send(event).expect("channel closed");
                }
                request = self.request_rx.recv() => match request.expect("channel closed") {
                    // reenable emulation
                    EmulationRequest::Reenable => self.emulation_proxy.reenable(),
                    EmulationRequest::Suspend(value) => self.emulation_proxy.suspend(value),
                    // notify the other end that we hit a barrier (should release capture)
                    EmulationRequest::Release(addr) => self.listener.reply(addr, ProtoEvent::Leave(0)).await,
                    EmulationRequest::ChangePort(port) => {
                        self.listener.request_port_change(port);
                        let result = self.listener.port_changed().await;
                        self.event_tx.send(EmulationEvent::PortChanged(result)).expect("channel closed");
                    }
                    EmulationRequest::Terminate => break,
                },
                _ = interval.tick() => {
                    for addr in peers.expire_inputs(Instant::now()) {
                        log::warn!("releasing keys: {addr} not responding!");
                        self.emulation_proxy.remove(addr);
                        self.event_tx.send(EmulationEvent::Disconnected { addr }).expect("channel closed");
                    }
                }
            }
        }
        self.listener.terminate().await;
        self.emulation_proxy.terminate().await;
    }
}

/// proxy handling the actual input emulation,
/// discarding events when it is disabled
pub(crate) struct EmulationProxy {
    emulation_active: Rc<Cell<bool>>,
    exit_requested: Rc<Cell<bool>>,
    request_tx: Sender<ProxyRequest>,
    event_rx: Receiver<EmulationEvent>,
    task: JoinHandle<()>,
}

#[derive(Debug, PartialEq)]
enum ProxyRequest {
    Input(Event, SocketAddr),
    Buttons(u8, SocketAddr),
    Remove(SocketAddr),
    Terminate,
    Reenable,
    Suspend(bool),
}

impl EmulationProxy {
    fn new(backend: Option<input_emulation::Backend>, start_suspended: bool) -> Self {
        let (request_tx, request_rx) = channel();
        let (event_tx, event_rx) = channel();
        let emulation_active = Rc::new(Cell::new(false));
        let exit_requested = Rc::new(Cell::new(false));
        let emulation_task = EmulationTask {
            backend,
            exit_requested: exit_requested.clone(),
            request_rx,
            event_tx,
            handles: Default::default(),
            next_id: 0,
            suspended: start_suspended,
        };
        let task = spawn_local(emulation_task.run());
        Self {
            emulation_active,
            exit_requested,
            request_tx,
            task,
            event_rx,
        }
    }

    async fn event(&mut self) -> EmulationEvent {
        let event = self.event_rx.recv().await.expect("channel closed");
        if let EmulationEvent::EmulationEnabled = event {
            self.emulation_active.replace(true);
        }
        if let EmulationEvent::EmulationDisabled = event {
            self.emulation_active.replace(false);
        }
        event
    }

    fn consume(&self, event: Event, addr: SocketAddr) {
        // ignore events if emulation is currently disabled
        if self.emulation_active.get() {
            self.request_tx
                .send(ProxyRequest::Input(event, addr))
                .expect("channel closed");
        }
    }

    fn sync_buttons(&self, mask: u8, addr: SocketAddr) {
        if self.emulation_active.get() {
            self.request_tx
                .send(ProxyRequest::Buttons(mask, addr))
                .expect("channel closed");
        }
    }

    fn remove(&self, addr: SocketAddr) {
        self.request_tx
            .send(ProxyRequest::Remove(addr))
            .expect("channel closed");
    }

    fn reenable(&self) {
        self.request_tx
            .send(ProxyRequest::Reenable)
            .expect("channel closed");
    }

    fn suspend(&self, suspended: bool) {
        if suspended {
            self.emulation_active.set(false);
        }
        self.request_tx
            .send(ProxyRequest::Suspend(suspended))
            .expect("channel closed");
    }

    async fn terminate(&mut self) {
        self.exit_requested.replace(true);
        self.request_tx
            .send(ProxyRequest::Terminate)
            .expect("channel closed");
        let _ = (&mut self.task).await;
    }
}

struct EmulationTask {
    backend: Option<input_emulation::Backend>,
    exit_requested: Rc<Cell<bool>>,
    request_rx: Receiver<ProxyRequest>,
    event_tx: Sender<EmulationEvent>,
    handles: HashMap<SocketAddr, EmulationHandle>,
    next_id: EmulationHandle,
    suspended: bool,
}

impl EmulationTask {
    async fn run(mut self) {
        loop {
            if !self.suspended {
                if let Err(e) = self.do_emulation().await {
                    log::warn!("input emulation exited: {e}");
                }
            }
            if self.exit_requested.get() {
                break;
            }
            // wait for reenable request
            loop {
                match self.request_rx.recv().await.expect("channel closed") {
                    ProxyRequest::Reenable if !self.suspended => break,
                    ProxyRequest::Reenable => {}
                    ProxyRequest::Suspend(value) => {
                        self.suspended = value;
                        if !value {
                            break;
                        }
                    }
                    ProxyRequest::Terminate => return,
                    ProxyRequest::Input(..) | ProxyRequest::Buttons(..) => { /* emulation inactive => ignore */
                    }
                    ProxyRequest::Remove(..) => { /* emulation inactive => ignore */ }
                }
            }
        }
    }

    async fn do_emulation(&mut self) -> Result<(), InputEmulationError> {
        log::info!("creating input emulation ...");
        let mut emulation = tokio::select! {
            r = InputEmulation::new(self.backend) => r?,
            // allow termination event while requesting input emulation
            _ = wait_for_termination(&mut self.request_rx) => return Ok(()),
        };

        // used to send enabled and disabled events
        let _emulation_guard = DropGuard::new(
            self.event_tx.clone(),
            EmulationEvent::EmulationEnabled,
            EmulationEvent::EmulationDisabled,
        );

        // create active handles
        if let Err(e) = self.create_clients(&mut emulation).await {
            emulation.terminate().await;
            return Err(e);
        }

        let res = self.do_emulation_session(&mut emulation).await;
        // FIXME replace with async drop when stabilized
        emulation.terminate().await;
        res
    }

    async fn create_clients(
        &mut self,
        emulation: &mut InputEmulation,
    ) -> Result<(), InputEmulationError> {
        for handle in self.handles.values() {
            tokio::select! {
                _ = emulation.create(*handle) => {},
                _ = wait_for_termination(&mut self.request_rx) => return Ok(()),
            }
        }
        Ok(())
    }

    async fn do_emulation_session(
        &mut self,
        emulation: &mut InputEmulation,
    ) -> Result<(), InputEmulationError> {
        let mut pending = None;
        loop {
            tokio::select! {
                e = receive_input_batch(&mut self.request_rx, &mut pending) => match e {
                    ProxyRequest::Input(event, addr) => {
                        let handle = match self.handles.get(&addr) {
                            Some(&handle) => handle,
                            None => {
                                let handle = self.next_id;
                                self.next_id += 1;
                                emulation.create(handle).await;
                                self.handles.insert(addr, handle);
                                handle
                            }
                        };
                        emulation.consume(event, handle).await?;
                    },
                    ProxyRequest::Buttons(mask,addr) => {
                        let handle=match self.handles.get(&addr) {
                            Some(&h)=>h,
                            None=>{let h=self.next_id;self.next_id+=1;emulation.create(h).await;self.handles.insert(addr,h);h}
                        };
                        emulation.sync_pointer_buttons(handle,mask).await?;
                    }
                    ProxyRequest::Remove(addr) => {
                        if let Some(handle) = self.handles.remove(&addr) {
                            emulation.destroy(handle).await;
                        }
                    }
                    ProxyRequest::Terminate => break Ok(()),
                    ProxyRequest::Reenable => continue,
                    ProxyRequest::Suspend(true) => {
                        self.suspended = true;
                        break Ok(());
                    },
                    ProxyRequest::Suspend(false) => {},
                },
            }
        }
    }
}

/// A native-input timeout releases held state, while the authenticated DTLS
/// connection may remain alive. Keep route authorization and sequence ordering
/// until an explicit Leave or a new connection changes that route.
#[derive(Default)]
struct PeerInputSessions {
    responses: HashMap<SocketAddr, Instant>,
    button_serials: HashMap<SocketAddr, u32>,
    button_sources: HashSet<SocketAddr>,
}
impl PeerInputSessions {
    fn new_connection(&mut self, addr: SocketAddr) {
        self.button_serials.remove(&addr);
        self.button_sources.remove(&addr);
    }
    fn accept_buttons(&mut self, addr: SocketAddr, serial: u32, mask: u8) -> bool {
        self.button_sources.contains(&addr)
            && accept_button_snapshot(&mut self.button_serials, addr, serial, mask)
    }
    fn expire_inputs(&mut self, now: Instant) -> Vec<SocketAddr> {
        let mut expired = Vec::new();
        self.responses.retain(|&addr, stamp| {
            if now.saturating_duration_since(*stamp) > Duration::from_secs(1) {
                // The caller releases native held keys/buttons. Silence is
                // not a Leave: keep route authorization and sequence tombstones
                // so fresh clicks resume without replaying an old held drag.
                expired.push(addr);
                false
            } else {
                true
            }
        });
        expired
    }
}

fn accept_button_snapshot(
    serials: &mut HashMap<SocketAddr, u32>,
    addr: SocketAddr,
    serial: u32,
    mask: u8,
) -> bool {
    if mask & !31 != 0 {
        return false;
    }
    if serials
        .get(&addr)
        .is_some_and(|previous| serial.wrapping_sub(*previous) as i32 <= 0)
    {
        return false;
    }
    serials.insert(addr, serial);
    true
}

#[cfg(test)]
mod button_snapshot_tests {
    use super::*;
    #[test]
    fn clicks_resume_after_input_timeout_on_the_same_connection() {
        let addr = "127.0.0.1:4242".parse().unwrap();
        let now = Instant::now();
        let mut peers = PeerInputSessions::default();
        peers.new_connection(addr);
        peers.button_sources.insert(addr);
        peers.responses.insert(addr, now);
        assert!(peers.accept_buttons(addr, 40, 1));
        assert!(peers.accept_buttons(addr, 41, 0));
        assert_eq!(
            peers.expire_inputs(now + Duration::from_secs(2)),
            vec![addr]
        );
        // No new Enter is sent when DTLS survives the brief stall. Motion and
        // keyboard already resume on this connection; buttons must resume too.
        peers.responses.insert(addr, now + Duration::from_secs(2));
        assert!(
            peers.accept_buttons(addr, 42, 1),
            "timeout made live-route clicks permanently unavailable"
        );
        assert!(peers.accept_buttons(addr, 43, 0));
        assert!(
            !peers.accept_buttons(addr, 40, 1),
            "a stale down must not be resurrected"
        );
        // An explicit Leave still blocks all later snapshots.
        peers.button_sources.remove(&addr);
        assert!(!peers.accept_buttons(addr, 44, 1));
    }
    #[test]
    fn timeout_does_not_repress_a_held_drag_or_reauthorize_a_closed_route() {
        let addr = "127.0.0.1:4242".parse().unwrap();
        let now = Instant::now();
        let mut peers = PeerInputSessions::default();
        peers.button_sources.insert(addr);
        peers.responses.insert(addr, now);
        assert!(peers.accept_buttons(addr, 10, 1));
        peers.expire_inputs(now + Duration::from_secs(2));
        assert!(
            !peers.accept_buttons(addr, 10, 1),
            "held snapshot replayed after native cleanup"
        );
        assert!(peers.accept_buttons(addr, 11, 0));
        peers.button_sources.remove(&addr);
        peers.responses.insert(addr, now);
        peers.expire_inputs(now + Duration::from_secs(2));
        assert!(!peers.accept_buttons(addr, 12, 1));
        peers.new_connection(addr);
        assert!(!peers.accept_buttons(addr, 0, 1));
        peers.button_sources.insert(addr);
        assert!(peers.accept_buttons(addr, 0, 1));
    }
    #[test]
    fn duplicate_and_reordered_snapshots_cannot_repress_released_button() {
        let addr = "127.0.0.1:4242".parse().unwrap();
        let mut serials = HashMap::new();
        assert!(accept_button_snapshot(&mut serials, addr, 41, 1));
        assert!(accept_button_snapshot(&mut serials, addr, 43, 0)); // release 42 was lost
        assert!(!accept_button_snapshot(&mut serials, addr, 41, 1));
        assert!(!accept_button_snapshot(&mut serials, addr, 43, 0));
        assert!(!accept_button_snapshot(&mut serials, addr, 44, 128));
        let other = "127.0.0.2:4242".parse().unwrap();
        assert!(accept_button_snapshot(&mut serials, other, u32::MAX, 1));
        assert!(accept_button_snapshot(&mut serials, other, 0, 0));
    }
}

// Drain only input already queued: no timer and no added latency at normal rates.
// Keep boundaries (keys, buttons, wheel, control, peer, direction) in exact order.
async fn receive_input_batch(
    rx: &mut Receiver<ProxyRequest>,
    pending: &mut Option<ProxyRequest>,
) -> ProxyRequest {
    let mut first = match pending.take() {
        Some(value) => value,
        None => rx.recv().await.expect("channel closed"),
    };
    if !matches!(
        first,
        ProxyRequest::Input(Event::Pointer(PointerEvent::Motion { .. }), _)
    ) {
        return first;
    }
    // Bound work per turn so the coordinator and disconnect handling stay fair.
    for _ in 1..64 {
        let Some(Some(next)) = rx.recv().now_or_never() else {
            break;
        };
        let merged = match (&mut first, &next) {
            (
                ProxyRequest::Input(Event::Pointer(PointerEvent::Motion { time, dx, dy }), addr),
                ProxyRequest::Input(
                    Event::Pointer(PointerEvent::Motion {
                        time: nt,
                        dx: nx,
                        dy: ny,
                    }),
                    na,
                ),
            ) if addr == na
                && nt.wrapping_sub(*time) <= 8
                && *dx * nx >= 0.
                && *dy * ny >= 0.
                && (*dx + nx).is_finite()
                && (*dy + ny).is_finite() =>
            {
                *time = *nt;
                *dx += nx;
                *dy += ny;
                true
            }
            _ => false,
        };
        if !merged {
            *pending = Some(next);
            break;
        }
    }
    first
}

fn to_ipc_pos(pos: Position) -> lan_mouse_ipc::Position {
    match pos {
        Position::Left => lan_mouse_ipc::Position::Left,
        Position::Right => lan_mouse_ipc::Position::Right,
        Position::Top => lan_mouse_ipc::Position::Top,
        Position::Bottom => lan_mouse_ipc::Position::Bottom,
    }
}

async fn wait_for_termination(rx: &mut Receiver<ProxyRequest>) {
    loop {
        match rx.recv().await.expect("channel closed") {
            ProxyRequest::Terminate => return,
            ProxyRequest::Input(_, _) | ProxyRequest::Buttons(_, _) => continue,
            ProxyRequest::Remove(_) => continue,
            ProxyRequest::Reenable => continue,
            ProxyRequest::Suspend(_) => continue,
        }
    }
}

struct DropGuard<T> {
    tx: Sender<T>,
    on_drop: Option<T>,
}

impl<T> DropGuard<T> {
    fn new(tx: Sender<T>, on_new: T, on_drop: T) -> Self {
        tx.send(on_new).expect("channel closed");
        let on_drop = Some(on_drop);
        Self { tx, on_drop }
    }
}

impl<T> Drop for DropGuard<T> {
    fn drop(&mut self) {
        self.tx
            .send(self.on_drop.take().expect("item"))
            .expect("channel closed");
    }
}

#[cfg(test)]
mod latency_tests {
    use super::*;
    use input_event::KeyboardEvent;

    fn motion(time: u32, dx: f64, addr: SocketAddr) -> ProxyRequest {
        ProxyRequest::Input(
            Event::Pointer(PointerEvent::Motion { time, dx, dy: 0. }),
            addr,
        )
    }

    #[tokio::test]
    async fn mouse_backlog_preserves_displacement_and_key_release_order() {
        let addr = "127.0.0.1:4242".parse().unwrap();
        let (tx, mut rx) = channel();
        let key = |state| {
            ProxyRequest::Input(
                Event::Keyboard(KeyboardEvent::Key {
                    time: 0,
                    key: 30,
                    state,
                }),
                addr,
            )
        };
        tx.send(key(1)).unwrap();
        for time in 0..2000 {
            tx.send(motion(time, 1., addr)).unwrap();
        }
        tx.send(key(0)).unwrap();
        tx.send(ProxyRequest::Suspend(true)).unwrap();
        let mut pending = None;
        assert_eq!(receive_input_batch(&mut rx, &mut pending).await, key(1));
        let mut total = 0.;
        let mut native_calls = 0;
        loop {
            match receive_input_batch(&mut rx, &mut pending).await {
                ProxyRequest::Input(Event::Pointer(PointerEvent::Motion { dx, .. }), _) => {
                    total += dx;
                    native_calls += 1;
                }
                event => {
                    assert_eq!(event, key(0));
                    break;
                }
            }
        }
        assert_eq!(total, 2000.);
        assert!(native_calls <= 32, "native calls: {native_calls}");
        assert_eq!(
            receive_input_batch(&mut rx, &mut pending).await,
            ProxyRequest::Suspend(true)
        );
    }

    #[tokio::test]
    async fn coalescing_preserves_peer_direction_buttons_wheel_and_control_boundaries() {
        let a = "127.0.0.1:4242".parse().unwrap();
        let b = "127.0.0.2:4242".parse().unwrap();
        let events = [
            motion(0, 1., a),
            motion(1, -1., a),
            motion(2, -1., b),
            ProxyRequest::Input(
                Event::Pointer(PointerEvent::Button {
                    time: 3,
                    button: 272,
                    state: 1,
                }),
                b,
            ),
            motion(4, -1., b),
            ProxyRequest::Input(
                Event::Pointer(PointerEvent::AxisDiscrete120 {
                    axis: 0,
                    value: 120,
                }),
                b,
            ),
            motion(5, -1., b),
            ProxyRequest::Remove(b),
            ProxyRequest::Terminate,
        ];
        let (tx, mut rx) = channel();
        for event in &events {
            // Events are Copy; control requests are reconstructed for this fixture.
            let copy = match event {
                ProxyRequest::Input(e, addr) => ProxyRequest::Input(*e, *addr),
                ProxyRequest::Remove(addr) => ProxyRequest::Remove(*addr),
                ProxyRequest::Terminate => ProxyRequest::Terminate,
                _ => unreachable!(),
            };
            tx.send(copy).unwrap();
        }
        let mut pending = None;
        for expected in events {
            assert_eq!(receive_input_batch(&mut rx, &mut pending).await, expected);
        }
    }
}
