//! Background Raw Input metadata. Never suppresses or rewrites input.
use input_event::activity;
use std::{
    ffi::c_void,
    ptr::{null, null_mut},
    sync::OnceLock,
};
type H = *mut c_void;
#[repr(C)]
struct Point {
    x: i32,
    y: i32,
}
#[repr(C)]
struct Message {
    hwnd: H,
    message: u32,
    w: usize,
    l: isize,
    time: u32,
    point: Point,
    private: u32,
}
#[repr(C)]
struct Class {
    style: u32,
    proc: Option<unsafe extern "system" fn(H, u32, usize, isize) -> isize>,
    class_extra: i32,
    window_extra: i32,
    instance: H,
    icon: H,
    cursor: H,
    background: H,
    menu: *const u16,
    name: *const u16,
}
#[repr(C)]
struct Device {
    page: u16,
    usage: u16,
    flags: u32,
    target: H,
}
#[repr(C)]
struct Header {
    kind: u32,
    size: u32,
    device: H,
    w: usize,
}
#[repr(C)]
struct Mouse {
    flags: u16,
    buttons: u32,
    raw_buttons: u32,
    x: i32,
    y: i32,
    extra: u32,
}
#[repr(C)]
struct Key {
    scan: u16,
    flags: u16,
    reserved: u16,
    vk: u16,
    message: u32,
    extra: u32,
}
#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegisterClassW(class: *const Class) -> u16;
    fn CreateWindowExW(
        ex: u32,
        class: *const u16,
        title: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: H,
        menu: H,
        instance: H,
        param: H,
    ) -> H;
    fn DefWindowProcW(hwnd: H, message: u32, w: usize, l: isize) -> isize;
    fn RegisterRawInputDevices(devices: *const Device, count: u32, size: u32) -> i32;
    fn GetRawInputData(raw: H, command: u32, data: H, size: *mut u32, header_size: u32) -> u32;
    fn GetMessageW(message: *mut Message, hwnd: H, min: u32, max: u32) -> i32;
    fn TranslateMessage(message: *const Message) -> i32;
    fn DispatchMessageW(message: *const Message) -> isize;
    fn DestroyWindow(hwnd: H) -> i32;
}
#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> H;
}
unsafe extern "system" fn window(hwnd: H, message: u32, w: usize, l: isize) -> isize {
    if message == 0xfe {
        activity::clear_held();
    }
    if message == 0xff && activity::enabled() {
        // Mouse/keyboard RAWINPUT have fixed bounded layouts. HID payloads are not
        // registered or decoded. The aligned stack buffer never enters a queue.
        let mut buffer = [0u64; 8];
        let mut size = std::mem::size_of_val(&buffer) as u32;
        let head = std::mem::size_of::<Header>() as u32;
        let read = GetRawInputData(
            l as H,
            0x10000003,
            buffer.as_mut_ptr().cast(),
            &mut size,
            head,
        );
        if read != u32::MAX && read >= head && read <= std::mem::size_of_val(&buffer) as u32 {
            let header = &*(buffer.as_ptr() as *const Header);
            if !header.device.is_null() {
                let data = buffer.as_ptr().cast::<u8>().add(head as usize);
                if header.kind == 0 && read >= head + std::mem::size_of::<Mouse>() as u32 {
                    let mouse = &*(data as *const Mouse);
                    let buttons = mouse.buttons & 0xffff;
                    for (index, down, up) in [
                        (0, 1, 2),
                        (1, 4, 8),
                        (2, 0x10, 0x20),
                        (3, 0x40, 0x80),
                        (4, 0x100, 0x200),
                    ] {
                        if buttons & down != 0 {
                            activity::physical_key(256 + index, true);
                        }
                        if buttons & up != 0 {
                            activity::physical_key(256 + index, false);
                        }
                    }
                    if mouse.x != 0 || mouse.y != 0 || buttons & 0xc00 != 0 {
                        activity::physical_motion();
                    }
                } else if header.kind == 1 && read >= head + std::mem::size_of::<Key>() as u32 {
                    let key = &*(data as *const Key);
                    if key.vk < 255 {
                        let vk = match key.vk {
                            16 => {
                                if key.scan == 42 {
                                    160
                                } else {
                                    161
                                }
                            }
                            17 => {
                                if key.flags & 2 != 0 {
                                    163
                                } else {
                                    162
                                }
                            }
                            18 => {
                                if key.flags & 2 != 0 {
                                    165
                                } else {
                                    164
                                }
                            }
                            other => other,
                        };
                        activity::physical_key(vk, key.flags & 1 == 0);
                    }
                }
            }
        }
    }
    DefWindowProcW(hwnd, message, w, l)
}
pub(super) fn start() {
    static START: OnceLock<()> = OnceLock::new();
    START.get_or_init(|| {
        let _ = std::thread::Builder::new()
            .name("human-activity".into())
            .spawn(|| unsafe {
                let instance = GetModuleHandleW(null());
                let name: Vec<u16> = "PonteDeskPhysicalActivity\0".encode_utf16().collect();
                let class = Class {
                    style: 0,
                    proc: Some(window),
                    class_extra: 0,
                    window_extra: 0,
                    instance,
                    icon: null_mut(),
                    cursor: null_mut(),
                    background: null_mut(),
                    menu: null(),
                    name: name.as_ptr(),
                };
                if RegisterClassW(&class) == 0 {
                    log::warn!("automatic activity class unavailable");
                    return;
                }
                let hwnd = CreateWindowExW(
                    0,
                    name.as_ptr(),
                    name.as_ptr(),
                    0,
                    0,
                    0,
                    0,
                    0,
                    -3isize as H,
                    null_mut(),
                    instance,
                    null_mut(),
                );
                if hwnd.is_null() {
                    log::warn!("automatic activity window unavailable");
                    return;
                }
                let devices = [
                    Device {
                        page: 1,
                        usage: 2,
                        flags: 0x2100,
                        target: hwnd,
                    },
                    Device {
                        page: 1,
                        usage: 6,
                        flags: 0x2100,
                        target: hwnd,
                    },
                ];
                if RegisterRawInputDevices(
                    devices.as_ptr(),
                    2,
                    std::mem::size_of::<Device>() as u32,
                ) == 0
                {
                    DestroyWindow(hwnd);
                    log::warn!("automatic Raw Input observer unavailable");
                    return;
                }
                activity::ready(true);
                log::info!("automatic physical activity observer active (Raw Input)");
                let mut message: Message = std::mem::zeroed();
                while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                activity::ready(false);
                DestroyWindow(hwnd);
            });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_input_layouts_match_win64_headers() {
        assert_eq!(std::mem::size_of::<Header>(), 24);
        assert_eq!(std::mem::size_of::<Mouse>(), 24);
        assert_eq!(std::mem::size_of::<Key>(), 16);
        assert_eq!(std::mem::size_of::<Device>(), 16);
        assert_eq!(std::mem::size_of::<Class>(), 72);
    }
}
