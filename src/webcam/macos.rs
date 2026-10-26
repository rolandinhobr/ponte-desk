use super::{Capture, Format, Sink};
use std::{
    ffi::{CStr, CString, c_char, c_void},
    sync::OnceLock,
};
struct Api {
    list: unsafe extern "C" fn() -> *mut c_char,
    start: unsafe extern "C" fn(*const c_char, u32, u32, u32) -> i32,
    copy: unsafe extern "C" fn(*mut u8, u32) -> i32,
    stop: unsafe extern "C" fn(),
    sink_start: unsafe extern "C" fn(u32, u32, u32) -> i32,
    sink_push: unsafe extern "C" fn(*const u8, u32) -> i32,
    sink_stop: unsafe extern "C" fn(),
}
unsafe impl Send for Api {}
unsafe impl Sync for Api {}
fn api() -> Result<&'static Api, String> {
    static API: OnceLock<Result<Api, String>> = OnceLock::new();
    API.get_or_init(|| unsafe {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let path = exe
            .parent()
            .ok_or("executable directory missing")?
            .join("../Frameworks/PonteDeskUI.dylib");
        let name = CString::new(path.as_os_str().as_encoded_bytes()).map_err(|e| e.to_string())?;
        let library = libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        if library.is_null() {
            return Err("native camera module unavailable".into());
        }
        let symbol = |s: &CStr| {
            let ptr = libc::dlsym(library, s.as_ptr());
            if ptr.is_null() {
                Err("native camera symbol missing".to_string())
            } else {
                Ok(ptr)
            }
        };
        Ok(Api {
            list: std::mem::transmute::<*mut c_void, unsafe extern "C" fn() -> *mut c_char>(
                symbol(c"ponte_camera_list_json")?,
            ),
            start: std::mem::transmute::<
                *mut c_void,
                unsafe extern "C" fn(*const c_char, u32, u32, u32) -> i32,
            >(symbol(c"ponte_camera_start")?),
            copy: std::mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut u8, u32) -> i32>(
                symbol(c"ponte_camera_copy_jpeg")?,
            ),
            stop: std::mem::transmute::<*mut c_void, unsafe extern "C" fn()>(symbol(
                c"ponte_camera_stop",
            )?),
            sink_start: std::mem::transmute::<
                *mut c_void,
                unsafe extern "C" fn(u32, u32, u32) -> i32,
            >(symbol(c"ponte_camera_sink_start")?),
            sink_push: std::mem::transmute::<
                *mut c_void,
                unsafe extern "C" fn(*const u8, u32) -> i32,
            >(symbol(c"ponte_camera_sink_push_jpeg")?),
            sink_stop: std::mem::transmute::<*mut c_void, unsafe extern "C" fn()>(symbol(
                c"ponte_camera_sink_stop",
            )?),
        })
    })
    .as_ref()
    .map_err(Clone::clone)
}
pub fn list() -> Result<String, String> {
    let api = api()?;
    unsafe {
        let ptr = (api.list)();
        if ptr.is_null() {
            return Err("camera enumeration failed".into());
        }
        let value = CStr::from_ptr(ptr).to_string_lossy().into_owned();
        libc::free(ptr.cast());
        Ok(value)
    }
}
pub struct Camera {
    api: &'static Api,
}
impl Camera {
    pub fn start(device: &str, format: Format) -> Result<Self, String> {
        let api = api()?;
        let name = CString::new(device).map_err(|e| e.to_string())?;
        let result = unsafe { (api.start)(name.as_ptr(), format.width, format.height, format.fps) };
        if result < 0 {
            return Err(match result {
                -2 => "Autorize Câmera para o Ponte Desk nas permissões do macOS",
                -3 => "Webcam USB externa não encontrada; a câmera integrada não será usada",
                _ => "Não foi possível abrir a webcam externa",
            }
            .into());
        }
        Ok(Self { api })
    }
}
impl Capture for Camera {
    fn copy_jpeg(&mut self, buffer: &mut [u8]) -> Result<usize, String> {
        let result = unsafe { (self.api.copy)(buffer.as_mut_ptr(), buffer.len() as u32) };
        if result < 0 {
            Err("webcam capture ended or frame too large".into())
        } else {
            Ok(result as usize)
        }
    }
}
impl Drop for Camera {
    fn drop(&mut self) {
        unsafe {
            (self.api.stop)();
        }
    }
}
struct CameraSink {
    api: &'static Api,
}
impl Sink for CameraSink {
    fn push(&mut self, jpeg: &[u8]) -> Result<(), String> {
        let result = unsafe { (self.api.sink_push)(jpeg.as_ptr(), jpeg.len() as u32) };
        if result < 0 {
            Err(format!("macOS virtual camera rejected frame ({result})"))
        } else {
            Ok(())
        }
    }
    fn stop(&mut self) {
        unsafe {
            (self.api.sink_stop)();
        }
    }
}
pub fn sink_start(format: Format) -> Result<Box<dyn Sink>, String> {
    let api = api()?;
    let result = unsafe { (api.sink_start)(format.width, format.height, format.fps) };
    if result < 0 {
        Err(format!(
            "Instale/autorize o componente de câmera virtual do Mac ({result})"
        ))
    } else {
        Ok(Box::new(CameraSink { api }))
    }
}
