//! Windows MF virtual-camera sink. DLL is loaded only for an active session.
use super::Sink;
use std::ffi::{OsStr, c_void};
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::ptr;
type Start = unsafe extern "C" fn(u32, u32, u32, *mut *mut c_void) -> i32;
type Push = unsafe extern "C" fn(*mut c_void, *const u8, u32) -> i32;
type Stop = unsafe extern "C" fn(*mut c_void) -> i32;
#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryExW(path: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    fn FreeLibrary(module: *mut c_void) -> i32;
}
struct CameraSink {
    module: *mut c_void,
    context: *mut c_void,
    push: Push,
    stop: Stop,
}
impl Sink for CameraSink {
    fn push(&mut self, jpeg: &[u8]) -> Result<(), String> {
        if self.context.is_null() {
            return Err("Câmera virtual encerrada".into());
        }
        if jpeg.is_empty() || jpeg.len() > 2_097_152 {
            return Err("JPEG deve ter entre 1 byte e 2 MiB".into());
        }
        let hr = unsafe { (self.push)(self.context, jpeg.as_ptr(), jpeg.len() as u32) };
        if hr < 0 {
            return Err(format!(
                "Falha ao publicar quadro da câmera: 0x{:08X}",
                hr as u32
            ));
        }
        Ok(())
    }
    fn stop(&mut self) {
        if !self.context.is_null() {
            let hr = unsafe { (self.stop)(self.context) };
            if hr < 0 {
                log::warn!("Webcam stop failed: 0x{:08X}", hr as u32);
            }
            self.context = ptr::null_mut();
        }
    }
}
impl Drop for CameraSink {
    fn drop(&mut self) {
        self.stop();
        unsafe {
            FreeLibrary(self.module);
        }
    }
}
pub fn start(width: u32, height: u32, fps: u32) -> Result<Box<dyn Sink>, String> {
    let path = std::env::var_os("PONTE_DESK_WEBCAM_DLL")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(
                std::env::var_os("ProgramFiles").unwrap_or_else(|| "C:\\Program Files".into()),
            )
            .join("PonteDesk")
            .join("Webcam")
            .join("PonteDeskWebcam.dll")
        });
    let wide: Vec<u16> = OsStr::new(&path).encode_wide().chain(Some(0)).collect();
    // Restrict dependent DLL lookup to the protected DLL directory and System32.
    let module = unsafe { LoadLibraryExW(wide.as_ptr(), ptr::null_mut(), 0x100 | 0x800) };
    if module.is_null() {
        return Err(format!(
            "Instale o componente Windows da webcam Ponte Desk: {}",
            std::io::Error::last_os_error()
        ));
    }
    let result = (|| {
        let get = |name: &[u8]| -> Result<*mut c_void, String> {
            let address = unsafe { GetProcAddress(module, name.as_ptr()) };
            if address.is_null() {
                Err("Componente Windows da webcam incompatível; reinstale-o".into())
            } else {
                Ok(address)
            }
        };
        let begin: Start = unsafe { std::mem::transmute(get(b"PonteWebcamStart\0")?) };
        let push: Push = unsafe { std::mem::transmute(get(b"PonteWebcamPush\0")?) };
        let stop: Stop = unsafe { std::mem::transmute(get(b"PonteWebcamStop\0")?) };
        let mut context = ptr::null_mut();
        let hr = unsafe { begin(width, height, fps, &mut context) };
        if hr < 0 {
            return Err(format!(
                "Não foi possível iniciar a câmera virtual: 0x{:08X}",
                hr as u32
            ));
        }
        if context.is_null() {
            return Err("A câmera virtual não confirmou inicialização".into());
        }
        Ok(Box::new(CameraSink {
            module,
            context,
            push,
            stop,
        }) as Box<dyn Sink>)
    })();
    if result.is_err() {
        unsafe {
            FreeLibrary(module);
        }
    }
    result
}

type CaptureList = unsafe extern "C" fn(*mut u8, u32, *mut u32) -> i32;
type CaptureStart = unsafe extern "C" fn(*const u16, u32, u32, u32, *mut *mut c_void) -> i32;
type CaptureCopy = unsafe extern "C" fn(*mut c_void, *mut u8, u32, *mut u32) -> i32;
fn load_capture_library() -> Result<*mut c_void, String> {
    let path = std::env::var_os("PONTE_DESK_WEBCAM_DLL")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(
                std::env::var_os("ProgramFiles").unwrap_or_else(|| "C:\\Program Files".into()),
            )
            .join("PonteDesk")
            .join("Webcam")
            .join("PonteDeskWebcam.dll")
        });
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let module = unsafe { LoadLibraryExW(wide.as_ptr(), ptr::null_mut(), 0x100 | 0x800) };
    if module.is_null() {
        Err(format!(
            "Instale o componente Windows da webcam Ponte Desk: {}",
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(module)
    }
}
fn capture_proc(module: *mut c_void, name: &[u8]) -> Result<*mut c_void, String> {
    let proc = unsafe { GetProcAddress(module, name.as_ptr()) };
    if proc.is_null() {
        Err("Componente de captura Windows incompatível; reinstale-o".into())
    } else {
        Ok(proc)
    }
}
pub fn list() -> Result<String, String> {
    let module = load_capture_library()?;
    let result = (|| {
        let list: CaptureList =
            unsafe { std::mem::transmute(capture_proc(module, b"PonteCaptureList\0")?) };
        let mut needed = 0;
        let hr = unsafe { list(ptr::null_mut(), 0, &mut needed) };
        if hr < 0 {
            return Err(format!(
                "Falha ao enumerar webcams USB: 0x{:08X}",
                hr as u32
            ));
        }
        if needed == 0 || needed > 1_048_576 {
            return Err("Lista de webcams inválida".into());
        }
        let mut bytes = vec![0; needed as usize];
        let hr = unsafe { list(bytes.as_mut_ptr(), bytes.len() as u32, &mut needed) };
        if hr < 0 {
            return Err(format!(
                "Falha ao enumerar webcams USB: 0x{:08X}",
                hr as u32
            ));
        }
        let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
        String::from_utf8(bytes[..end].to_vec()).map_err(|e| e.to_string())
    })();
    unsafe {
        FreeLibrary(module);
    }
    result
}
struct CameraCapture {
    module: *mut c_void,
    context: *mut c_void,
    copy: CaptureCopy,
    stop: Stop,
}
impl super::Capture for CameraCapture {
    fn copy_jpeg(&mut self, buffer: &mut [u8]) -> Result<usize, String> {
        if self.context.is_null() {
            return Err("Captura encerrada".into());
        }
        let mut length = 0;
        let capacity = buffer.len().min(2_097_152) as u32;
        let hr = unsafe { (self.copy)(self.context, buffer.as_mut_ptr(), capacity, &mut length) };
        if hr < 0 {
            return Err(format!(
                "Falha na webcam USB selecionada: 0x{:08X}",
                hr as u32
            ));
        }
        if length > capacity {
            return Err("Quadro da webcam excedeu o limite".into());
        }
        Ok(length as usize)
    }
}
impl Drop for CameraCapture {
    fn drop(&mut self) {
        if !self.context.is_null() {
            let hr = unsafe { (self.stop)(self.context) };
            if hr < 0 {
                log::warn!("Webcam capture stop failed: 0x{:08X}", hr as u32);
            }
        }
        unsafe {
            FreeLibrary(self.module);
        }
    }
}
pub fn start_capture(
    device_id: &str,
    format: super::Format,
) -> Result<Box<dyn super::Capture>, String> {
    if device_id.is_empty() || device_id.contains('\0') {
        return Err("Selecione uma webcam USB externa".into());
    }
    let module = load_capture_library()?;
    let result = (|| {
        let start: CaptureStart =
            unsafe { std::mem::transmute(capture_proc(module, b"PonteCaptureStart\0")?) };
        let copy: CaptureCopy =
            unsafe { std::mem::transmute(capture_proc(module, b"PonteCaptureCopy\0")?) };
        let stop: Stop =
            unsafe { std::mem::transmute(capture_proc(module, b"PonteCaptureStop\0")?) };
        let wide: Vec<u16> = OsStr::new(device_id).encode_wide().chain(Some(0)).collect();
        let mut context = ptr::null_mut();
        let hr = unsafe {
            start(
                wide.as_ptr(),
                format.width,
                format.height,
                format.fps,
                &mut context,
            )
        };
        if hr < 0 {
            return Err(format!(
                "Não foi possível abrir a webcam USB selecionada: 0x{:08X}",
                hr as u32
            ));
        }
        if context.is_null() {
            return Err("A webcam USB não confirmou inicialização".into());
        }
        Ok(Box::new(CameraCapture {
            module,
            context,
            copy,
            stop,
        }) as Box<dyn super::Capture>)
    })();
    if result.is_err() {
        unsafe {
            FreeLibrary(module);
        }
    }
    result
}
