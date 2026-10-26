#![cfg_attr(windows, windows_subsystem = "windows")]
use env_logger::Env;
use input_capture::InputCaptureError;
use input_emulation::InputEmulationError;
use lan_mouse::{
    capture_test,
    config::{self, Command, Config, ConfigError},
    emulation_test,
    service::{Service, ServiceError},
};
use lan_mouse_cli::CliError;
#[cfg(feature = "gtk")]
use lan_mouse_gtk::GtkError;
use lan_mouse_ipc::{IpcError, IpcListenerCreationError};
use std::{future::Future, io, process};
use thiserror::Error;
use tokio::task::LocalSet;

#[derive(Debug, Error)]
enum LanMouseError {
    #[error(transparent)]
    Service(#[from] ServiceError),
    #[error(transparent)]
    IpcError(#[from] IpcError),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Capture(#[from] InputCaptureError),
    #[error(transparent)]
    Emulation(#[from] InputEmulationError),
    #[cfg(feature = "gtk")]
    #[error(transparent)]
    Gtk(#[from] GtkError),
    #[error(transparent)]
    Cli(#[from] CliError),
}

fn main() {
    // init logging
    let env = Env::default().filter_or("LAN_MOUSE_LOG_LEVEL", "info");
    #[cfg(not(windows))]
    env_logger::init_from_env(env);
    #[cfg(windows)]
    env_logger::Builder::from_env(env)
        .target(windows_log_target())
        .init();

    if let Err(e) = run() {
        log::error!("{e}");
        process::exit(1);
    }
}

/// A windowless Windows process still needs persistent startup diagnostics.
#[cfg(windows)]
fn windows_log_target() -> env_logger::Target {
    use std::{fs, path::PathBuf};
    let root = std::env::var_os("PONTE_DESK_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("PonteDesk")
        });
    let open_log = || -> std::io::Result<fs::File> {
        fs::create_dir_all(&root)?;
        let path = root.join("engine.log");
        if fs::metadata(&path).is_ok_and(|m| m.len() >= 1_048_576) {
            let _ = fs::remove_file(root.join("engine.previous.log"));
            fs::rename(&path, root.join("engine.previous.log"))?;
        }
        fs::OpenOptions::new().create(true).append(true).open(path)
    };
    match open_log() {
        Ok(file) => env_logger::Target::Pipe(Box::new(file)),
        Err(_) => env_logger::Target::Stderr,
    }
}

fn run() -> Result<(), LanMouseError> {
    if let Some(code) = lan_mouse::preferences::dispatch() {
        process::exit(code);
    }
    if let Some(code) = lan_mouse::webcam::dispatch() {
        process::exit(code);
    }
    if let Some(code) = lan_mouse::integration::agent::dispatch() {
        process::exit(code);
    }
    #[cfg(target_os = "macos")]
    if run_native_ui() {
        return Ok(());
    }
    let config = config::Config::new()?;
    match config.command() {
        Some(command) => match command {
            Command::TestEmulation(args) => run_async(emulation_test::run(config, args))?,
            Command::TestCapture(args) => run_async(capture_test::run(config, args))?,
            Command::Cli(cli_args) => run_async(lan_mouse_cli::run(cli_args))?,
            Command::Daemon => {
                // if daemon is specified we run the service
                match run_async(run_service(config)) {
                    Err(LanMouseError::Service(ServiceError::IpcListen(
                        IpcListenerCreationError::AlreadyRunning,
                    ))) => log::info!("service already running!"),
                    r => r?,
                }
            }
        },
        None => {
            //  otherwise start the service as a child process and
            //  run a frontend
            #[cfg(feature = "gtk")]
            {
                // Only spawn a new daemon if one isn't already running
                let mut service = if lan_mouse_ipc::is_service_running() {
                    log::info!("daemon already running, connecting to existing instance");
                    None
                } else {
                    Some(start_service()?)
                };
                let res = lan_mouse_gtk::run(config::local_commit());
                if let Some(ref mut service) = service {
                    #[cfg(unix)]
                    {
                        // on unix we give the service a chance to terminate gracefully
                        let pid = service.id() as libc::pid_t;
                        unsafe {
                            libc::kill(pid, libc::SIGINT);
                        }
                        service.wait()?;
                    }
                    service.kill()?;
                }
                res?;
            }
            #[cfg(not(feature = "gtk"))]
            {
                // run daemon if gtk is diabled
                match run_async(run_service(config)) {
                    Err(LanMouseError::Service(ServiceError::IpcListen(
                        IpcListenerCreationError::AlreadyRunning,
                    ))) => log::info!("service already running!"),
                    r => r?,
                }
            }
        }
    }

    Ok(())
}

/// Use the same signed executable for UI and daemon, so macOS grants one
/// Accessibility identity. The daemon never loads AppKit/Swift UI resources.
#[cfg(target_os = "macos")]
fn run_native_ui() -> bool {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !args.is_empty() && args != ["--background"] {
        return false;
    }
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let Some(dir) = exe.parent() else {
        return false;
    };
    let path = dir.join("../Frameworks/PonteDeskUI.dylib");
    if !path.exists() {
        return false;
    }
    let Ok(path) = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) else {
        return false;
    };
    unsafe {
        let library = libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        if library.is_null() {
            log::error!("unable to load native Ponte Desk UI");
            return true;
        }
        let entry = libc::dlsym(library, c"ponte_desk_app_main".as_ptr());
        if entry.is_null() {
            log::error!("native Ponte Desk UI entry missing");
            return true;
        }
        let run: unsafe extern "C" fn() = std::mem::transmute(entry);
        run();
        libc::dlclose(library);
    }
    true
}

fn run_async<F, E>(f: F) -> Result<(), LanMouseError>
where
    F: Future<Output = Result<(), E>>,
    LanMouseError: From<E>,
{
    // create single threaded tokio runtime
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?;

    // run async event loop
    Ok(runtime.block_on(LocalSet::new().run_until(f))?)
}

#[cfg(feature = "gtk")]
fn start_service() -> Result<std::process::Child, io::Error> {
    let child = process::Command::new(std::env::current_exe()?)
        .args(std::env::args().skip(1))
        .arg("daemon")
        .spawn()?;
    Ok(child)
}

async fn run_service(config: Config) -> Result<(), ServiceError> {
    let release_bind = config.release_bind();
    let config_path = config.config_path().to_owned();
    let mut service = Service::new(config).await?;
    log::info!("using config: {config_path:?}");
    log::info!("Press {release_bind:?} to release the mouse");
    service.run().await?;
    log::info!("service exited!");
    Ok(())
}
