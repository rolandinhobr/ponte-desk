//! In-process desktop coordination and native hook suspension.
//! An external policy override is retained for isolated backend tests.
use serde::Deserialize;
pub mod agent;
#[cfg(target_os = "macos")]
mod macos;
mod native;
mod probe;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use native::PlatformState;
fn platform_probe() -> native::PlatformState {
    #[cfg(target_os = "macos")]
    {
        macos::probe()
    }
    #[cfg(windows)]
    {
        windows::probe()
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        native::PlatformState::default()
    }
}
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct Policy {
    pub capture: bool,
    #[serde(default)]
    pub direct: bool,
    pub emulation: bool,
    pub reason: String,
    pub expires_ms: u64,
}

impl Policy {
    fn local(reason: &str) -> Self {
        Self {
            capture: false,
            direct: false,
            emulation: false,
            reason: reason.into(),
            expires_ms: 0,
        }
    }
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn read_policy(path: &std::path::Path, now: u64) -> Policy {
    let Ok(data) = fs::read(path) else {
        return Policy::local("observer unavailable");
    };
    if data.len() > 4096 {
        return Policy::local("invalid observer policy");
    }
    match serde_json::from_slice::<Policy>(&data) {
        Ok(policy) if policy.expires_ms >= now && policy.expires_ms <= now + 30_000 => policy,
        _ => Policy::local("observer policy expired or invalid"),
    }
}

pub(crate) struct Guard {
    path: Option<PathBuf>,
    coordinator: Option<native::Coordinator>,
    status_root: Option<PathBuf>,
    pub policy: Policy,
    pub direct_active: bool,
    last_status: Instant,
    last_ignored_software: u64,
}

impl Guard {
    pub fn new() -> Self {
        let path = std::env::var_os("LAN_MOUSE_INTEGRATION_CONTROL").map(PathBuf::from);
        let coordinator = if path.is_none() {
            match native::Coordinator::new() {
                Ok(value) => Some(value),
                Err(err) => {
                    log::warn!("Ponte Desk coordinator: {err}");
                    None
                }
            }
        } else {
            None
        };
        let status_root = path
            .as_ref()
            .and_then(|p| p.parent().map(|p| p.to_owned()))
            .or_else(|| Some(native::root()));
        let policy = match path.as_ref() {
            Some(path) => read_policy(path, now_ms()),
            None => Policy {
                capture: false,
                direct: false,
                emulation: false,
                reason: "aguardando conexão integrada".into(),
                expires_ms: 0,
            },
        };
        Self {
            path,
            coordinator,
            status_root,
            policy,
            direct_active: false,
            last_status: Instant::now() - Duration::from_secs(6),
            last_ignored_software: 0,
        }
    }

    pub fn refresh(&mut self) -> bool {
        let policy = if let Some(coordinator) = self.coordinator.as_mut() {
            coordinator.refresh()
        } else if let Some(path) = self.path.as_ref() {
            read_policy(path, now_ms())
        } else {
            return false;
        };
        let changed = self.policy.direct != policy.direct
            || self.policy.capture != policy.capture
            || self.policy.emulation != policy.emulation
            || self.policy.reason != policy.reason;
        self.policy = policy;
        changed
    }

    pub fn write_status(&mut self, capture: bool, emulation: bool, force: bool) {
        if let Some(coordinator) = self.coordinator.as_mut() {
            coordinator.actual(capture, emulation, self.direct_active);
        }
        let Some(root) = self.status_root.as_ref() else {
            return;
        };
        let ignored = input_capture::input_origin::ignored_software_events();
        if !force
            && ignored == self.last_ignored_software
            && self.last_status.elapsed() < Duration::from_secs(5)
        {
            return;
        }
        self.last_ignored_software = ignored;
        self.last_status = Instant::now();
        let status = serde_json::json!({
            "timestamp_ms": now_ms(), "pid": std::process::id(),
            "capture_installed": capture, "emulation_enabled": emulation, "direct_active": self.direct_active,
            "reason": self.policy.reason,
            "ignored_software_events": ignored,
        });
        {
            let parent = root;
            let tmp = parent.join("engine-status.json.tmp");
            if let Ok(data) = serde_json::to_vec(&status) {
                if fs::write(&tmp, data).is_ok() {
                    let _ = fs::rename(tmp, parent.join("engine-status.json"));
                }
            }
        }
    }
}

pub(crate) fn preferences_root() -> PathBuf {
    native::root()
}
pub(crate) fn atomic_preferences(
    root: &std::path::Path,
    name: &str,
    value: &impl serde::Serialize,
) -> std::io::Result<()> {
    native::atomic_json(&root.join(name), value)
}
pub(crate) fn monitor_catalog() -> serde_json::Value {
    #[cfg(windows)]
    {
        windows::monitor_catalog()
    }
    #[cfg(target_os = "macos")]
    {
        macos::monitor_catalog()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        serde_json::json!({"monitors":[]})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_or_missing_observer_cannot_capture() {
        let path = std::env::temp_dir().join(format!("lanmouse-guard-{}.json", std::process::id()));
        assert!(!read_policy(&path, now_ms()).capture);
        fs::write(
            &path,
            br#"{"capture":true,"emulation":true,"reason":"test","expires_ms":1}"#,
        )
        .unwrap();
        assert!(!read_policy(&path, now_ms()).capture);
        let now = now_ms();
        fs::write(&path, serde_json::to_vec(&serde_json::json!({"capture":true,"emulation":true,"reason":"test","expires_ms":now+10_000})).unwrap()).unwrap();
        assert!(read_policy(&path, now).capture);
        fs::remove_file(path).unwrap();
    }
}
