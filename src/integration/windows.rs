//! Windows display/foreground metadata, without keyboard hooks or screen capture.
//!
//! `probe` is nonblocking with respect to DDC. One process-lifetime worker reads
//! the explicitly selected monitor every 3 s (10 s in fullscreen) and on entering fullscreen. The
//! successful sample's age keeps increasing across read failures.
use std::{
    ffi::c_void,
    mem::size_of,
    ptr::{null, null_mut},
    sync::{
        Condvar, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use super::PlatformState;

type Handle = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
struct MonitorInfo {
    size: u32,
    monitor: Rect,
    work: Rect,
    flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PhysicalMonitor {
    handle: Handle,
    description: [u16; 128],
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

// GUID_SESSION_DISPLAY_STATUS is for interactive user-mode applications.
const SESSION_DISPLAY_STATUS: Guid = Guid {
    data1: 0x2b84c20e,
    data2: 0xad23,
    data3: 0x4ddf,
    data4: [0x93, 0xdb, 0x05, 0xff, 0xbd, 0x7e, 0xfc, 0xa5],
};

#[repr(C)]
struct PowerSubscription {
    callback: unsafe extern "system" fn(Handle, u32, Handle) -> u32,
    // A pointer-sized null context. usize keeps the static owner Send + Sync.
    context: usize,
}

#[repr(C)]
struct PowerSetting {
    guid: Guid,
    data_length: u32,
    value: u32,
}

#[allow(non_snake_case)]
mod ffi {
    use super::{Guid, Handle, MonitorInfo, PhysicalMonitor, Rect, c_void};

    #[link(name = "user32", kind = "raw-dylib")]
    unsafe extern "system" {
        pub fn SetProcessDpiAwarenessContext(context: Handle) -> i32;
        pub fn SetThreadDpiAwarenessContext(context: Handle) -> Handle;
        pub fn GetForegroundWindow() -> Handle;
        pub fn GetAncestor(window: Handle, flags: u32) -> Handle;
        pub fn GetDesktopWindow() -> Handle;
        pub fn GetShellWindow() -> Handle;
        pub fn IsIconic(window: Handle) -> i32;
        pub fn IsWindowVisible(window: Handle) -> i32;
        pub fn GetClassNameW(window: Handle, class: *mut u16, maximum: i32) -> i32;
        pub fn GetWindowThreadProcessId(window: Handle, pid: *mut u32) -> u32;
        pub fn GetWindowRect(window: Handle, rect: *mut Rect) -> i32;
        #[cfg(target_pointer_width = "64")]
        pub fn GetWindowLongPtrW(window: Handle, index: i32) -> isize;
        #[cfg(target_pointer_width = "32")]
        pub fn GetWindowLongW(window: Handle, index: i32) -> i32;
        pub fn MonitorFromWindow(window: Handle, flags: u32) -> Handle;
        pub fn GetMonitorInfoW(monitor: Handle, info: *mut MonitorInfo) -> i32;
        pub fn EnumDisplayMonitors(
            dc: Handle,
            clip: *const Rect,
            callback: Option<unsafe extern "system" fn(Handle, Handle, *mut Rect, isize) -> i32>,
            data: isize,
        ) -> i32;
    }

    #[link(name = "dwmapi", kind = "raw-dylib")]
    unsafe extern "system" {
        pub fn DwmGetWindowAttribute(
            window: Handle,
            attribute: u32,
            value: *mut c_void,
            size: u32,
        ) -> i32;
    }

    #[link(name = "dxva2", kind = "raw-dylib")]
    unsafe extern "system" {
        pub fn GetNumberOfPhysicalMonitorsFromHMONITOR(monitor: Handle, count: *mut u32) -> i32;
        pub fn GetPhysicalMonitorsFromHMONITOR(
            monitor: Handle,
            count: u32,
            physical: *mut PhysicalMonitor,
        ) -> i32;
        pub fn DestroyPhysicalMonitors(count: u32, physical: *const PhysicalMonitor) -> i32;
        pub fn GetVCPFeatureAndVCPFeatureReply(
            monitor: Handle,
            code: u8,
            kind: *mut u32,
            current: *mut u32,
            maximum: *mut u32,
        ) -> i32;
    }

    #[link(name = "powrprof", kind = "raw-dylib")]
    unsafe extern "system" {
        pub fn PowerSettingRegisterNotification(
            guid: *const Guid,
            flags: u32,
            recipient: Handle,
            registration: *mut Handle,
        ) -> u32;
    }
}

#[derive(Default)]
struct Cache {
    present: bool,
    input: Option<u32>,
    sampled_at: Option<Instant>,
    fullscreen: bool,
    asleep: bool,
    refresh: bool,
}

struct Runtime {
    cache: Mutex<Cache>,
    wake: Condvar,
    // Keep callback parameters and subscription alive for the process lifetime.
    _power_subscription: Box<PowerSubscription>,
    _power_registration: usize,
}

static RUNTIME: OnceLock<Runtime> = OnceLock::new();
static STARTED: OnceLock<()> = OnceLock::new();
static DISPLAY_OFF: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn power_changed(_context: Handle, kind: u32, setting: Handle) -> u32 {
    if kind == 0x8013 && !setting.is_null() {
        // Windows owns this callback buffer; Data is a DWORD for this GUID.
        let data = &*setting.cast::<PowerSetting>();
        if data.guid == SESSION_DISPLAY_STATUS && data.data_length >= 4 {
            DISPLAY_OFF.store(data.value == 0, Ordering::Relaxed);
            if let Some(runtime) = RUNTIME.get() {
                runtime.wake.notify_one();
            }
        }
    }
    0
}

/// Start once, before creating any user interface, when possible.
pub fn start_monitor_probe() {
    STARTED.get_or_init(|| {
        unsafe {
            // Per-monitor aware V2 avoids virtualized window/monitor coordinates.
            // If the host already chose its DPI awareness, thread guards below
            // still make metadata queries use physical coordinates.
            ffi::SetProcessDpiAwarenessContext((-4isize) as Handle);
        }
        RUNTIME.get_or_init(|| {
            let mut subscription = Box::new(PowerSubscription {
                callback: power_changed,
                context: 0,
            });
            let mut registration = null_mut();
            unsafe {
                let _ = ffi::PowerSettingRegisterNotification(
                    &SESSION_DISPLAY_STATUS,
                    2, // DEVICE_NOTIFY_CALLBACK
                    (&mut *subscription as *mut PowerSubscription).cast(),
                    &mut registration,
                );
            }
            Runtime {
                cache: Mutex::new(Cache::default()),
                wake: Condvar::new(),
                _power_subscription: subscription,
                _power_registration: registration as usize,
            }
        });
        let _ = thread::Builder::new()
            .name("ponte-desk-ddc".into())
            .spawn(monitor_worker);
    });
}

/// Fast foreground snapshot. DDC never runs on this calling thread.
pub fn probe() -> PlatformState {
    start_monitor_probe();
    let fullscreen = foreground_is_fullscreen();
    let asleep = DISPLAY_OFF.load(Ordering::Relaxed);
    let runtime = RUNTIME.get().expect("initialized runtime");
    let mut cache = runtime.cache.lock().unwrap_or_else(|e| e.into_inner());
    if cache.fullscreen != fullscreen || cache.asleep != asleep {
        cache.fullscreen = fullscreen;
        cache.asleep = asleep;
        cache.refresh = true;
        runtime.wake.notify_one();
    }
    PlatformState {
        monitor_present: cache.present,
        fullscreen,
        input: cache.input,
        input_age_ms: cache
            .sampled_at
            .map(|at| u64::try_from(at.elapsed().as_millis()).unwrap_or(u64::MAX)),
        asleep,
    }
}

pub fn request_refresh() {
    if let Some(runtime) = RUNTIME.get() {
        runtime
            .cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .refresh = true;
        runtime.wake.notify_one();
    }
}

struct PhysicalDpi(Handle);
impl PhysicalDpi {
    fn enter() -> Self {
        Self(unsafe { ffi::SetThreadDpiAwarenessContext((-4isize) as Handle) })
    }
}
impl Drop for PhysicalDpi {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { ffi::SetThreadDpiAwarenessContext(self.0) };
        }
    }
}

fn covers_monitor(window: Rect, monitor: Rect, captioned: bool) -> bool {
    !captioned
        && window.left <= monitor.left + 2
        && window.top <= monitor.top + 2
        && window.right >= monitor.right - 2
        && window.bottom >= monitor.bottom - 2
        && monitor.right > monitor.left
        && monitor.bottom > monitor.top
}

fn foreground_is_fullscreen() -> bool {
    let _dpi = PhysicalDpi::enter();
    unsafe {
        let foreground = ffi::GetForegroundWindow();
        if foreground.is_null() {
            // Secure desktop/focus transition: never re-enable sharing blindly.
            return true;
        }
        let root = ffi::GetAncestor(foreground, 2); // GA_ROOT
        let window = if root.is_null() { foreground } else { root };
        if window == ffi::GetDesktopWindow()
            || window == ffi::GetShellWindow()
            || ffi::IsIconic(window) != 0
            || ffi::IsWindowVisible(window) == 0
        {
            return false;
        }
        let mut class = [0u16; 256];
        let length = ffi::GetClassNameW(window, class.as_mut_ptr(), class.len() as i32);
        let class = String::from_utf16_lossy(&class[..length.max(0) as usize]);
        if matches!(
            class.as_str(),
            "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
        ) {
            return false;
        }
        let mut process = 0;
        ffi::GetWindowThreadProcessId(window, &mut process);
        if process == std::process::id() {
            return false;
        }
        let mut cloaked = 0u32;
        if ffi::DwmGetWindowAttribute(window, 14, (&mut cloaked as *mut u32).cast(), 4) >= 0
            && cloaked != 0
        {
            return false;
        }
        let mut info = MonitorInfo {
            size: size_of::<MonitorInfo>() as u32,
            monitor: Rect::default(),
            work: Rect::default(),
            flags: 0,
        };
        let monitor = ffi::MonitorFromWindow(window, 2); // MONITOR_DEFAULTTONEAREST
        if monitor.is_null() || ffi::GetMonitorInfoW(monitor, &mut info) == 0 {
            return true;
        }
        let mut rect = Rect::default();
        if ffi::DwmGetWindowAttribute(
            window,
            9, // DWMWA_EXTENDED_FRAME_BOUNDS: physical, visible frame bounds.
            (&mut rect as *mut Rect).cast(),
            size_of::<Rect>() as u32,
        ) < 0
            && ffi::GetWindowRect(window, &mut rect) == 0
        {
            return true;
        }
        #[cfg(target_pointer_width = "64")]
        let style = ffi::GetWindowLongPtrW(window, -16) as u32;
        #[cfg(target_pointer_width = "32")]
        let style = ffi::GetWindowLongW(window, -16) as u32;
        covers_monitor(rect, info.monitor, style & 0x00c0_0000 != 0) // WS_CAPTION
    }
}

#[derive(Default)]
struct DdcSample {
    present: bool,
    input: Option<u32>,
}

struct PhysicalHandles(Vec<PhysicalMonitor>);
impl Drop for PhysicalHandles {
    fn drop(&mut self) {
        unsafe {
            ffi::DestroyPhysicalMonitors(self.0.len() as u32, self.0.as_ptr());
        }
    }
}

#[repr(C)]
struct MonitorInfoExtended {
    base: MonitorInfo,
    device: [u16; 32],
}
struct MonitorScan {
    sample: DdcSample,
    target: String,
    catalog: Vec<serde_json::Value>,
    listing: bool,
}
unsafe extern "system" fn visit_monitor(
    monitor: Handle,
    _dc: Handle,
    _rect: *mut Rect,
    data: isize,
) -> i32 {
    let scan = &mut *(data as *mut MonitorScan);
    let mut info = MonitorInfoExtended {
        base: MonitorInfo {
            size: size_of::<MonitorInfoExtended>() as u32,
            monitor: Rect::default(),
            work: Rect::default(),
            flags: 0,
        },
        device: [0; 32],
    };
    if ffi::GetMonitorInfoW(monitor, &mut info.base) == 0 {
        return 1;
    }
    let length = info.device.iter().position(|&c| c == 0).unwrap_or(32);
    let device = String::from_utf16_lossy(&info.device[..length]);
    let mut count = 0;
    if ffi::GetNumberOfPhysicalMonitorsFromHMONITOR(monitor, &mut count) == 0
        || count == 0
        || count > 64
    {
        return 1;
    }
    let mut monitors = vec![
        PhysicalMonitor {
            handle: null_mut(),
            description: [0; 128]
        };
        count as usize
    ];
    if ffi::GetPhysicalMonitorsFromHMONITOR(monitor, count, monitors.as_mut_ptr()) == 0 {
        return 1;
    }
    let monitors = PhysicalHandles(monitors);
    for (index, physical) in monitors.0.iter().enumerate() {
        let length = physical
            .description
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(128);
        let name = String::from_utf16_lossy(&physical.description[..length]);
        let id = format!("{device}#{index}#{name}");
        if !scan.listing && id != scan.target {
            continue;
        }
        let (mut kind, mut current, mut maximum) = (0, 0, 0);
        let input = if ffi::GetVCPFeatureAndVCPFeatureReply(
            physical.handle,
            0x60,
            &mut kind,
            &mut current,
            &mut maximum,
        ) != 0
        {
            Some(current & 255)
        } else {
            None
        };
        if scan.listing && scan.catalog.len() < 16 {
            scan.catalog
                .push(serde_json::json!({"id":id,"name":name,"input":input}));
        }
        if id == scan.target {
            scan.sample.present = true;
            scan.sample.input = input;
        }
    }
    1
}
fn scan_monitors(target: String, listing: bool) -> MonitorScan {
    let _dpi = PhysicalDpi::enter();
    let mut scan = MonitorScan {
        sample: DdcSample::default(),
        target,
        catalog: Vec::new(),
        listing,
    };
    unsafe {
        ffi::EnumDisplayMonitors(
            null_mut(),
            null(),
            Some(visit_monitor),
            (&mut scan as *mut MonitorScan) as isize,
        );
    }
    scan
}
pub fn monitor_catalog() -> serde_json::Value {
    serde_json::json!({"monitors":scan_monitors(String::new(),true).catalog})
}
fn read_monitor_input() -> DdcSample {
    let options = crate::preferences::Preferences::load(&super::native::root());
    if options.input_mode != "edges"
        || !options.shared_monitor
        || options.windows_monitor.is_empty()
    {
        return DdcSample::default();
    }
    scan_monitors(options.windows_monitor, false).sample
}

fn monitor_worker() {
    let runtime = RUNTIME.get().expect("initialized runtime");
    let mut next_read = Instant::now();
    loop {
        let mut cache = runtime.cache.lock().unwrap_or_else(|e| e.into_inner());
        if cache.refresh || Instant::now() >= next_read {
            cache.refresh = false;
            drop(cache);
            // A DDC driver may block. It can never block input or the core loop.
            let sample = read_monitor_input();
            cache = runtime.cache.lock().unwrap_or_else(|e| e.into_inner());
            cache.present = sample.present;
            if let Some(value) = sample.input {
                cache.input = Some(value);
                cache.sampled_at = Some(Instant::now());
            }
            next_read = Instant::now()
                + Duration::from_secs(if cache.fullscreen || DISPLAY_OFF.load(Ordering::Relaxed) {
                    10
                } else {
                    3
                });
        }
        let timeout = next_read.saturating_duration_since(Instant::now());
        drop(
            runtime
                .wake
                .wait_timeout(cache, timeout)
                .unwrap_or_else(|e| e.into_inner()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn monitor() -> Rect {
        Rect {
            left: 0,
            top: 0,
            right: 2560,
            bottom: 1440,
        }
    }
    #[test]
    fn borderless_fullscreen_and_dwm_border_overflow_are_protected() {
        assert!(covers_monitor(monitor(), monitor(), false));
        assert!(covers_monitor(
            Rect {
                left: -8,
                top: -8,
                right: 2568,
                bottom: 1448
            },
            monitor(),
            false
        ));
    }
    #[test]
    fn normal_window_or_maximized_caption_is_not_fullscreen() {
        assert!(!covers_monitor(monitor(), monitor(), true));
        assert!(!covers_monitor(
            Rect {
                bottom: 1392,
                ..monitor()
            },
            monitor(),
            false
        ));
    }
}
