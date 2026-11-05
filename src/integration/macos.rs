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

pub fn monitor_catalog() -> serde_json::Value {
    unsafe {
        let load = || -> Option<serde_json::Value> {
            let path = std::env::current_exe()
                .ok()?
                .parent()?
                .join("../Frameworks/PonteDeskUI.dylib");
            let path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).ok()?;
            let lib = libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
            if lib.is_null() {
                return None;
            }
            let symbol = libc::dlsym(lib, c"ponte_monitor_catalog_json".as_ptr());
            let result = if symbol.is_null() {
                None
            } else {
                let call = std::mem::transmute::<
                    *mut libc::c_void,
                    unsafe extern "C" fn() -> *mut libc::c_char,
                >(symbol);
                let data = call();
                if data.is_null() {
                    None
                } else {
                    let value =
                        serde_json::from_slice(std::ffi::CStr::from_ptr(data).to_bytes()).ok();
                    libc::free(data.cast());
                    value
                }
            };
            libc::dlclose(lib);
            result
        };
        load().unwrap_or_else(|| serde_json::json!({"monitors":[]}))
    }
}
