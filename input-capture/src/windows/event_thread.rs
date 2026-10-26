use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::ptr::addr_of_mut;

use std::default::Default;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::thread;
use tokio::sync::mpsc::Sender;
use tokio::sync::mpsc::error::TrySendError;
use windows::Win32::Foundation::{FALSE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    DEVMODEW, DISPLAY_DEVICE_ATTACHED_TO_DESKTOP, DISPLAY_DEVICEW, ENUM_CURRENT_SETTINGS,
    EnumDisplayDevicesW, EnumDisplaySettingsW,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::core::{PCWSTR, w};

use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DestroyWindow, DispatchMessageW,
    EDD_GET_DEVICE_INTERFACE_NAME, GetCursorPos, GetMessageW, HHOOK, KBDLLHOOKSTRUCT,
    LLKHF_EXTENDED, MSG, MSLLHOOKSTRUCT, PM_NOREMOVE, PeekMessageW, PostThreadMessageW,
    RegisterClassW, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL,
    WH_MOUSE_LL, WINDOW_STYLE, WM_DISPLAYCHANGE, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_USER, WM_XBUTTONDOWN,
    WM_XBUTTONUP, WNDCLASSW,
};

use input_event::{
    BTN_BACK, BTN_FORWARD, BTN_LEFT, BTN_MIDDLE, BTN_RIGHT, Event, KeyboardEvent, PointerEvent,
    scancode::{self, Linux},
};

use super::{CaptureEvent, Position, display_util};
use crate::CaptureCreationError;

pub(crate) struct EventThread {
    request_buffer: Arc<Mutex<Vec<ClientUpdate>>>,
    thread: Option<thread::JoinHandle<()>>,
    thread_id: u32,
    stopping: Arc<AtomicBool>,
    release_pending: Arc<AtomicBool>,
}

impl EventThread {
    pub(crate) fn new(
        event_tx: Sender<(Position, CaptureEvent)>,
    ) -> Result<Self, CaptureCreationError> {
        let request_buffer = Default::default();
        let stopping = Arc::new(AtomicBool::new(false));
        let release_pending = Arc::new(AtomicBool::new(false));
        let (thread, thread_id) = start(
            event_tx,
            Arc::clone(&request_buffer),
            Arc::clone(&stopping),
            Arc::clone(&release_pending),
        )?;
        Ok(Self {
            request_buffer,
            thread: Some(thread),
            thread_id,
            stopping,
            release_pending,
        })
    }

    pub(crate) fn release_capture(&self) {
        self.release_pending.store(true, Ordering::Release);
        self.signal(RequestType::Release);
    }

    pub(crate) fn create(&self, pos: Position) {
        self.client_update(ClientUpdate::Create(pos));
    }

    pub(crate) fn destroy(&self, pos: Position) {
        self.client_update(ClientUpdate::Destroy(pos));
    }

    pub(crate) fn direct(&self, pos: Option<Position>) {
        self.client_update(ClientUpdate::Direct(pos));
    }

    fn exit(&self) {
        self.stopping.store(true, Ordering::Release);
        self.signal(RequestType::Exit);
    }

    fn client_update(&self, request: ClientUpdate) {
        {
            let mut requests = self
                .request_buffer
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            requests.push(request);
        }
        self.signal(RequestType::ClientUpdate);
    }

    fn signal(&self, event_type: RequestType) {
        let id = self.thread_id;
        if id != 0 {
            // A dead thread or a full queue must never panic during teardown.
            // Sticky stop/release flags are checked before every queued message.
            if let Err(e) =
                unsafe { PostThreadMessageW(id, WM_USER, WPARAM(event_type as usize), LPARAM(0)) }
            {
                log::debug!("capture thread notification failed: {e}");
            }
        }
    }
}

impl Drop for EventThread {
    fn drop(&mut self) {
        self.exit();
        if let Some(thread) = self.thread.take() {
            if thread.join().is_err() {
                log::error!("Windows capture thread exited unexpectedly");
            }
        }
    }
}

enum RequestType {
    ClientUpdate = 0,
    Release = 1,
    Exit = 2,
}

enum ClientUpdate {
    Direct(Option<Position>),
    Create(Position),
    Destroy(Position),
}

fn try_send_event(
    pos: Position,
    event: CaptureEvent,
) -> Result<(), TrySendError<(Position, CaptureEvent)>> {
    EVENT_TX.with_borrow(|tx| match tx {
        Some(tx) => {
            // Bound motion backlog while retaining room for Begin, key-up and
            // button transitions. Never block a native hook callback.
            if tx.capacity() <= 16
                && matches!(
                    event,
                    CaptureEvent::Input(Event::Pointer(PointerEvent::Motion { .. }))
                )
            {
                Err(TrySendError::Full((pos, event)))
            } else {
                tx.try_send((pos, event))
            }
        }
        None => Err(TrySendError::Closed((pos, event))),
    })
}

thread_local! {
    /// all configured clients
    static CLIENTS: RefCell<HashSet<Position>> = RefCell::new(HashSet::new());
    /// currently active client
    static ACTIVE_CLIENT: Cell<Option<Position>> = const { Cell::new(None) };
    /// input event channel
    static EVENT_TX: RefCell<Option<Sender<(Position, CaptureEvent)>>> = const { RefCell::new(None) };
    /// position of barrier entry
    static ENTRY_POINT: Cell<(i32, i32)> = const { Cell::new((0, 0)) };
    /// previous mouse position
    static PREV_POS: Cell<Option<(i32, i32)>> = const { Cell::new(None) };
    /// displays and generation counter
    static DISPLAYS: RefCell<(Vec<RECT>, i32)> = const { RefCell::new((Vec::new(), 0)) };
}

fn get_msg() -> Option<MSG> {
    unsafe {
        let mut msg = std::mem::zeroed();
        let ret = GetMessageW(addr_of_mut!(msg), None, 0, 0);
        match ret.0 {
            0 => None,
            x if x > 0 => Some(msg),
            _ => {
                log::error!(
                    "Windows capture GetMessageW failed: {}",
                    windows::core::Error::from_win32()
                );
                None
            }
        }
    }
}

/// All Win32 resources are owned by the message thread and released there,
/// including partial initialization and a closed/full event consumer.
#[derive(Default)]
struct ThreadResources {
    mouse: Option<HHOOK>,
    keyboard: Option<HHOOK>,
    window: Option<HWND>,
}
impl Drop for ThreadResources {
    fn drop(&mut self) {
        ACTIVE_CLIENT.set(None);
        PREV_POS.set(None);
        CLIENTS.with_borrow_mut(HashSet::clear);
        unsafe {
            if let Some(hook) = self.mouse.take() {
                let _ = UnhookWindowsHookEx(hook);
            }
            if let Some(hook) = self.keyboard.take() {
                let _ = UnhookWindowsHookEx(hook);
            }
            if let Some(window) = self.window.take() {
                let _ = DestroyWindow(window);
            }
        }
        EVENT_TX.take();
    }
}

fn initialize() -> Result<ThreadResources, String> {
    // PeekMessage creates the queue even when there are no pending messages.
    unsafe {
        let _ = PeekMessageW(&mut MSG::default(), None, 0, 0, PM_NOREMOVE);
    }
    let instance = unsafe { GetModuleHandleW(None) }
        .map_err(|e| e.to_string())?
        .into();
    static WINDOW_CLASS: OnceLock<Result<(), String>> = OnceLock::new();
    WINDOW_CLASS
        .get_or_init(|| {
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                lpszClassName: w!("lan-mouse-message-window-class"),
                ..Default::default()
            };
            if unsafe { RegisterClassW(&class) } == 0 {
                let error = windows::core::Error::from_win32();
                // The process-wide class remains valid between capture lifetimes.
                if error.code().0 as u32 != 0x80070582 {
                    return Err(error.to_string());
                }
            }
            Ok(())
        })
        .clone()?;
    let mut resources = ThreadResources {
        mouse: None,
        keyboard: None,
        window: Some(
            unsafe {
                CreateWindowExW(
                    Default::default(),
                    w!("lan-mouse-message-window-class"),
                    w!("lan-mouse-msg-window"),
                    WINDOW_STYLE::default(),
                    0,
                    0,
                    0,
                    0,
                    None,
                    None,
                    Some(instance),
                    None,
                )
            }
            .map_err(|e| e.to_string())?,
        ),
    };
    resources.mouse = Some(
        unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0) }
            .map_err(|e| e.to_string())?,
    );
    resources.keyboard = Some(
        unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(kybrd_proc), None, 0) }
            .map_err(|e| e.to_string())?,
    );
    Ok(resources)
}

fn start(
    event_tx: Sender<(Position, CaptureEvent)>,
    request_buffer: Arc<Mutex<Vec<ClientUpdate>>>,
    stopping: Arc<AtomicBool>,
    release_pending: Arc<AtomicBool>,
) -> Result<(thread::JoinHandle<()>, u32), CaptureCreationError> {
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let msg_thread = thread::Builder::new()
        .name("windows-input-capture".into())
        .spawn(move || {
            start_routine(
                ready_tx,
                event_tx,
                request_buffer,
                stopping,
                release_pending,
            )
        })
        .map_err(|error| {
            log::error!("Windows capture thread creation failed: {error}");
            CaptureCreationError::Windows
        })?;
    // EOF also wakes the caller if initialization fails; it cannot wait forever.
    match ready_rx.recv() {
        Ok(id) => Ok((msg_thread, id)),
        Err(_) => {
            let _ = msg_thread.join();
            Err(CaptureCreationError::Windows)
        }
    }
}

fn start_routine(
    ready: mpsc::SyncSender<u32>,
    event_tx: Sender<(Position, CaptureEvent)>,
    request_buffer: Arc<Mutex<Vec<ClientUpdate>>>,
    stopping: Arc<AtomicBool>,
    release_pending: Arc<AtomicBool>,
) {
    EVENT_TX.replace(Some(event_tx));
    let _resources = match initialize() {
        Ok(resources) => resources,
        Err(error) => {
            EVENT_TX.take();
            log::error!("Windows capture initialization failed: {error}");
            return;
        }
    };
    // READY means queue, window and both hooks are completely initialized.
    if ready.send(unsafe { GetCurrentThreadId() }).is_err() {
        return;
    }
    while !stopping.load(Ordering::Acquire) {
        let Some(msg) = get_msg() else {
            break;
        };
        if stopping.load(Ordering::Acquire) {
            break;
        }
        if release_pending.swap(false, Ordering::AcqRel) {
            ACTIVE_CLIENT.set(None);
            PREV_POS.set(None);
        }
        if msg.hwnd.0.is_null() && msg.message == WM_USER {
            match msg.wParam.0 {
                x if x == RequestType::Exit as usize => break,
                x if x == RequestType::Release as usize => {}
                x if x == RequestType::ClientUpdate as usize => {
                    let requests = std::mem::take(
                        &mut *request_buffer.lock().unwrap_or_else(|e| e.into_inner()),
                    );
                    for request in requests {
                        update_clients(request);
                    }
                }
                _ => {}
            }
        } else {
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

fn check_client_activation(wparam: WPARAM, lparam: LPARAM) -> bool {
    if activate_direct() {
        return true;
    }
    if wparam.0 != WM_MOUSEMOVE as usize {
        return ACTIVE_CLIENT.get().is_some();
    }
    let mouse_low_level: MSLLHOOKSTRUCT = unsafe { *(lparam.0 as *const MSLLHOOKSTRUCT) };
    let curr_pos = (mouse_low_level.pt.x, mouse_low_level.pt.y);
    let prev_pos = PREV_POS.get().unwrap_or(curr_pos);
    PREV_POS.replace(Some(curr_pos));

    /* next event is the first actual event */
    let ret = ACTIVE_CLIENT.get().is_some();

    /* client already active, no need to check */
    if ACTIVE_CLIENT.get().is_some() {
        return ret;
    }

    /* check if a client was activated */
    let entered = DISPLAYS.with_borrow_mut(|(displays, generation)| {
        update_display_regions(displays, generation);
        display_util::entered_barrier(prev_pos, curr_pos, displays)
    });

    let Some(pos) = entered else {
        return ret;
    };

    /* check if a client is registered for the barrier */
    if !CLIENTS.with_borrow(|clients| clients.contains(&pos)) {
        return ret;
    }

    /* update active client and entry point */
    ACTIVE_CLIENT.replace(Some(pos));
    let entry_point = DISPLAYS.with_borrow(|(displays, _)| {
        display_util::clamp_to_display_bounds(displays, prev_pos, curr_pos)
    });
    ENTRY_POINT.replace(entry_point);

    /* notify main thread */
    log::debug!("ENTERED @ {prev_pos:?} -> {curr_pos:?}");
    // A hook callback must never block waiting for the async consumer. A full
    // or closed channel leaves this physical event and subsequent input local.
    if try_send_event(pos, CaptureEvent::Begin).is_err() {
        ACTIVE_CLIENT.set(None);
        PREV_POS.set(None);
        return false;
    }

    ret
}

unsafe extern "system" fn mouse_proc(ncode: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if ncode < 0 {
        return CallNextHookEx(None, ncode, wparam, lparam);
    }
    let event = &*(lparam.0 as *const MSLLHOOKSTRUCT);
    if crate::input_origin::windows_mouse_is_software(event.flags)
        && event.dwExtraInfo != crate::input_origin::EMULATION_TAG
    {
        crate::input_origin::record_software_event();
        // Do not alter PREV_POS: an agent's warp must not create a physical
        // barrier crossing on the user's next event.
        PREV_POS.set(None);
        return CallNextHookEx(None, ncode, wparam, lparam);
    }
    if DIRECT_CLIENT.get().is_some() {
        let mut point = POINT::default();
        if GetCursorPos(&mut point).is_err() {
            DIRECT_CLIENT.set(None);
            ACTIVE_CLIENT.set(None);
            return CallNextHookEx(None, ncode, wparam, lparam);
        }
        ENTRY_POINT.set((point.x, point.y));
    }
    let active = check_client_activation(wparam, lparam);

    /* no client was active */
    if !active {
        return CallNextHookEx(None, ncode, wparam, lparam);
    }

    /* get active client if any */
    let Some(pos) = ACTIVE_CLIENT.get() else {
        return LRESULT(1);
    };

    /* convert to lan-mouse event */
    let Some(pointer_event) = to_mouse_event(wparam, lparam) else {
        return LRESULT(1);
    };

    /* notify mainthread (drop events if sending too fast) */
    if let Err(e) = try_send_event(pos, CaptureEvent::Input(Event::Pointer(pointer_event))) {
        if matches!(e, TrySendError::Closed(_)) {
            ACTIVE_CLIENT.set(None);
            PREV_POS.set(None);
            return CallNextHookEx(None, ncode, wparam, lparam);
        }
    }

    /* don't pass event to applications */
    LRESULT(1)
}

unsafe extern "system" fn kybrd_proc(ncode: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if ncode < 0 {
        return CallNextHookEx(None, ncode, wparam, lparam);
    }
    let event = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
    if crate::input_origin::windows_key_is_software(event.flags.0)
        && event.dwExtraInfo != crate::input_origin::EMULATION_TAG
    {
        crate::input_origin::record_software_event();
        return CallNextHookEx(None, ncode, wparam, lparam);
    }
    /* get active client if any */
    activate_direct();
    let Some(client) = ACTIVE_CLIENT.get() else {
        return CallNextHookEx(None, ncode, wparam, lparam);
    };

    /* convert to key event */
    let Some(key_event) = to_key_event(wparam, lparam) else {
        return LRESULT(1);
    };

    if let Err(e) = try_send_event(client, CaptureEvent::Input(Event::Keyboard(key_event))) {
        // Missing a keyboard transition must not trap further local input.
        ACTIVE_CLIENT.set(None);
        PREV_POS.set(None);
        log::debug!("capture keyboard consumer unavailable: {e}");
        return CallNextHookEx(None, ncode, wparam, lparam);
    }

    /* don't pass event to applications */
    LRESULT(1)
}

unsafe extern "system" fn window_proc(
    _hwnd: HWND,
    uint: u32,
    _wparam: WPARAM,
    _lparam: LPARAM,
) -> LRESULT {
    if uint == WM_DISPLAYCHANGE {
        log::debug!("display resolution changed");
        DISPLAY_RESOLUTION_GENERATION.fetch_add(1, Ordering::Release);
    }
    LRESULT(1)
}

static DISPLAY_RESOLUTION_GENERATION: AtomicI32 = AtomicI32::new(1);

fn update_display_regions(displays: &mut Vec<RECT>, generation: &mut i32) {
    let global_generation = DISPLAY_RESOLUTION_GENERATION.load(Ordering::Acquire);
    if *generation != global_generation {
        enumerate_displays(displays);
        log::debug!("displays: {displays:?}");
        *generation = global_generation;
    }
}

fn enumerate_displays(display_rects: &mut Vec<RECT>) {
    display_rects.clear();
    unsafe {
        let mut devices = vec![];
        for i in 0.. {
            let mut device: DISPLAY_DEVICEW = std::mem::zeroed();
            device.cb = std::mem::size_of::<DISPLAY_DEVICEW>() as u32;
            let ret = EnumDisplayDevicesW(None, i, &mut device, EDD_GET_DEVICE_INTERFACE_NAME);
            if ret == FALSE {
                break;
            }
            if device
                .StateFlags
                .contains(DISPLAY_DEVICE_ATTACHED_TO_DESKTOP)
            {
                devices.push(device.DeviceName);
            }
        }
        for device in devices {
            let mut dev_mode: DEVMODEW = std::mem::zeroed();
            dev_mode.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
            let ret = EnumDisplaySettingsW(
                PCWSTR::from_raw(&device as *const _),
                ENUM_CURRENT_SETTINGS,
                &mut dev_mode,
            );
            if ret == FALSE {
                log::warn!("no display mode");
            }

            let pos = dev_mode.Anonymous1.Anonymous2.dmPosition;
            let (x, y) = (pos.x, pos.y);
            let (width, height) = (dev_mode.dmPelsWidth, dev_mode.dmPelsHeight);

            display_rects.push(RECT {
                left: x,
                right: x + width as i32,
                top: y,
                bottom: y + height as i32,
            });
        }
    }
}

fn update_clients(request: ClientUpdate) {
    match request {
        ClientUpdate::Direct(pos) => {
            ACTIVE_CLIENT.set(None);
            PREV_POS.set(None);
            DIRECT_CLIENT.set(pos);
        }
        ClientUpdate::Create(pos) => {
            CLIENTS.with_borrow_mut(|clients| clients.insert(pos));
        }
        ClientUpdate::Destroy(pos) => {
            if let Some(active_pos) = ACTIVE_CLIENT.get() {
                if pos == active_pos {
                    let _ = ACTIVE_CLIENT.take();
                }
            }
            CLIENTS.with_borrow_mut(|clients| clients.remove(&pos));
        }
    }
}

thread_local! {static DIRECT_CLIENT:Cell<Option<Position>>=const{Cell::new(None)};}
fn activate_direct() -> bool {
    let Some(pos) = DIRECT_CLIENT.get() else {
        return false;
    };
    if ACTIVE_CLIENT.get().is_none() {
        if !CLIENTS.with_borrow(|clients| clients.contains(&pos))
            || try_send_event(pos, CaptureEvent::Begin).is_err()
        {
            DIRECT_CLIENT.set(None);
            return false;
        }
        ACTIVE_CLIENT.set(Some(pos));
    }
    true
}

fn to_key_event(wparam: WPARAM, lparam: LPARAM) -> Option<KeyboardEvent> {
    let kybrdllhookstruct: KBDLLHOOKSTRUCT = unsafe { *(lparam.0 as *const KBDLLHOOKSTRUCT) };
    let mut scan_code = kybrdllhookstruct.scanCode;
    log::trace!("scan_code: {scan_code}");
    if kybrdllhookstruct.flags.contains(LLKHF_EXTENDED) {
        scan_code |= 0xE000;
    }
    let Ok(win_scan_code) = scancode::Windows::try_from(scan_code) else {
        log::warn!("failed to translate to windows scancode: {scan_code}");
        return None;
    };
    log::trace!("windows_scan: {win_scan_code:?}");
    let Ok(linux_scan_code): Result<Linux, ()> = win_scan_code.try_into() else {
        log::warn!("failed to translate into linux scancode: {win_scan_code:?}");
        return None;
    };
    log::trace!("windows_scan: {linux_scan_code:?}");
    let scan_code = linux_scan_code as u32;
    match wparam {
        WPARAM(p) if p == WM_KEYDOWN as usize => Some(KeyboardEvent::Key {
            time: 0,
            key: scan_code,
            state: 1,
        }),
        WPARAM(p) if p == WM_KEYUP as usize => Some(KeyboardEvent::Key {
            time: 0,
            key: scan_code,
            state: 0,
        }),
        WPARAM(p) if p == WM_SYSKEYDOWN as usize => Some(KeyboardEvent::Key {
            time: 0,
            key: scan_code,
            state: 1,
        }),
        WPARAM(p) if p == WM_SYSKEYUP as usize => Some(KeyboardEvent::Key {
            time: 0,
            key: scan_code,
            state: 0,
        }),
        _ => None,
    }
}

fn to_mouse_event(wparam: WPARAM, lparam: LPARAM) -> Option<PointerEvent> {
    let mouse_low_level: MSLLHOOKSTRUCT = unsafe { *(lparam.0 as *const MSLLHOOKSTRUCT) };
    match wparam {
        WPARAM(p) if p == WM_LBUTTONDOWN as usize => Some(PointerEvent::Button {
            time: 0,
            button: BTN_LEFT,
            state: 1,
        }),
        WPARAM(p) if p == WM_MBUTTONDOWN as usize => Some(PointerEvent::Button {
            time: 0,
            button: BTN_MIDDLE,
            state: 1,
        }),
        WPARAM(p) if p == WM_RBUTTONDOWN as usize => Some(PointerEvent::Button {
            time: 0,
            button: BTN_RIGHT,
            state: 1,
        }),
        WPARAM(p) if p == WM_LBUTTONUP as usize => Some(PointerEvent::Button {
            time: 0,
            button: BTN_LEFT,
            state: 0,
        }),
        WPARAM(p) if p == WM_MBUTTONUP as usize => Some(PointerEvent::Button {
            time: 0,
            button: BTN_MIDDLE,
            state: 0,
        }),
        WPARAM(p) if p == WM_RBUTTONUP as usize => Some(PointerEvent::Button {
            time: 0,
            button: BTN_RIGHT,
            state: 0,
        }),
        WPARAM(p) if p == WM_MOUSEMOVE as usize => {
            let (x, y) = (mouse_low_level.pt.x, mouse_low_level.pt.y);
            let (ex, ey) = ENTRY_POINT.get();
            let (dx, dy) = (x - ex, y - ey);
            let (dx, dy) = (dx as f64, dy as f64);
            Some(PointerEvent::Motion { time: 0, dx, dy })
        }
        WPARAM(p) if p == WM_MOUSEWHEEL as usize => Some(PointerEvent::AxisDiscrete120 {
            axis: 0,
            value: -(mouse_low_level.mouseData as i32 >> 16),
        }),
        WPARAM(p) if p == WM_XBUTTONDOWN as usize || p == WM_XBUTTONUP as usize => {
            let hb = mouse_low_level.mouseData >> 16;
            let button = match hb {
                1 => BTN_BACK,
                2 => BTN_FORWARD,
                _ => {
                    log::warn!("unknown mouse button");
                    return None;
                }
            };
            Some(PointerEvent::Button {
                time: 0,
                button,
                state: if p == WM_XBUTTONDOWN as usize { 1 } else { 0 },
            })
        }
        WPARAM(p) if p == WM_MOUSEHWHEEL as usize => Some(PointerEvent::AxisDiscrete120 {
            axis: 1, // Horizontal
            value: mouse_low_level.mouseData as i32 >> 16,
        }),
        w => {
            log::warn!("unknown mouse event: {w:?}");
            None
        }
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    #[test]
    fn direct_route_starts_on_a_physical_key_and_ignores_injected_keys() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(128);
        EVENT_TX.replace(Some(tx));
        update_clients(ClientUpdate::Create(Position::Left));
        update_clients(ClientUpdate::Direct(Some(Position::Left)));
        let mut key = KBDLLHOOKSTRUCT {
            scanCode: 0x1e,
            ..Default::default()
        };
        key.flags = windows::Win32::UI::WindowsAndMessaging::KBDLLHOOKSTRUCT_FLAGS(0x10);
        unsafe {
            kybrd_proc(
                0,
                WPARAM(WM_KEYDOWN as usize),
                LPARAM((&key as *const KBDLLHOOKSTRUCT) as isize),
            );
        }
        assert!(rx.try_recv().is_err());
        assert!(ACTIVE_CLIENT.get().is_none());
        key.flags = Default::default();
        let result = unsafe {
            kybrd_proc(
                0,
                WPARAM(WM_KEYDOWN as usize),
                LPARAM((&key as *const KBDLLHOOKSTRUCT) as isize),
            )
        };
        assert_eq!(result, LRESULT(1));
        assert_eq!(
            rx.try_recv().unwrap(),
            (Position::Left, CaptureEvent::Begin)
        );
        assert!(matches!(
            rx.try_recv().unwrap().1,
            CaptureEvent::Input(Event::Keyboard(KeyboardEvent::Key { state: 1, .. }))
        ));
        update_clients(ClientUpdate::Direct(None));
        update_clients(ClientUpdate::Destroy(Position::Left));
        EVENT_TX.take();
    }
    #[test]
    fn motion_backlog_reserves_capacity_for_critical_transitions() {
        let (tx, _rx) = tokio::sync::mpsc::channel(128);
        EVENT_TX.replace(Some(tx));
        let motion = CaptureEvent::Input(Event::Pointer(PointerEvent::Motion {
            time: 0,
            dx: 1.0,
            dy: 0.0,
        }));
        for _ in 0..112 {
            assert!(try_send_event(Position::Left, motion).is_ok());
        }
        assert!(matches!(
            try_send_event(Position::Left, motion),
            Err(TrySendError::Full(_))
        ));
        for i in 0..16 {
            let critical = match i % 3 {
                0 => CaptureEvent::Begin,
                1 => CaptureEvent::Input(Event::Keyboard(KeyboardEvent::Key {
                    time: 0,
                    key: 30,
                    state: 0,
                })),
                _ => CaptureEvent::Input(Event::Pointer(PointerEvent::Button {
                    time: 0,
                    button: BTN_LEFT,
                    state: 0,
                })),
            };
            assert!(try_send_event(Position::Left, critical).is_ok());
        }
        EVENT_TX.take();
    }

    #[test]
    fn a_full_or_closed_consumer_never_blocks_the_hook_sender() {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        EVENT_TX.replace(Some(tx));
        assert!(try_send_event(Position::Left, CaptureEvent::Begin).is_ok());
        assert!(matches!(
            try_send_event(Position::Left, CaptureEvent::Begin),
            Err(TrySendError::Full(_))
        ));
        drop(rx);
        assert!(matches!(
            try_send_event(Position::Left, CaptureEvent::Begin),
            Err(TrySendError::Closed(_))
        ));
        EVENT_TX.take();
        assert!(matches!(
            try_send_event(Position::Left, CaptureEvent::Begin),
            Err(TrySendError::Closed(_))
        ));
    }
}
