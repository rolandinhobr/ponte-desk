use serde_json::Value;
#[cfg(any(target_os = "macos", windows))]
use std::{
    ffi::{CStr, CString, c_char, c_void},
    sync::OnceLock,
};
#[cfg(any(target_os = "macos", windows))]
struct Api {
    call: unsafe extern "C" fn(*const c_char) -> *mut c_char,
    free: unsafe extern "C" fn(*mut c_char),
}
#[cfg(any(target_os = "macos", windows))]
unsafe impl Send for Api {}
#[cfg(any(target_os = "macos", windows))]
unsafe impl Sync for Api {}
#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryExW(path: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
    fn WaitForSingleObject(handle: *mut c_void, millis: u32) -> u32;
    fn CloseHandle(handle: *mut c_void) -> i32;
}
#[cfg(windows)]
pub fn parent_alive(pid: u32) -> bool {
    unsafe {
        let h = OpenProcess(0x100000, 0, pid);
        if h.is_null() {
            return false;
        }
        let alive = WaitForSingleObject(h, 0) == 258;
        CloseHandle(h);
        alive
    }
}
#[cfg(any(target_os = "macos", windows))]
fn api() -> Result<&'static Api, String> {
    static API: OnceLock<Result<Api, String>> = OnceLock::new();
    API.get_or_init(|| unsafe {
        #[cfg(target_os = "macos")]
        let symbol = {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let path = exe
                .parent()
                .ok_or("Missing executable directory")?
                .join("../Frameworks/PonteDeskUI.dylib");
            let name =
                CString::new(path.as_os_str().as_encoded_bytes()).map_err(|e| e.to_string())?;
            let lib = libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
            if lib.is_null() {
                return Err("Native storage component unavailable".into());
            }
            move |name: &CStr| libc::dlsym(lib, name.as_ptr())
        };
        #[cfg(windows)]
        let symbol = {
            use std::os::windows::ffi::OsStrExt;
            let path = program_files()?.join("PonteDesk/Storage/PonteDeskStorage.dll");
            let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let lib = LoadLibraryExW(path.as_ptr(), std::ptr::null_mut(), 0x100 | 0x800);
            if lib.is_null() {
                return Err("Install the native Ponte Desk storage component".into());
            }
            move |name: &CStr| GetProcAddress(lib, name.as_ptr().cast())
        };
        let call = symbol(c"ponte_storage_call_json");
        let free = symbol(c"ponte_storage_free");
        if call.is_null() || free.is_null() {
            return Err("Storage component version mismatch".into());
        }
        Ok(Api {
            call: std::mem::transmute::<
                *mut c_void,
                unsafe extern "C" fn(*const c_char) -> *mut c_char,
            >(call),
            free: std::mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut c_char)>(free),
        })
    })
    .as_ref()
    .map_err(Clone::clone)
}

#[cfg(windows)]
fn program_files() -> Result<std::path::PathBuf, String> {
    #[repr(C)]
    struct Guid {
        a: u32,
        b: u16,
        c: u16,
        d: [u8; 8],
    }
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SHGetKnownFolderPath(
            id: *const Guid,
            flags: u32,
            token: *mut c_void,
            path: *mut *mut u16,
        ) -> i32;
    }
    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoTaskMemFree(p: *mut c_void);
    }
    use std::os::windows::ffi::OsStringExt;
    unsafe {
        let id = Guid {
            a: 0x6d809377,
            b: 0x6af0,
            c: 0x444b,
            d: [0x89, 0x57, 0xa3, 0x77, 0x3f, 0x02, 0x20, 0x0e],
        };
        let mut path = std::ptr::null_mut();
        if SHGetKnownFolderPath(&id, 0, std::ptr::null_mut(), &mut path) < 0 || path.is_null() {
            return Err("Program Files unavailable".into());
        }
        let mut n = 0;
        while n < 32768 && *path.add(n) != 0 {
            n += 1
        }
        let result = std::path::PathBuf::from(std::ffi::OsString::from_wide(
            std::slice::from_raw_parts(path, n),
        ));
        CoTaskMemFree(path.cast());
        Ok(result)
    }
}

pub fn call(value: &Value) -> Result<Value, String> {
    #[cfg(any(target_os = "macos", windows))]
    {
        let api = api()?;
        let input = CString::new(value.to_string()).map_err(|e| e.to_string())?;
        unsafe {
            let result = (api.call)(input.as_ptr());
            if result.is_null() {
                return Err("Storage operation failed".into());
            }
            let bytes = CStr::from_ptr(result).to_bytes();
            let decoded = if bytes.len() <= 16384 {
                serde_json::from_slice::<Value>(bytes).map_err(|e| e.to_string())
            } else {
                Err("Storage response too large".into())
            };
            (api.free)(result);
            let decoded = decoded?;
            if let Some(e) = decoded["error"].as_str() {
                Err(e.into())
            } else {
                Ok(decoded)
            }
        }
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = value;
        Err("Storage requires macOS or Windows".into())
    }
}
