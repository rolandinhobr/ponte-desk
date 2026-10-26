//! Explicit, read-only removable-media sharing. File bytes use the OS SMB stack.
//! This module never enters the input pipeline and never handles SMB passwords.
use serde_json::{Value, json};
use std::{
    fs,
    net::IpAddr,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Mutex, OnceLock},
    time::Duration,
};
mod native;
fn root() -> PathBuf {
    crate::webcam::root()
}
fn state(name: &str) -> Value {
    fs::read(root().join(format!("storage-{name}.json")))
        .ok()
        .filter(|b| b.len() <= 16384)
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(json!({"active":false,"phase":"idle"}))
}
fn save(name: &str, value: &Value) -> Result<(), String> {
    crate::integration::atomic_preferences(&root(), &format!("storage-{name}.json"), value)
        .map_err(|e| e.to_string())
}
fn nonce() -> Result<String, String> {
    let mut bytes = [0; 16];
    rustls::crypto::ring::default_provider()
        .secure_random
        .fill(&mut bytes)
        .map_err(|_| "Random source unavailable")?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
pub fn valid_session(s: &str) -> bool {
    s.len() == 32
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
pub fn valid_share(s: &str) -> bool {
    s.strip_prefix("PDSD_").is_some_and(valid_session)
}
fn paired(peer: &str) -> Result<IpAddr, String> {
    let address: IpAddr = peer.parse().map_err(|_| "Invalid peer address")?;
    if !address.is_ipv4() {
        return Err("Storage currently requires a paired IPv4 address".into());
    }
    let settings: Value =
        serde_json::from_slice(&fs::read(root().join("observer.json")).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    if !settings["peers"]
        .as_array()
        .is_some_and(|p| p.iter().any(|p| p.as_str() == Some(peer)))
    {
        return Err("Storage requires the paired computer".into());
    }
    Ok(address)
}
fn request(peer: &str, value: Value) -> Result<Value, String> {
    paired(peer)?;
    crate::webcam::remote_command(peer, value).map_err(|e| e.to_string())
}
pub fn status() -> Value {
    json!({"host":state("host"),"mount":state("mount")})
}
fn selected(id: &str) -> Result<Value, String> {
    if id.is_empty() || id.len() > 512 || id.chars().any(char::is_control) {
        return Err("Select a removable card".into());
    }
    native::call(&json!({"op":"list"}))?["volumes"]
        .as_array()
        .and_then(|v| v.iter().find(|v| v["id"] == id))
        .cloned()
        .ok_or("The selected card is no longer connected".into())
}
#[derive(Default)]
struct Controls {
    host: Option<Child>,
    mount: Option<Child>,
}
fn controls() -> &'static Mutex<Controls> {
    static CONTROLS: OnceLock<Mutex<Controls>> = OnceLock::new();
    CONTROLS.get_or_init(|| Mutex::new(Controls::default()))
}
fn spawn(args: &[&str]) -> Result<Child, String> {
    Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())
}
fn start_host(id: &str, peer: &str, parent: u32) -> Result<Value, String> {
    paired(peer)?;
    let volume = selected(id)?;
    let previous = state("host");
    if previous["pid"]
        .as_u64()
        .is_some_and(|p| parent_alive(p as u32))
        && ["preparing", "sharing"].contains(&previous["phase"].as_str().unwrap_or(""))
    {
        return Err("A card is already being shared".into());
    }
    let mut guard = controls()
        .lock()
        .map_err(|_| "Storage controller unavailable")?;
    if guard
        .host
        .as_mut()
        .is_some_and(|c| c.try_wait().ok() == Some(None))
    {
        return Err("A card is already being shared".into());
    }
    let session = nonce()?;
    let share = format!("PDSD_{session}");
    save(
        "host",
        &json!({"active":false,"phase":"preparing","session":session,"share":share,"peer":peer,"volume":volume,"read_only":true}),
    )?;
    guard.host = Some(spawn(&[
        "storage",
        "host",
        "--id",
        id,
        "--peer",
        peer,
        "--session",
        &session,
        "--parent",
        &parent.to_string(),
    ])?);
    Ok(json!({"ok":true,"session":session,"share":share,"phase":"preparing"}))
}
fn start_mount(peer: &str, share: &str, session: &str, op: &str) -> Result<Value, String> {
    paired(peer)?;
    if !valid_session(session) || share != format!("PDSD_{session}") {
        return Err("Invalid storage session".into());
    }
    let mut guard = controls()
        .lock()
        .map_err(|_| "Storage controller unavailable")?;
    if guard
        .mount
        .as_mut()
        .is_some_and(|c| c.try_wait().ok() == Some(None))
    {
        return Err("A mount operation is already pending".into());
    }
    if op == "mount" && state("mount")["active"] == true {
        return Err("Disconnect the mounted card first".into());
    }
    if op == "unmount" {
        let old = state("mount");
        if old["active"] == true && (old["session"] != session || old["peer"] != peer) {
            return Err("Another card is mounted; it was left unchanged".into());
        }
    }
    save(
        "mount",
        &json!({"active":op=="unmount","phase":if op=="mount"{"mounting"}else{"unmounting"},"peer":peer,"share":share,"session":session,"read_only":true}),
    )?;
    guard.mount = Some(spawn(&[
        "storage",
        op,
        "--peer",
        peer,
        "--share",
        share,
        "--session",
        session,
    ])?);
    Ok(json!({"ok":true,"session":session,"phase":"pending"}))
}
pub fn rpc(peer: IpAddr, op: &str, req: &Value) -> Result<Value, String> {
    let peer = peer.to_string();
    paired(&peer)?;
    let text = |key: &str| req[key].as_str().unwrap_or("");
    match op {
        "storage.list" => native::call(&json!({"op":"list"})),
        "storage.status" => Ok(status()),
        "storage.publish" => start_host(text("id"), &peer, std::process::id()),
        "storage.mount" | "storage.unmount" => start_mount(
            &peer,
            text("share"),
            text("session"),
            op.trim_start_matches("storage."),
        ),
        "storage.stop" => stop_host(text("session"), Some(&peer)),
        "storage.open" => {
            let s = state("mount");
            if s["peer"] != peer || s["session"] != text("session") || s["active"] != true {
                return Err("Mounted card not found".into());
            }
            native::call(&json!({"op":"open","peer":peer,"share":s["share"]}))
        }
        _ => Err("Unknown storage operation".into()),
    }
}
fn stop_host(session: &str, peer: Option<&str>) -> Result<Value, String> {
    let s = state("host");
    if !valid_session(session) || s["session"] != session || peer.is_some_and(|p| s["peer"] != p) {
        return Err("Storage session not found".into());
    }
    if s["active"] == false && s["cleanup_confirmed"] == true {
        let mut stopped = s;
        stopped["phase"] = json!("stopped");
        save("host", &stopped)?;
        return Ok(json!({"ok":true,"phase":"stopped"}));
    }
    if !s["pid"]
        .as_u64()
        .is_some_and(|pid| parent_alive(pid as u32))
    {
        return Err("The storage controller is unavailable; cleanup has not been confirmed".into());
    }
    fs::write(root().join(format!("storage-stop-{session}")), b"stop")
        .map_err(|e| e.to_string())?;
    Ok(json!({"ok":true,"phase":"stopping"}))
}
fn parent_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    #[cfg(windows)]
    {
        native::parent_alive(pid)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        false
    }
}
fn host(id: &str, peer: &str, session: &str, parent: u32) -> Result<Value, String> {
    paired(peer)?;
    if !valid_session(session) || parent < 2 || !parent_alive(parent) {
        return Err("Invalid host controller".into());
    }
    fs::create_dir_all(root()).map_err(|e| e.to_string())?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root().join("storage-host.lock"))
        .map_err(|e| e.to_string())?;
    lock.try_lock()
        .map_err(|_| "A card is already being shared")?;
    let volume = selected(id)?;
    let share = format!("PDSD_{session}");
    let mut base = json!({"active":false,"phase":"preparing","session":session,"share":share,"peer":peer,"volume":volume,"read_only":true,"pid":std::process::id()});
    save("host", &base)?;
    if let Err(error) =
        native::call(&json!({"op":"host","id":id,"peer":peer,"session":session,"share":share}))
    {
        let cleanup = native::call(&json!({"op":"stop_host"}));
        base["cleanup_confirmed"] = json!(cleanup.is_ok());
        base["active"] = json!(cleanup.is_err());
        base["phase"] = json!("error");
        base["error"] = json!(&error);
        save("host", &base)?;
        return Err(error);
    }
    base["active"] = json!(true);
    base["phase"] = json!("sharing");
    save("host", &base)?;
    let stop_file = root().join(format!("storage-stop-{session}"));
    loop {
        let parent_present = parent_alive(parent);
        let stopped = !parent_present
            || stop_file.exists()
            || native::call(&json!({"op":"host_status"})).is_err();
        if stopped {
            let _ = fs::remove_file(&stop_file);
            match native::call(&json!({"op":"stop_host"})) {
                Ok(_) => {
                    base["active"] = json!(false);
                    base["phase"] = json!("stopped");
                    base["cleanup_confirmed"] = json!(true);
                    save("host", &base)?;
                    return Ok(base);
                }
                Err(error) => {
                    base["phase"] = json!("error");
                    base["error"] = json!(&error);
                    base["cleanup_confirmed"] = json!(false);
                    save("host", &base)?;
                    if !parent_present {
                        return Err(error);
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn mount(peer: &str, share: &str, session: &str, op: &str) -> Result<Value, String> {
    paired(peer)?;
    if !valid_share(share) || share != format!("PDSD_{session}") {
        return Err("Invalid share identity".into());
    }
    let mut s = json!({"active":op=="unmount","phase":if op=="mount"{"mounting"}else{"unmounting"},"peer":peer,"share":share,"session":session,"read_only":true});
    save("mount", &s)?;
    let result = native::call(&json!({"op":op,"peer":peer,"share":share}));
    match result {
        Ok(v) => {
            s["active"] = json!(op == "mount");
            s["phase"] = json!(if op == "mount" {
                "mounted"
            } else {
                "unmounted"
            });
            s["location"] = v["location"].clone();
            save("mount", &s)?;
            Ok(s)
        }
        Err(e) => {
            s["phase"] = json!("error");
            s["error"] = json!(&e);
            save("mount", &s)?;
            Err(e)
        }
    }
}
pub fn dispatch() -> Option<i32> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some("storage") {
        return None;
    }
    let verb = args.get(1).map(String::as_str).unwrap_or("");
    let arg = |key: &str| {
        args.iter()
            .position(|a| a == key)
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
            .ok_or_else(|| format!("Missing {key}"))
    };
    let result = (|| -> Result<Value, String> {
        let remote = args.iter().any(|a| a == "--remote");
        let peer = || arg("--peer");
        match verb {
            "list" => {
                if remote {
                    request(peer()?, json!({"op":"storage.list"}))
                } else {
                    native::call(&json!({"op":"list"}))
                }
            }
            "status" => {
                if remote {
                    request(peer()?, json!({"op":"storage.status"}))
                } else {
                    Ok(status())
                }
            }
            "prepare" => native::call(&json!({"op":"prepare"})),
            "publish" => {
                if remote {
                    request(peer()?, json!({"op":"storage.publish","id":arg("--id")?}))
                } else {
                    start_host(
                        arg("--id")?,
                        peer()?,
                        arg("--parent")?.parse().map_err(|_| "Invalid parent")?,
                    )
                }
            }
            "host" => host(
                arg("--id")?,
                peer()?,
                arg("--session")?,
                arg("--parent")?.parse().map_err(|_| "Invalid parent")?,
            ),
            "mount" | "unmount" => {
                if remote {
                    request(
                        peer()?,
                        json!({"op":format!("storage.{verb}"),"share":arg("--share")?,"session":arg("--session")?}),
                    )
                } else {
                    mount(peer()?, arg("--share")?, arg("--session")?, verb)
                }
            }
            "stop" => {
                if remote {
                    request(
                        peer()?,
                        json!({"op":"storage.stop","session":arg("--session")?}),
                    )
                } else {
                    stop_host(arg("--session")?, None)
                }
            }
            "open" => {
                if remote {
                    request(
                        peer()?,
                        json!({"op":"storage.open","session":arg("--session")?}),
                    )
                } else {
                    let s = state("mount");
                    if s["session"] != arg("--session")? || s["active"] != true {
                        return Err("Mounted card not found".into());
                    }
                    native::call(&json!({"op":"open","peer":s["peer"],"share":s["share"]}))
                }
            }
            "elevated-host" => native::call(
                &json!({"op":"elevated_host","id":arg("--id")?,"peer":peer()?,"session":arg("--session")?,"parent":arg("--parent")?.parse::<u32>().map_err(|_|"Invalid parent")?}),
            ),
            _ => Err("Use storage list|publish|mount|unmount|stop|status|prepare".into()),
        }
    })();
    match result {
        Ok(v) => {
            println!("{v}");
            Some(0)
        }
        Err(e) => {
            if verb == "host" {
                if let Ok(session) = arg("--session") {
                    let mut current = state("host");
                    if current["session"] == session && current["phase"] == "preparing" {
                        current["active"] = json!(false);
                        current["cleanup_confirmed"] = json!(true);
                        current["phase"] = json!("error");
                        current["error"] = json!(&e);
                        let _ = save("host", &current);
                    }
                }
            }
            println!("{}", json!({"error":e}));
            Some(1)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_owned_namespace() {
        let s = "0123456789abcdef0123456789abcdef";
        assert!(valid_share(&format!("PDSD_{s}")));
        for bad in [
            "Public",
            "PDSD_../Public",
            "PDSD_ABCDEF0123456789ABCDEF0123456789",
            "PDSD_a",
        ] {
            assert!(!valid_share(bad));
        }
    }
}
