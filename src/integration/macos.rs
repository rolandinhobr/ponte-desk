use super::native::PlatformState;
use std::sync::OnceLock;
fn entry() -> Option<unsafe extern "C" fn() -> u32> {
    static ENTRY: OnceLock<Option<unsafe extern "C" fn() -> u32>> = OnceLock::new();
    *ENTRY.get_or_init(|| unsafe {
        let path = std::env::current_exe()
            .ok()?
            .parent()?
            .join("../Frameworks/PonteDeskUI.dylib");
        let path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).ok()?;
        let library = libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        if library.is_null() {
            return None;
        }
        let symbol = libc::dlsym(library, c"ponte_platform_probe".as_ptr());
        if symbol.is_null() {
            return None;
        }
        Some(std::mem::transmute::<
            *mut libc::c_void,
            unsafe extern "C" fn() -> u32,
        >(symbol))
    })
}
pub fn probe() -> PlatformState {
    let bits = entry().map_or(0, |entry| unsafe { entry() });
    PlatformState {
        monitor_present: bits & 1 != 0,
        fullscreen: bits & 2 != 0,
        asleep: bits & 4 != 0,
        input: None,
        input_age_ms: None,
    }
}
