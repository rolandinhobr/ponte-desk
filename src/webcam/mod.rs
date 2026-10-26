//! External USB camera in either direction, opt-in with a pinned TLS stream.
//! Camera pixels never enter the KVM event loop or persistent logs.
use hmac::{Hmac, Mac};
use rustls::{
    ClientConfig, ClientConnection, ServerConfig, ServerConnection, StreamOwned,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::{self, Read, Write},
    net::{IpAddr, SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

pub trait Capture {
    fn copy_jpeg(&mut self, buffer: &mut [u8]) -> Result<usize, String>;
}
pub trait Sink {
    fn push(&mut self, jpeg: &[u8]) -> Result<(), String>;
    fn stop(&mut self);
}
pub const MAX_JPEG: usize = 2 * 1024 * 1024;
const PORT: u16 = 4244;
const MAGIC: &[u8; 8] = b"PDCAM001";
const DOMAIN: &[u8] = b"ponte-desk-external-webcam-v1";
#[derive(Deserialize)]
struct Settings {
    side: String,
    key: String,
    peers: Vec<String>,
}
#[derive(Deserialize)]
struct KvmSettings {
    #[serde(default)]
    authorized_fingerprints: HashMap<String, String>,
}
#[derive(Clone, Copy, Debug)]
pub struct Format {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}
impl Format {
    fn validate(self) -> io::Result<Self> {
        if ![(1280, 720), (1920, 1080)].contains(&(self.width, self.height)) || self.fps != 30 {
            Err(io::Error::other("unsupported webcam format"))
        } else {
            Ok(self)
        }
    }
}
fn quality_format(quality: Option<&str>) -> Result<Format, String> {
    let (width, height) = match quality.unwrap_or("1080p") {
        "1080p" => (1920, 1080),
        "720p" => (1280, 720),
        _ => return Err("Escolha qualidade 1080p ou 720p".into()),
    };
    Ok(Format {
        width,
        height,
        fps: 30,
    })
}
fn cli_quality(args: &[String]) -> Result<&str, String> {
    match args.iter().position(|arg| arg == "--quality") {
        Some(index) => args
            .get(index + 1)
            .map(String::as_str)
            .ok_or_else(|| "missing --quality value".to_string()),
        None => Ok("1080p"),
    }
}
pub fn root() -> PathBuf {
    if let Some(path) = std::env::var_os("PONTE_DESK_HOME") {
        return path.into();
    }
    #[cfg(windows)]
    {
        PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("PonteDesk")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config/ponte-desk")
    }
}
fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}
fn settings() -> io::Result<Settings> {
    Ok(serde_json::from_slice(&fs::read(
        root().join("observer.json"),
    )?)?)
}
fn pairing_key(cfg: &Settings) -> io::Result<Vec<u8>> {
    unhex(&cfg.key)
        .filter(|v| v.len() == 32)
        .ok_or_else(|| io::Error::other("invalid pairing key"))
}
fn status(mode: &str, frames: u64, message: &str) {
    #[cfg(not(test))]
    let folder = root();
    #[cfg(test)]
    let folder =
        std::env::temp_dir().join(format!("ponte-desk-camera-tests-{}", std::process::id()));
    #[cfg(test)]
    let _ = fs::create_dir_all(&folder);
    let path = folder.join("webcam-status.json");
    let tmp = path.with_extension("tmp");
    let data = serde_json::json!({"mode":mode,"frames":frames,"message":message,"pid":std::process::id(),"timestamp_ms":crate::integration::now_ms()});
    if let Ok(bytes) = serde_json::to_vec(&data) {
        if fs::write(&tmp, bytes).is_ok() {
            let _ = fs::rename(tmp, path);
        }
    }
}
fn proof(key: &[u8], challenge: &[u8; 32], header: &[u8; 20]) -> [u8; 32] {
    let mut h = Hmac::<Sha256>::new_from_slice(key).expect("HMAC");
    h.update(DOMAIN);
    h.update(challenge);
    h.update(header);
    h.finalize().into_bytes().into()
}
fn encode_header(format: Format) -> [u8; 20] {
    let mut h = [0; 20];
    h[..8].copy_from_slice(MAGIC);
    h[8..12].copy_from_slice(&format.width.to_be_bytes());
    h[12..16].copy_from_slice(&format.height.to_be_bytes());
    h[16..20].copy_from_slice(&format.fps.to_be_bytes());
    h
}
fn read_header(h: &[u8; 20]) -> io::Result<Format> {
    if &h[..8] != MAGIC {
        return Err(io::Error::other("invalid camera protocol"));
    }
    Format {
        width: u32::from_be_bytes(h[8..12].try_into().unwrap()),
        height: u32::from_be_bytes(h[12..16].try_into().unwrap()),
        fps: u32::from_be_bytes(h[16..20].try_into().unwrap()),
    }
    .validate()
}
#[derive(Debug)]
struct PinnedServer {
    pins: Vec<[u8; 32]>,
}
impl rustls::client::danger::ServerCertVerifier for PinnedServer {
    fn verify_server_cert(
        &self,
        end: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let digest: [u8; 32] = Sha256::digest(end.as_ref()).into();
        if self.pins.contains(&digest) {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(
                "webcam peer certificate is not paired".into(),
            ))
        }
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
fn client_config() -> io::Result<Arc<ClientConfig>> {
    let kvm: KvmSettings = toml::from_str(&fs::read_to_string(root().join("config.toml"))?)
        .map_err(io::Error::other)?;
    let pins: Vec<[u8; 32]> = kvm
        .authorized_fingerprints
        .keys()
        .filter_map(|s| unhex(&s.replace(':', ""))?.try_into().ok())
        .collect();
    if pins.is_empty() {
        return Err(io::Error::other(
            "pair the Windows certificate before sharing a camera",
        ));
    }
    let config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(io::Error::other)?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PinnedServer { pins }))
            .with_no_client_auth();
    Ok(Arc::new(config))
}
fn server_config() -> io::Result<Arc<ServerConfig>> {
    let blocks =
        pem::parse_many(fs::read(root().join("ponte-desk.pem"))?).map_err(io::Error::other)?;
    let mut certs = Vec::new();
    let mut key = None;
    for block in blocks {
        match block.tag() {
            "CERTIFICATE" => certs.push(CertificateDer::from(block.contents().to_vec())),
            "PRIVATE_KEY" | "PRIVATE KEY" => {
                key = Some(PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
                    block.contents().to_vec(),
                )))
            }
            _ => {}
        }
    }
    let key = key.ok_or_else(|| io::Error::other("camera TLS certificate missing"))?;
    let config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(io::Error::other)?
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(io::Error::other)?;
    Ok(Arc::new(config))
}
fn socket(stream: &TcpStream) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    stream.set_nodelay(true)
}
struct ActiveSink(Box<dyn Sink>);
impl Drop for ActiveSink {
    fn drop(&mut self) {
        self.0.stop();
        status("idle", 0, "Webcam remota desligada");
    }
}
fn receive_connection(
    stream: TcpStream,
    tls: Arc<ServerConfig>,
    key: &[u8],
    stop: &AtomicBool,
    control: &std::sync::Mutex<Option<std::process::Child>>,
    video: &std::sync::Mutex<()>,
    make_sink: impl FnOnce(Format) -> Result<Box<dyn Sink>, String>,
) -> io::Result<()> {
    let peer_ip = stream.peer_addr()?.ip();
    socket(&stream)?;
    let mut wire = StreamOwned::new(
        ServerConnection::new(tls).map_err(io::Error::other)?,
        stream,
    );
    let mut challenge = [0; 32];
    rustls::crypto::ring::default_provider()
        .secure_random
        .fill(&mut challenge)
        .map_err(|_| io::Error::other("OS RNG failed"))?;
    wire.write_all(&challenge)?;
    wire.flush()?;
    let mut header = [0; 20];
    let mut auth = [0; 32];
    wire.read_exact(&mut header)?;
    wire.read_exact(&mut auth)?;
    let mut verifier = Hmac::<Sha256>::new_from_slice(key).expect("HMAC");
    verifier.update(DOMAIN);
    verifier.update(&challenge);
    verifier.update(&header);
    verifier
        .verify_slice(&auth)
        .map_err(|_| io::Error::other("camera pairing authentication failed"))?;
    if &header[..8] == b"PDCMD001" {
        return receive_command(&mut wire, peer_ip, control);
    }
    let format = read_header(&header)?;
    let _exclusive = video
        .lock()
        .map_err(|_| io::Error::other("camera receiver unavailable"))?;
    if stop.load(Ordering::Relaxed) {
        return Err(io::Error::other("camera receiver stopped"));
    }
    let result = if crate::preferences::Preferences::load(&root()).webcam_enabled {
        make_sink(format)
    } else {
        Err("Webcam sharing is disabled in settings".into())
    };
    let mut sink = match result {
        Ok(sink) => ActiveSink(sink),
        Err(message) => {
            let message = if message.len() > 4096 {
                "camera receiver unavailable".to_string()
            } else {
                message
            };
            wire.write_all(b"ERROR")?;
            wire.write_all(&(message.len() as u32).to_be_bytes())?;
            wire.write_all(message.as_bytes())?;
            wire.flush()?;
            status("error", 0, &message);
            return Err(io::Error::other(message));
        }
    };
    wire.write_all(b"READY")?;
    wire.flush()?;
    // Cold USB capture startup can exceed the steady-state network timeout.
    // Only an authenticated connection gets this initial grace period.
    wire.sock.set_read_timeout(Some(Duration::from_secs(15)))?;
    let receiving_message = format!(
        "Webcam externa recebida · {}×{}",
        format.width, format.height
    );
    status("receiving", 0, &receiving_message);
    let mut last = 0;
    let mut jpeg = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        let mut prefix = [0; 12];
        wire.read_exact(&mut prefix)?;
        let length = u32::from_be_bytes(prefix[..4].try_into().unwrap()) as usize;
        if length == 0 {
            break;
        }
        if length > MAX_JPEG {
            return Err(io::Error::other("camera frame exceeds limit"));
        }
        let sequence = u64::from_be_bytes(prefix[4..].try_into().unwrap());
        if sequence <= last {
            return Err(io::Error::other("camera sequence invalid"));
        }
        jpeg.resize(length, 0);
        wire.read_exact(&mut jpeg)?;
        if !jpeg.starts_with(&[0xff, 0xd8]) {
            return Err(io::Error::other("invalid JPEG frame"));
        }
        sink.0.push(&jpeg).map_err(io::Error::other)?;
        if last == 0 {
            wire.sock.set_read_timeout(Some(Duration::from_secs(3)))?;
        }
        last = sequence;
        if last == 1 || last.is_multiple_of(30) {
            status("receiving", last, &receiving_message);
        }
    }
    Ok(())
}

type Sessions = Arc<std::sync::Mutex<Vec<(TcpStream, thread::JoinHandle<()>)>>>;
pub struct Receiver {
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
    sessions: Sessions,
}
impl Drop for Receiver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect_timeout(
            &SocketAddr::from(([127, 0, 0, 1], PORT)),
            Duration::from_millis(50),
        );
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        if let Ok(mut sessions) = self.sessions.lock() {
            for (socket, _) in sessions.iter() {
                let _ = socket.shutdown(std::net::Shutdown::Both);
            }
            for (_, thread) in sessions.drain(..) {
                let _ = thread.join();
            }
        }
    }
}
pub fn start_receiver() -> Option<Receiver> {
    match receiver() {
        Ok(server) => Some(server),
        Err(err) => {
            log::warn!("Webcam receiver unavailable: {err}");
            status("unavailable", 0, "Receptor de webcam indisponível");
            None
        }
    }
}
fn receiver() -> io::Result<Receiver> {
    let cfg = settings()?;
    if !["pc", "mac"].contains(&cfg.side.as_str()) {
        return Err(io::Error::other("invalid camera role"));
    }
    let key = pairing_key(&cfg)?;
    let tls = server_config()?;
    let listener = TcpListener::bind(("0.0.0.0", PORT))?;
    let allowed: Vec<IpAddr> = cfg.peers.iter().filter_map(|p| p.parse().ok()).collect();
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let controls = Arc::new(std::sync::Mutex::new(None));
    let children = controls.clone();
    let sessions: Sessions = Arc::new(std::sync::Mutex::new(Vec::new()));
    let active = sessions.clone();
    let video = Arc::new(std::sync::Mutex::new(()));
    let worker = thread::spawn(move || {
        let connections = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        status("idle", 0, "Webcam remota desligada");
        for next in listener.incoming() {
            if stopped.load(Ordering::Relaxed) {
                break;
            }
            let Ok(stream) = next else {
                continue;
            };
            if !stream.peer_addr().is_ok_and(|a| allowed.contains(&a.ip())) {
                continue;
            }
            if connections.fetch_add(1, Ordering::SeqCst) >= 3 {
                connections.fetch_sub(1, Ordering::SeqCst);
                continue;
            }
            let count = connections.clone();
            let tls = tls.clone();
            let key = key.clone();
            let stopped = stopped.clone();
            let controls = controls.clone();
            let video = video.clone();
            let Ok(cancel) = stream.try_clone() else {
                count.fetch_sub(1, Ordering::SeqCst);
                continue;
            };
            let session = thread::spawn(move || {
                if let Err(error) = receive_connection(
                    stream,
                    tls,
                    &key,
                    &stopped,
                    &controls,
                    &video,
                    platform_sink,
                ) {
                    log::info!("Webcam session ended: {error}");
                }
                count.fetch_sub(1, Ordering::SeqCst);
            });
            if let Ok(mut sessions) = active.lock() {
                let mut i = 0;
                while i < sessions.len() {
                    if sessions[i].1.is_finished() {
                        let (_, thread) = sessions.swap_remove(i);
                        let _ = thread.join();
                    } else {
                        i += 1;
                    }
                }
                sessions.push((cancel, session));
            }
        }
        if let Ok(mut child) = children.lock() {
            if let Some(mut child) = child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    });
    Ok(Receiver {
        stop,
        thread: Some(worker),
        sessions,
    })
}
fn send(device: &str, peer: &str, format: Format) -> io::Result<()> {
    if !crate::preferences::Preferences::load(&root()).webcam_enabled {
        return Err(io::Error::other("Webcam sharing is disabled in settings"));
    }
    let cfg = settings()?;
    if !cfg.peers.iter().any(|p| p == peer) {
        return Err(io::Error::other("select the paired Windows computer"));
    }
    let key = pairing_key(&cfg)?;
    let address: IpAddr = peer.parse().map_err(io::Error::other)?;
    let stream =
        TcpStream::connect_timeout(&SocketAddr::new(address, PORT), Duration::from_secs(3))?;
    socket(&stream)?;
    let mut wire = StreamOwned::new(
        ClientConnection::new(
            client_config()?,
            ServerName::try_from("ponte-desk.local").unwrap(),
        )
        .map_err(io::Error::other)?,
        stream,
    );
    let mut challenge = [0; 32];
    wire.read_exact(&mut challenge)?;
    let header = encode_header(format);
    wire.write_all(&header)?;
    wire.write_all(&proof(&key, &challenge, &header))?;
    wire.flush()?;
    // Media Foundation may start FrameServer on the first activation. Keep
    // authentication/steady streaming bounded at 3s, but allow cold setup.
    wire.sock.set_read_timeout(Some(Duration::from_secs(15)))?;
    let mut ready = [0; 5];
    wire.read_exact(&mut ready).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("A câmera virtual do outro computador não ficou pronta: {error}"),
        )
    })?;
    wire.sock.set_read_timeout(Some(Duration::from_secs(3)))?;
    if &ready == b"ERROR" {
        let mut count = [0; 4];
        wire.read_exact(&mut count)?;
        let length = u32::from_be_bytes(count) as usize;
        if length > 4096 {
            return Err(io::Error::other("invalid camera error"));
        }
        let mut message = vec![0; length];
        wire.read_exact(&mut message)?;
        return Err(io::Error::other(
            String::from_utf8_lossy(&message).into_owned(),
        ));
    }
    if &ready != b"READY" {
        return Err(io::Error::other("camera receiver did not become ready"));
    }
    let mut camera = platform_capture(device, format).map_err(io::Error::other)?;
    let mut buffer = vec![0; MAX_JPEG];
    let mut frames = 0u64;
    let mut last_frame = std::time::Instant::now();
    let mut parent_check = std::time::Instant::now() - Duration::from_secs(2);
    let destination = if cfg.side == "mac" { "Windows" } else { "Mac" };
    let sending_message = format!(
        "Webcam externa enviada ao {destination} · {}×{}",
        format.width, format.height
    );
    status("sending", 0, &sending_message);
    loop {
        if parent_check.elapsed() >= Duration::from_secs(1) {
            if !parent_alive() {
                return Err(io::Error::other("camera controller exited"));
            }
            parent_check = std::time::Instant::now();
        }
        let length = camera.copy_jpeg(&mut buffer).map_err(io::Error::other)?;
        if length == 0 {
            if last_frame.elapsed() > Duration::from_secs(5) {
                return Err(io::Error::other("webcam did not provide frames"));
            }
            continue;
        }
        last_frame = std::time::Instant::now();
        frames += 1;
        wire.write_all(&(length as u32).to_be_bytes())?;
        wire.write_all(&frames.to_be_bytes())?;
        wire.write_all(&buffer[..length])?;
        wire.flush()?;
        if frames == 1 || frames.is_multiple_of(30) {
            status("sending", frames, &sending_message);
        }
    }
}
#[derive(Serialize)]
struct CommandError {
    error: String,
}
pub fn dispatch() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some("webcam") {
        return None;
    }
    let result = (|| -> Result<(), String> {
        match args.get(1).map(String::as_str) {
            Some("doctor") => {
                let local = platform_list().and_then(|json| {
                    serde_json::from_str::<serde_json::Value>(&json).map_err(|e| e.to_string())
                });
                let server = server_config().is_ok();
                let client = client_config().is_ok();
                println!(
                    "{}",
                    serde_json::json!({"tls_server_ready":server,"paired_certificates_ready":client,"external_devices":local.unwrap_or_else(|e|serde_json::json!({"error":e}))})
                );
                Ok(())
            }
            Some("devices") | Some("request") | Some("stop") | Some("peer-status") => {
                let arg = |name: &str| {
                    args.iter()
                        .position(|a| a == name)
                        .and_then(|i| args.get(i + 1))
                        .map(String::as_str)
                        .ok_or_else(|| format!("missing {name}"))
                };
                let command = match args[1].as_str() {
                    "devices" => serde_json::json!({"op":"list"}),
                    "peer-status" => serde_json::json!({"op":"status"}),
                    "request" => {
                        let quality = cli_quality(&args)?;
                        quality_format(Some(quality))?;
                        serde_json::json!({"op":"send","device":arg("--device")?,"quality":quality})
                    }
                    _ => serde_json::json!({"op":"stop"}),
                };
                println!(
                    "{}",
                    remote_command(arg("--peer")?, command).map_err(|e| e.to_string())?
                );
                Ok(())
            }
            Some("status") => {
                println!(
                    "{}",
                    fs::read_to_string(root().join("webcam-status.json"))
                        .unwrap_or_else(|_| "{\"mode\":\"idle\"}".into())
                );
                Ok(())
            }
            Some("list") => {
                println!("{}", platform_list()?);
                Ok(())
            }
            Some("send") => {
                let arg = |name: &str| {
                    args.iter()
                        .position(|a| a == name)
                        .and_then(|i| args.get(i + 1))
                        .map(String::as_str)
                        .ok_or_else(|| format!("missing {name}"))
                };
                let format = quality_format(Some(cli_quality(&args)?))?;
                send(arg("--device")?, arg("--peer")?, format).map_err(|e| e.to_string())
            }
            _ => Err("use webcam list, webcam status or webcam send --device ID --peer IP".into()),
        }
    })();
    Some(match result {
        Ok(()) => 0,
        Err(error) => {
            status("error", 0, &error);
            eprintln!(
                "{}",
                serde_json::to_string(&CommandError { error }).unwrap()
            );
            1
        }
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quality_defaults_to_full_hd_and_rejects_unknown_values() {
        let full = quality_format(None).unwrap();
        assert_eq!((full.width, full.height, full.fps), (1920, 1080, 30));
        let light = quality_format(Some("720p")).unwrap();
        assert_eq!((light.width, light.height, light.fps), (1280, 720, 30));
        assert!(quality_format(Some("4k")).is_err());
        assert!(cli_quality(&["--quality".into()]).is_err());
    }
    #[test]
    fn camera_header_only_accepts_bounded_supported_formats() {
        assert!(
            read_header(&encode_header(Format {
                width: 1280,
                height: 720,
                fps: 30
            }))
            .is_ok()
        );
        assert!(
            read_header(&encode_header(Format {
                width: 99999,
                height: 99999,
                fps: 30
            }))
            .is_err()
        );
    }
    #[test]
    fn auth_binds_challenge_and_format() {
        let key = [42u8; 32];
        let header = encode_header(Format {
            width: 1280,
            height: 720,
            fps: 30,
        });
        assert_ne!(
            proof(&key, &[1; 32], &header),
            proof(&key, &[2; 32], &header)
        );
        let mut changed = header;
        changed[12] ^= 1;
        assert_ne!(
            proof(&key, &[1; 32], &header),
            proof(&key, &[1; 32], &changed)
        );
    }
}
fn platform_list() -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        macos::list()
    }
    #[cfg(windows)]
    {
        windows::list()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        Err("camera requires macOS or Windows".into())
    }
}
fn platform_capture(device: &str, format: Format) -> Result<Box<dyn Capture>, String> {
    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(macos::Camera::start(device, format)?))
    }
    #[cfg(windows)]
    {
        windows::start_capture(device, format)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (device, format);
        Err("camera requires macOS or Windows".into())
    }
}
fn platform_sink(format: Format) -> Result<Box<dyn Sink>, String> {
    #[cfg(target_os = "macos")]
    {
        macos::sink_start(format)
    }
    #[cfg(windows)]
    {
        windows::start(format.width, format.height, format.fps)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = format;
        Err("camera requires macOS or Windows".into())
    }
}
pub(crate) fn remote_command(
    peer: &str,
    request: serde_json::Value,
) -> io::Result<serde_json::Value> {
    let cfg = settings()?;
    if !cfg.peers.iter().any(|p| p == peer) {
        return Err(io::Error::other("camera control requires paired peer"));
    }
    let address: IpAddr = peer.parse().map_err(io::Error::other)?;
    let stream =
        TcpStream::connect_timeout(&SocketAddr::new(address, PORT), Duration::from_secs(3))?;
    socket(&stream)?;
    let mut wire = StreamOwned::new(
        ClientConnection::new(
            client_config()?,
            ServerName::try_from("ponte-desk.local").unwrap(),
        )
        .map_err(io::Error::other)?,
        stream,
    );
    let mut challenge = [0; 32];
    wire.read_exact(&mut challenge)?;
    let mut header = [0u8; 20];
    header[..8].copy_from_slice(b"PDCMD001");
    wire.write_all(&header)?;
    wire.write_all(&proof(&pairing_key(&cfg)?, &challenge, &header))?;
    let data = serde_json::to_vec(&request)?;
    wire.write_all(&(data.len() as u32).to_be_bytes())?;
    wire.write_all(&data)?;
    wire.flush()?;
    let mut length = [0; 4];
    wire.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    if length > 16384 {
        return Err(io::Error::other("camera reply too long"));
    }
    let mut data = vec![0; length];
    wire.read_exact(&mut data)?;
    Ok(serde_json::from_slice(&data)?)
}
fn receive_command(
    wire: &mut StreamOwned<ServerConnection, TcpStream>,
    peer: IpAddr,
    control: &std::sync::Mutex<Option<std::process::Child>>,
) -> io::Result<()> {
    let mut length = [0; 4];
    wire.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    if length > 4096 {
        return Err(io::Error::other("camera request too long"));
    }
    let mut data = vec![0; length];
    wire.read_exact(&mut data)?;
    let request: serde_json::Value = serde_json::from_slice(&data)?;
    let result = (|| -> Result<serde_json::Value, String> {
        match request["op"].as_str() {
            Some("monitors") => Ok(crate::integration::monitor_catalog()),
            Some("layout") => {
                crate::preferences::set_layout(request["position"].as_str().unwrap_or(""))
                    .map_err(|e| e.to_string())
            }
            Some("status") => {
                let bytes =
                    fs::read(root().join("webcam-status.json")).map_err(|e| e.to_string())?;
                serde_json::from_slice(&bytes).map_err(|e| e.to_string())
            }
            Some("list") => serde_json::from_str(&platform_list()?).map_err(|e| e.to_string()),
            Some("stop") => {
                let mut guard = control
                    .lock()
                    .map_err(|_| "camera controller unavailable")?;
                if let Some(mut child) = guard.take() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                status("idle", 0, "Webcam externa desligada");
                Ok(serde_json::json!({"ok":true}))
            }
            Some("send") => {
                let quality = match request.get("quality") {
                    Some(value) => value.as_str().ok_or("invalid camera quality")?,
                    None => "1080p",
                };
                quality_format(Some(quality))?;
                let device = request["device"]
                    .as_str()
                    .filter(|s| !s.is_empty() && s.len() < 2048)
                    .ok_or("select an external USB camera")?;
                let available: serde_json::Value =
                    serde_json::from_str(&platform_list()?).map_err(|e| e.to_string())?;
                if !available["devices"]
                    .as_array()
                    .is_some_and(|devices| devices.iter().any(|d| d["id"] == device))
                {
                    return Err("external USB camera not found".into());
                }
                let mut guard = control
                    .lock()
                    .map_err(|_| "camera controller unavailable")?;
                if let Some(mut child) = guard.take() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                let mut command =
                    std::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
                command.args([
                    "webcam",
                    "send",
                    "--device",
                    device,
                    "--peer",
                    &peer.to_string(),
                    "--parent",
                    &std::process::id().to_string(),
                    "--quality",
                    quality,
                ]);
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    command.creation_flags(0x08000000);
                }
                command
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
                let child = command.spawn().map_err(|e| e.to_string())?;
                let pid = child.id();
                *guard = Some(child);
                Ok(serde_json::json!({"ok":true,"pid":pid}))
            }
            _ => Err("unsupported camera command".into()),
        }
    })();
    let reply = match result {
        Ok(value) => value,
        Err(error) => serde_json::json!({"error":error}),
    };
    let data = serde_json::to_vec(&reply)?;
    wire.write_all(&(data.len() as u32).to_be_bytes())?;
    wire.write_all(&data)?;
    wire.flush()
}
fn parent_alive() -> bool {
    let args: Vec<_> = std::env::args().collect();
    let Some(parent) = args
        .iter()
        .position(|s| s == "--parent")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse::<u32>().ok())
    else {
        return true;
    };
    #[cfg(unix)]
    {
        unsafe { libc::kill(parent as i32, 0) == 0 }
    }
    #[cfg(windows)]
    {
        unsafe {
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
                fn GetExitCodeProcess(handle: *mut std::ffi::c_void, code: *mut u32) -> i32;
                fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
            }
            let process = OpenProcess(0x1000, 0, parent);
            if process.is_null() {
                return false;
            }
            let mut code = 0;
            let alive = GetExitCodeProcess(process, &mut code) != 0 && code == 259;
            CloseHandle(process);
            alive
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = parent;
        false
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    fn configurations(pin_ok: bool) -> (Arc<ServerConfig>, Arc<ClientConfig>) {
        let identity = rcgen::generate_simple_self_signed(vec!["ponte-desk.local".into()]).unwrap();
        let cert = identity.cert.der().clone();
        let fingerprint: [u8; 32] = Sha256::digest(cert.as_ref()).into();
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(identity.key_pair.serialize_der()));
        let server =
            ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_protocol_versions(&[&rustls::version::TLS13])
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(vec![cert], key)
                .unwrap();
        let client =
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_protocol_versions(&[&rustls::version::TLS13])
                .unwrap()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(PinnedServer {
                    pins: vec![if pin_ok { fingerprint } else { [0; 32] }],
                }))
                .with_no_client_auth();
        (Arc::new(server), Arc::new(client))
    }
    struct FakeSink(Arc<AtomicUsize>);
    impl Sink for FakeSink {
        fn push(&mut self, jpeg: &[u8]) -> Result<(), String> {
            assert_eq!(jpeg, &[0xff, 0xd8, 0xff, 0xd9]);
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn stop(&mut self) {}
    }
    #[test]
    fn pinned_tls_and_pairing_deliver_bounded_frame_and_close() {
        let (server, client) = configurations(true);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let frames = Arc::new(AtomicUsize::new(0));
        let seen = frames.clone();
        let worker = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            receive_connection(
                stream,
                server,
                &[7; 32],
                &AtomicBool::new(false),
                &std::sync::Mutex::new(None),
                &std::sync::Mutex::new(()),
                |_| Ok(Box::new(FakeSink(seen))),
            )
        });
        let socket = TcpStream::connect(address).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut stream = StreamOwned::new(
            ClientConnection::new(client, ServerName::try_from("ponte-desk.local").unwrap())
                .unwrap(),
            socket,
        );
        let mut challenge = [0; 32];
        stream.read_exact(&mut challenge).unwrap();
        let header = encode_header(Format {
            width: 1280,
            height: 720,
            fps: 30,
        });
        stream.write_all(&header).unwrap();
        stream
            .write_all(&proof(&[7; 32], &challenge, &header))
            .unwrap();
        stream.flush().unwrap();
        let mut ready = [0; 5];
        stream.read_exact(&mut ready).unwrap();
        assert_eq!(&ready, b"READY");
        stream.write_all(&4u32.to_be_bytes()).unwrap();
        stream.write_all(&1u64.to_be_bytes()).unwrap();
        stream.write_all(&[0xff, 0xd8, 0xff, 0xd9]).unwrap();
        stream.write_all(&[0u8; 12]).unwrap();
        stream.flush().unwrap();
        assert!(worker.join().unwrap().is_ok());
        assert_eq!(frames.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn unpaired_tls_server_is_rejected_before_camera_open() {
        let (server, client) = configurations(false);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let opened = Arc::new(AtomicUsize::new(0));
        let seen = opened.clone();
        let worker = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            receive_connection(
                stream,
                server,
                &[7; 32],
                &AtomicBool::new(false),
                &std::sync::Mutex::new(None),
                &std::sync::Mutex::new(()),
                |_| {
                    seen.fetch_add(1, Ordering::SeqCst);
                    Ok(Box::new(FakeSink(Arc::new(AtomicUsize::new(0)))))
                },
            )
        });
        let mut stream = StreamOwned::new(
            ClientConnection::new(client, ServerName::try_from("ponte-desk.local").unwrap())
                .unwrap(),
            TcpStream::connect(address).unwrap(),
        );
        let mut challenge = [0; 32];
        assert!(stream.read_exact(&mut challenge).is_err());
        drop(stream);
        assert!(worker.join().unwrap().is_err());
        assert_eq!(opened.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn invalid_pairing_key_cannot_open_a_camera() {
        let (server, client) = configurations(true);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let opened = Arc::new(AtomicUsize::new(0));
        let seen = opened.clone();
        let worker = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            receive_connection(
                stream,
                server,
                &[7; 32],
                &AtomicBool::new(false),
                &std::sync::Mutex::new(None),
                &std::sync::Mutex::new(()),
                |_| {
                    seen.fetch_add(1, Ordering::SeqCst);
                    Ok(Box::new(FakeSink(Arc::new(AtomicUsize::new(0)))))
                },
            )
        });
        let mut stream = StreamOwned::new(
            ClientConnection::new(client, ServerName::try_from("ponte-desk.local").unwrap())
                .unwrap(),
            TcpStream::connect(address).unwrap(),
        );
        let mut challenge = [0; 32];
        stream.read_exact(&mut challenge).unwrap();
        let header = encode_header(Format {
            width: 1280,
            height: 720,
            fps: 30,
        });
        stream.write_all(&header).unwrap();
        stream
            .write_all(&proof(&[8; 32], &challenge, &header))
            .unwrap();
        stream.flush().unwrap();
        assert!(worker.join().unwrap().is_err());
        assert_eq!(opened.load(Ordering::SeqCst), 0);
    }
}
