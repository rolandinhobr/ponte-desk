//! Cooperative native automation leases; no mouse or keyboard data is stored.
use super::native::{atomic_json, root};
use super::now_ms;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read},
    path::Path,
    time::{Duration, Instant},
};
fn write(root: &Path, token: &str, owner: &str, ttl: u64) -> io::Result<()> {
    fs::create_dir_all(root.join("agent-leases"))?;
    atomic_json(
        &root.join("agent-leases").join(format!("{token}.json")),
        &serde_json::json!({"owner":owner,"expires":now_ms() as f64/1000.+ttl.clamp(2,600) as f64}),
    )
}
fn isolated(root: &Path, since: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(7);
    while Instant::now() < deadline {
        let read = |name| {
            fs::read(root.join(name))
                .ok()
                .and_then(|s| serde_json::from_slice::<serde_json::Value>(&s).ok())
        };
        if let (Some(engine), Some(observer)) =
            (read("engine-status.json"), read("observer-status.json"))
        {
            let recent = observer["timestamp"]
                .as_f64()
                .is_some_and(|t| (now_ms() as f64 / 1000. - t).abs() < 2.);
            let local = engine["timestamp_ms"]
                .as_u64()
                .is_some_and(|t| t + 1000 >= since)
                && engine["capture_installed"] == false
                && engine["emulation_enabled"] == false;
            let peer_age = observer["peer_age"].as_f64();
            let peer = &observer["peer_engine"];
            let remote = peer["capture_installed"] == false
                && peer["emulation_enabled"] == false
                && peer["ack"].as_u64().unwrap_or(0) > 0;
            // An unreachable peer cannot carry input here: both coordinators
            // fail closed on timeout. Wait the full timeout before proceeding.
            let offline = peer_age.is_none_or(|a| a > 6.) && now_ms() > since + 6500;
            if recent && local && (remote || offline) {
                return true;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}
fn run(args: &[String]) -> Result<(), String> {
    let root = root();
    let cmd = args.first().map(String::as_str).unwrap_or("");
    if cmd == "status" {
        println!(
            "{}",
            fs::read_to_string(root.join("observer-status.json")).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    let hook = cmd == "agent-hook";
    if cmd != "agent" && !hook {
        return Err("expected agent or agent-hook".into());
    }
    let verb = args.get(1).map(String::as_str).unwrap_or("");
    let option = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
    };
    let ttl = option("--ttl").and_then(|s| s.parse().ok()).unwrap_or(180);
    let token = if hook {
        let mut input = String::new();
        io::stdin()
            .take(1_048_576)
            .read_to_string(&mut input)
            .map_err(|e| e.to_string())?;
        let value: serde_json::Value = serde_json::from_str(&input).map_err(|e| e.to_string())?;
        let identity = value["tool_use_id"]
            .as_str()
            .or_else(|| value["session_id"].as_str())
            .unwrap_or("claude-computer");
        format!("claude{:x}", Sha256::digest(identity.as_bytes()))
    } else if ["renew", "end"].contains(&verb) {
        args.get(2).cloned().ok_or("missing lease token")?
    } else {
        let mut random = [0u8; 16];
        rustls::crypto::ring::default_provider()
            .secure_random
            .fill(&mut random)
            .map_err(|_| "OS RNG failed")?;
        random.iter().map(|b| format!("{b:02x}")).collect()
    };
    if token.len() > 80 || !token.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err("invalid lease token".into());
    }
    let lease = root.join("agent-leases").join(format!("{token}.json"));
    match verb {
        "begin" => {
            let since = now_ms();
            write(
                &root,
                &token,
                option("--owner").map(String::as_str).unwrap_or("agent"),
                if hook { 120 } else { ttl },
            )
            .map_err(|e| e.to_string())?;
            if !hook {
                println!("{token}");
            }
            if !isolated(&root, since) {
                return Err("Não foi possível confirmar isolamento nas duas máquinas; não inicie Computer Use.".into());
            }
        }
        "renew" => {
            let value: serde_json::Value =
                serde_json::from_slice(&fs::read(&lease).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            write(
                &root,
                &token,
                value["owner"].as_str().unwrap_or("agent"),
                ttl,
            )
            .map_err(|e| e.to_string())?;
        }
        "end" => {
            if hook {
                write(&root, &token, "claude-computer", 3).map_err(|e| e.to_string())?;
            } else if lease.exists() {
                fs::remove_file(lease).map_err(|e| e.to_string())?;
            }
        }
        _ => return Err("use begin, renew TOKEN or end TOKEN".into()),
    }
    Ok(())
}
pub fn dispatch() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args
        .first()
        .is_some_and(|s| ["agent", "agent-hook", "status"].contains(&s.as_str()))
    {
        return None;
    }
    Some(match run(&args) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("{err}");
            2
        }
    })
}
