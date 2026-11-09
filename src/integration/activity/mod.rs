//! Process-lifetime native listeners, enabled only for automatic routing.
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;
pub(super) fn start() {
    #[cfg(target_os = "macos")]
    macos::start();
    #[cfg(windows)]
    windows::start();
}
