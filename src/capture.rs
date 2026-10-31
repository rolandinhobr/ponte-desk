use std::{
    cell::{Cell, RefCell},
    collections::{HashSet, VecDeque},
    rc::Rc,
    time::{Duration, Instant},
};

use futures::StreamExt;
use input_capture::{
    CaptureError, CaptureEvent, CaptureHandle, InputCapture, InputCaptureError, Position,
};
use input_event::{Event, KeyboardEvent, PointerEvent, scancode};
use lan_mouse_proto::ProtoEvent;
use local_channel::mpsc::{Receiver, Sender, channel};
use tokio::task::{JoinHandle, spawn_local};
use tokio_util::sync::CancellationToken;

use crate::connect::{LanMouseConnection, LanMouseConnectionError};

pub(crate) struct Capture {
    cancellation_token: CancellationToken,
    request_tx: Sender<CaptureRequest>,
    task: JoinHandle<()>,
    event_rx: Receiver<ICaptureEvent>,
}

pub(crate) enum ICaptureEvent {
    DirectMode(bool),
    /// a client was entered
    CaptureBegin(CaptureHandle),
    /// capture disabled
    CaptureDisabled,
    /// capture disabled
    CaptureEnabled,
    /// A (new) client was entered.
    /// In contrast to [`ICaptureEvent::CaptureBegin`] this
    /// event is only triggered when the capture was
    /// explicitly released in the meantime by
    /// either the remote client leaving its device region,
    /// a new device entering the screen or the release bind.
    ClientEntered(u64),
    /// The previously active client was left, i.e. capture
    /// was released for the given handle. Mirrors
    /// [`ICaptureEvent::ClientEntered`] for the leave side
    /// and fires on every release path (release-bind chord,
    /// remote `Leave`, explicit `Release` request, send
    /// failure, or destroy of the active capture).
    ClientLeft(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaptureType {
    /// a normal input capture
    Default,
    /// A capture only interested in [`CaptureEvent::Begin`] events.
    /// The capture is released immediately, if there is no
    /// Default capture at the same position.
    EnterOnly,
}

#[derive(Clone, Debug)]
enum CaptureRequest {
    Direct(bool),
    /// capture must release the mouse
    Release,
    /// add a capture client
    Create(CaptureHandle, Position, CaptureType),
    /// destory a capture client
    Destroy(CaptureHandle),
    /// reenable input capture
    Reenable,
    Suspend(bool),
    /// set release bind
    SetReleaseBind(Vec<scancode::Linux>),
}

impl Capture {
    pub(crate) fn new(
        backend: Option<input_capture::Backend>,
        conn: LanMouseConnection,
        release_bind: Vec<scancode::Linux>,
        start_suspended: bool,
    ) -> Self {
        let (request_tx, request_rx) = channel();
        let (event_tx, event_rx) = channel();
        let cancellation_token = CancellationToken::new();
        let capture_task = CaptureTask {
            active_client: None,
            direct: false,
            pending_direct: VecDeque::new(),
            button_sync_clients: HashSet::new(),
            button_serial: crate::integration::now_ms() as u32,
            button_mask: 0,
            waiting_since: None,
            backend,
            cancellation_token: cancellation_token.clone(),
            captures: Default::default(),
            conn,
            event_tx,
            request_rx,
            release_bind: Rc::new(RefCell::new(release_bind)),
            state: Default::default(),
            suspended: start_suspended,
        };
        let task = spawn_local(capture_task.run());
        Self {
            cancellation_token,
            request_tx,
            task,
            event_rx,
        }
    }

    pub(crate) fn reenable(&self) {
        self.request_tx
            .send(CaptureRequest::Reenable)
            .expect("channel closed");
    }

    pub(crate) fn direct(&self, value: bool) {
        let _ = self.request_tx.send(CaptureRequest::Direct(value));
    }

    pub(crate) fn suspend(&self, suspended: bool) {
        self.request_tx
            .send(CaptureRequest::Suspend(suspended))
            .expect("channel closed");
    }

    pub(crate) async fn terminate(&mut self) {
        self.cancellation_token.cancel();
        log::debug!("terminating capture");
        if let Err(e) = (&mut self.task).await {
            log::warn!("{e}");
        }
    }

    pub(crate) fn create(
        &self,
        handle: CaptureHandle,
        pos: lan_mouse_ipc::Position,
        capture_type: CaptureType,
    ) {
        let pos = to_capture_pos(pos);
        self.request_tx
            .send(CaptureRequest::Create(handle, pos, capture_type))
            .expect("channel closed");
    }

    pub(crate) fn destroy(&self, handle: CaptureHandle) {
        self.request_tx
            .send(CaptureRequest::Destroy(handle))
            .expect("channel closed");
    }

    pub(crate) fn release(&self) {
        self.request_tx
            .send(CaptureRequest::Release)
            .expect("channel closed");
    }

    pub(crate) async fn event(&mut self) -> ICaptureEvent {
        self.event_rx.recv().await.expect("channel closed")
    }

    pub(crate) fn set_release_bind(&mut self, bind: Vec<scancode::Linux>) {
        let _ = self.request_tx.send(CaptureRequest::SetReleaseBind(bind));
    }
}

/// debounce a statement `$st`, i.e. the statement is executed only if the
/// time since the previous execution is at least `$dur`.
/// `$prev` is used to keep track of this timestamp
macro_rules! debounce {
    ($prev:ident, $dur:expr, $st:stmt) => {
        let exec = match $prev.get() {
            None => true,
            Some(instant) if instant.elapsed() > $dur => true,
            _ => false,
        };
        if exec {
            $prev.replace(Some(Instant::now()));
            $st
        }
    };
}

struct CaptureTask {
    direct: bool,
    pending_direct: VecDeque<Event>,
    button_sync_clients: HashSet<CaptureHandle>,
    button_serial: u32,
    button_mask: u8,
    waiting_since: Option<Instant>,
    active_client: Option<CaptureHandle>,
    backend: Option<input_capture::Backend>,
    cancellation_token: CancellationToken,
    captures: Vec<(CaptureHandle, Position, CaptureType)>,
    conn: LanMouseConnection,
    event_tx: Sender<ICaptureEvent>,
    release_bind: Rc<RefCell<Vec<scancode::Linux>>>,
    request_rx: Receiver<CaptureRequest>,
    state: State,
    suspended: bool,
}

impl CaptureTask {
    fn add_capture(&mut self, handle: CaptureHandle, pos: Position, capture_type: CaptureType) {
        self.captures.push((handle, pos, capture_type));
    }

    fn remove_capture(&mut self, handle: CaptureHandle) {
        self.captures.retain(|&(h, ..)| handle != h);
    }

    fn is_default_capture_at(&self, pos: Position) -> bool {
        self.captures
            .iter()
            .any(|&(_, p, t)| p == pos && t == CaptureType::Default)
    }

    fn get_pos(&self, handle: CaptureHandle) -> Position {
        self.captures
            .iter()
            .find(|(h, ..)| *h == handle)
            .expect("no such capture")
            .1
    }

    fn get_type(&self, handle: CaptureHandle) -> CaptureType {
        self.captures
            .iter()
            .find(|(h, ..)| *h == handle)
            .expect("no such capture")
            .2
    }

    async fn run(mut self) {
        loop {
            if !self.suspended {
                if let Err(e) = self.do_capture().await {
                    log::warn!("input capture exited: {e}");
                }
            }
            loop {
                tokio::select! {
                    r = self.request_rx.recv() => match r.expect("channel closed") {
                        CaptureRequest::Direct(value)=>{self.direct=value;},
                        CaptureRequest::Reenable if !self.suspended => break,
                        CaptureRequest::Reenable => {},
                        CaptureRequest::Suspend(value) => {
                            self.suspended = value;
                            if !value { break; }
                        },
                        CaptureRequest::Create(h, p, t) => self.add_capture(h, p, t),
                        CaptureRequest::Destroy(h) => self.remove_capture(h),
                        CaptureRequest::Release => { /* nothing to do */ }
                        CaptureRequest::SetReleaseBind(bind) => {
                            self.release_bind.borrow_mut().clone_from(&bind);
                        }
                    },
                    _ = self.cancellation_token.cancelled() => return,
                }
            }
        }
    }

    async fn do_capture(&mut self) -> Result<(), InputCaptureError> {
        /* allow cancelling capture request */
        let mut capture = tokio::select! {
            r = InputCapture::new(self.backend) => r?,
            _ = self.cancellation_token.cancelled() => return Ok(()),
        };

        let _capture_guard = DropGuard::new(
            self.event_tx.clone(),
            ICaptureEvent::CaptureEnabled,
            ICaptureEvent::CaptureDisabled,
        );

        /* create barriers for active clients */
        let r = self.create_captures(&mut capture).await;
        if let Err(e) = r {
            capture.terminate().await?;
            return Err(e.into());
        }

        self.apply_direct(&mut capture).await?;
        let r = self.do_capture_session(&mut capture).await;

        // FIXME replace with async drop when stabilized
        capture.terminate().await?;

        r
    }

    async fn create_captures(&mut self, capture: &mut InputCapture) -> Result<(), CaptureError> {
        let captures = self.captures.clone();
        for (handle, pos, _type) in captures {
            tokio::select! {
                r = capture.create(handle, pos) => r?,
                _ = self.cancellation_token.cancelled() => return Ok(()),
            }
        }
        Ok(())
    }

    async fn apply_direct(&self, capture: &mut InputCapture) -> Result<(), CaptureError> {
        let handle = if self.direct {
            self.captures
                .iter()
                .find(|(_, _, t)| *t == CaptureType::Default)
                .map(|(h, _, _)| *h)
        } else {
            None
        };
        capture.direct(handle).await?;
        let _ = self
            .event_tx
            .send(ICaptureEvent::DirectMode(handle.is_some()));
        Ok(())
    }
    fn pause_direct(&self) {
        if self.direct {
            let _ = std::fs::write(
                crate::integration::preferences_root().join("manual-pause"),
                b"direct input released",
            );
        }
    }

    async fn send_input(
        &mut self,
        event: Event,
        handle: CaptureHandle,
    ) -> Result<(), LanMouseConnectionError> {
        let event = button_wire_event(
            event,
            self.button_sync_clients.contains(&handle),
            &mut self.button_serial,
            &mut self.button_mask,
        );
        self.conn.send(event, handle).await
    }

    async fn do_capture_session(
        &mut self,
        capture: &mut InputCapture,
    ) -> Result<(), InputCaptureError> {
        let mut handshake = tokio::time::interval(Duration::from_millis(250));
        let mut buttons = tokio::time::interval(Duration::from_millis(100));
        buttons.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _=buttons.tick(), if self.active_client.is_some() && self.state == State::Sending => {
                    let handle=self.active_client.unwrap();
                    let event=if self.button_sync_clients.contains(&handle) {
                        ProtoEvent::PointerButtons {serial:self.button_serial,mask:self.button_mask}
                    } else { ProtoEvent::ButtonSync };
                    if self.conn.send(event,handle).await.is_err(){self.pause_direct();self.release_capture(capture).await?;}
                },
                _=handshake.tick(), if self.direct && self.waiting_since.is_some()=>{
                    if self.waiting_since.is_some_and(|t|t.elapsed()>Duration::from_secs(2)) {
                        self.pause_direct();self.release_capture(capture).await?;
                    } else if let Some(handle)=self.active_client {
                        if let Err(error)=self.conn.send(ProtoEvent::Enter(to_proto_pos(self.get_pos(handle).opposite())),handle).await{if !matches!(error,LanMouseConnectionError::NotConnected|LanMouseConnectionError::TargetEmulationDisabled){self.pause_direct();self.release_capture(capture).await?;}}
                    }
                },
                event = capture.next() => match event {
                    Some(event) => self.handle_capture_event(capture, event?).await?,
                    None => return Ok(()),
                },
                (handle, event) = self.conn.recv() => {
                    if let Some(active) = self.active_client {
                        if handle != active {
                            // we only care about events coming from the client we are currently connected to
                            // only `Ack` and `Leave` are relevant
                            continue
                        }
                    }

                    match event {
                        ProtoEvent::ButtonSync => { if self.button_sync_clients.insert(handle) { log::info!("pointer button state synchronization enabled for client {handle}"); } }
                        // connection acknowlegded => set state to Sending
                        ProtoEvent::Ack(_) => {
                            log::info!("client {handle} acknowledged the connection!");
                            self.state = State::Sending;self.waiting_since=None;
                            while let Some(event)=self.pending_direct.pop_front(){
                                if self.send_input(event,handle).await.is_err(){self.pause_direct();self.release_capture(capture).await?;break;}
                            }
                        }
                        // client disconnected
                        ProtoEvent::Leave(_) => {
                            log::info!("releasing capture: left remote client device region");
                            self.pause_direct();self.release_capture(capture).await?;
                        },
                        _ => {}
                    }
                },
                e = self.request_rx.recv() => match e.expect("channel closed") {
                    CaptureRequest::Direct(value)=>{if value!=self.direct{self.release_capture(capture).await?;self.direct=value;self.apply_direct(capture).await?;}},
                    CaptureRequest::Reenable => { /* already active */ },
                    CaptureRequest::Suspend(true) => {
                        self.suspended = true;
                        self.release_capture(capture).await?;
                        return Ok(());
                    },
                    CaptureRequest::Suspend(false) => {},
                    CaptureRequest::Release => {self.pause_direct();self.release_capture(capture).await?;},
                    CaptureRequest::Create(h, p, t) => {
                        self.add_capture(h, p, t);
                        capture.create(h, p).await?;
                        if self.direct{self.apply_direct(capture).await?;}
                    }
                    CaptureRequest::Destroy(h) => {
                        // If the capture we're tearing down is the
                        // currently-active one, treat this as a
                        // release for hook purposes. The release_capture
                        // path also clears active_client and flushes
                        // pressed-key state to the peer; without this,
                        // `cli deactivate` (or a hostname change
                        // re-creating the client) would skip leave_hook.
                        if self.active_client == Some(h) {
                            self.release_capture(capture).await?;
                        }
                        self.remove_capture(h);
                        capture.destroy(h).await?;
                    }
                    CaptureRequest::SetReleaseBind(bind) => {
                        self.release_bind.borrow_mut().clone_from(&bind);
                    }
                },
                _ = self.cancellation_token.cancelled() => break,
            }
        }
        Ok(())
    }

    async fn handle_capture_event(
        &mut self,
        capture: &mut InputCapture,
        event: (CaptureHandle, CaptureEvent),
    ) -> Result<(), CaptureError> {
        let (handle, event) = event;
        log::trace!("({handle}): {event:?}");

        if capture.keys_pressed(&self.release_bind.borrow()) {
            log::info!("releasing capture: release-bind pressed");
            self.pause_direct();
            return self.release_capture(capture).await;
        }

        if event == CaptureEvent::Begin {
            self.event_tx
                .send(ICaptureEvent::CaptureBegin(handle))
                .expect("channel closed");
        }

        // enter only capture (for incoming connections)
        if self.get_type(handle) == CaptureType::EnterOnly {
            // if there is no active outgoing connection at the current capture,
            // we release the capture
            if !self.is_default_capture_at(self.get_pos(handle)) {
                log::info!("releasing capture: no active client at this position");
                capture.release().await?;
            }
            // we dont care about events from incoming handles except for releasing the capture
            return Ok(());
        }

        // activated a new client
        if event == CaptureEvent::Begin && Some(handle) != self.active_client {
            self.state = State::WaitingForAck;
            if self.direct {
                self.waiting_since = Some(Instant::now());
            }
            self.active_client.replace(handle);
            self.event_tx
                .send(ICaptureEvent::ClientEntered(handle))
                .expect("channel closed");
        }

        let opposite_pos = to_proto_pos(self.get_pos(handle).opposite());

        let event = match event {
            CaptureEvent::Begin => ProtoEvent::Enter(opposite_pos),
            CaptureEvent::Input(e) => match self.state {
                // connection not acknowledged, repeat `Enter` event
                State::WaitingForAck if self.direct => {
                    if !queue_direct(&mut self.pending_direct, e) {
                        self.pause_direct();
                        return self.release_capture(capture).await;
                    }
                    return Ok(());
                }
                State::WaitingForAck => ProtoEvent::Enter(opposite_pos),
                State::Sending => ProtoEvent::Input(e),
            },
        };

        let result = match event {
            ProtoEvent::Input(e) => self.send_input(e, handle).await,
            e => self.conn.send(e, handle).await,
        };
        if let Err(e) = result {
            if self.direct
                && self.state == State::WaitingForAck
                && matches!(
                    e,
                    LanMouseConnectionError::NotConnected
                        | LanMouseConnectionError::TargetEmulationDisabled
                )
            {
                return Ok(());
            }
            const DUR: Duration = Duration::from_millis(500);
            debounce!(PREV_LOG, DUR, log::warn!("releasing capture: {e}"));
            // Funnel through release_capture so the leave_hook
            // fires and active_client is cleared (without this the
            // active_client field would stay stale until the next
            // Begin from a different handle).
            self.pause_direct();
            self.release_capture(capture).await?;
        }
        Ok(())
    }

    async fn release_capture(&mut self, capture: &mut InputCapture) -> Result<(), CaptureError> {
        self.pending_direct.clear();
        self.waiting_since = None;
        if self.direct {
            capture.direct(None).await?;
            let _ = self.event_tx.send(ICaptureEvent::DirectMode(false));
        }
        // If we have an active client, notify them we're leaving
        if let Some(handle) = self.active_client.take() {
            self.button_mask = 0;
            self.button_serial = self.button_serial.wrapping_add(1);
            if self.button_sync_clients.contains(&handle) {
                let _ = self
                    .conn
                    .send(
                        ProtoEvent::PointerButtons {
                            serial: self.button_serial,
                            mask: 0,
                        },
                        handle,
                    )
                    .await;
            }
            // Surface the leave to the service layer so it can fire
            // the per-client leave_hook. Sent before the network
            // teardown below so we never race against the peer
            // disappearing.
            self.event_tx
                .send(ICaptureEvent::ClientLeft(handle))
                .expect("channel closed");
            // Synthesize key-up events for every key still held in the
            // capture's pressed_keys set BEFORE sending Leave. Without
            // this, pressing the release-bind chord (typically all four
            // modifiers) leaves the peer with phantom held modifiers:
            // the down events were forwarded while capture was active,
            // but the matching up events arrive after the local tap
            // flips to passthrough and never reach the peer. The peer
            // then runs every subsequent keystroke through those held
            // mods until its watchdog times out (1+ s) or our Leave
            // arrives — and Leave can be lost over UDP/DTLS.
            for key in capture.take_pressed_keys() {
                let key_up = ProtoEvent::Input(Event::Keyboard(KeyboardEvent::Key {
                    time: 0,
                    key: key as u32,
                    state: 0,
                }));
                if let Err(e) = self.conn.send(key_up, handle).await {
                    log::warn!("failed to send key-up to client {handle}: {e}");
                }
            }
            // Reset the modifier mask too. The peer's input-emulation
            // layer keeps a separate XKB-style modifier state that's
            // updated by KeyboardEvent::Modifiers, distinct from the
            // pressed_keys set drained above. Without this, an
            // already-locked CapsLock would survive the release.
            let mods_zero = ProtoEvent::Input(Event::Keyboard(KeyboardEvent::Modifiers {
                depressed: 0,
                latched: 0,
                locked: 0,
                group: 0,
            }));
            if let Err(e) = self.conn.send(mods_zero, handle).await {
                log::warn!("failed to reset modifiers on client {handle}: {e}");
            }

            log::info!("sending Leave event to client {handle}");
            if let Err(e) = self.conn.send(ProtoEvent::Leave(0), handle).await {
                log::warn!("failed to send Leave to client {handle}: {e}");
            }
        }
        capture.release().await
    }
}

thread_local! {
    static PREV_LOG: Cell<Option<Instant>> = const { Cell::new(None) };
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum State {
    #[default]
    WaitingForAck,
    Sending,
}

fn button_wire_event(event: Event, supported: bool, serial: &mut u32, mask: &mut u8) -> ProtoEvent {
    if let Event::Pointer(PointerEvent::Button { button, state, .. }) = event {
        if let Some(index) = button.checked_sub(input_event::BTN_LEFT).filter(|i| *i < 5) {
            let next = if state == 1 {
                *mask | (1 << index)
            } else {
                *mask & !(1 << index)
            };
            if next != *mask {
                *mask = next;
                *serial = serial.wrapping_add(1);
            }
            if supported {
                return ProtoEvent::PointerButtons {
                    serial: *serial,
                    mask: *mask,
                };
            }
        }
    }
    ProtoEvent::Input(event)
}
#[cfg(test)]
mod button_wire_tests {
    use super::*;
    fn button(state: u32) -> Event {
        Event::Pointer(PointerEvent::Button {
            time: 0,
            button: input_event::BTN_LEFT,
            state,
        })
    }
    #[test]
    fn negotiated_full_state_preserves_legacy_and_ignores_duplicate_transitions() {
        let (mut serial, mut mask) = (40, 0);
        assert!(matches!(
            button_wire_event(button(1), false, &mut serial, &mut mask),
            ProtoEvent::Input(_)
        ));
        assert!(matches!(
            button_wire_event(button(1), true, &mut serial, &mut mask),
            ProtoEvent::PointerButtons {
                serial: 41,
                mask: 1
            }
        ));
        assert!(matches!(
            button_wire_event(button(0), true, &mut serial, &mut mask),
            ProtoEvent::PointerButtons {
                serial: 42,
                mask: 0
            }
        ));
        assert!(matches!(
            button_wire_event(button(0), true, &mut serial, &mut mask),
            ProtoEvent::PointerButtons {
                serial: 42,
                mask: 0
            }
        ));
    }
}

fn to_capture_pos(pos: lan_mouse_ipc::Position) -> input_capture::Position {
    match pos {
        lan_mouse_ipc::Position::Left => input_capture::Position::Left,
        lan_mouse_ipc::Position::Right => input_capture::Position::Right,
        lan_mouse_ipc::Position::Top => input_capture::Position::Top,
        lan_mouse_ipc::Position::Bottom => input_capture::Position::Bottom,
    }
}

fn to_proto_pos(pos: input_capture::Position) -> lan_mouse_proto::Position {
    match pos {
        input_capture::Position::Left => lan_mouse_proto::Position::Left,
        input_capture::Position::Right => lan_mouse_proto::Position::Right,
        input_capture::Position::Top => lan_mouse_proto::Position::Top,
        input_capture::Position::Bottom => lan_mouse_proto::Position::Bottom,
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

// Coalesce high-rate pointer motion while retaining ordered key/button transitions
// until Enter is acknowledged. Capacity is fixed even if the peer is unresponsive.
fn queue_direct(queue: &mut VecDeque<Event>, event: Event) -> bool {
    if let Event::Pointer(input_event::PointerEvent::Motion { time, dx, dy }) = event {
        if let Some(Event::Pointer(input_event::PointerEvent::Motion {
            time: last_time,
            dx: last_dx,
            dy: last_dy,
        })) = queue.back_mut()
        {
            *last_time = time;
            *last_dx += dx;
            *last_dy += dy;
            return true;
        }
    }
    if queue.len() >= 128 {
        return false;
    }
    queue.push_back(event);
    true
}
#[cfg(test)]
mod direct_tests {
    use super::*;
    #[test]
    fn first_key_and_release_survive_high_rate_motion_before_ack() {
        let mut queue = VecDeque::new();
        let down = Event::Keyboard(KeyboardEvent::Key {
            time: 0,
            key: 30,
            state: 1,
        });
        let up = Event::Keyboard(KeyboardEvent::Key {
            time: 1,
            key: 30,
            state: 0,
        });
        assert!(queue_direct(&mut queue, down));
        for _ in 0..1000 {
            assert!(queue_direct(
                &mut queue,
                Event::Pointer(input_event::PointerEvent::Motion {
                    time: 0,
                    dx: 1.,
                    dy: 2.
                })
            ));
        }
        assert!(queue_direct(&mut queue, up));
        assert_eq!(queue.len(), 3);
        assert_eq!(queue.pop_front(), Some(down));
        assert_eq!(
            queue.pop_front(),
            Some(Event::Pointer(input_event::PointerEvent::Motion {
                time: 0,
                dx: 1000.,
                dy: 2000.
            }))
        );
        assert_eq!(queue.pop_front(), Some(up));
    }
    #[test]
    fn stalled_peer_has_a_bounded_transition_queue() {
        let mut queue = VecDeque::new();
        for i in 0..128 {
            assert!(queue_direct(
                &mut queue,
                Event::Keyboard(KeyboardEvent::Key {
                    time: i,
                    key: 30,
                    state: (i % 2) as u8
                })
            ));
        }
        assert!(!queue_direct(
            &mut queue,
            Event::Keyboard(KeyboardEvent::Key {
                time: 129,
                key: 30,
                state: 0
            })
        ));
        assert_eq!(queue.len(), 128);
    }
}
