//! User-facing options, separate from private pairing identities.
use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub local_name: String,
    pub peer_name: String,
    pub input_mode: String,
    pub shared_monitor: bool,
    pub manage_displays: bool,
    pub protect_windows_fullscreen: bool,
    pub protect_mac_fullscreen: bool,
    pub webcam_enabled: bool,
    pub windows_monitor: String,
    pub mac_monitor_uuid: String,
    pub mac_input: u32,
    pub pc_input: u32,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            local_name: "Mac".into(),
            peer_name: "Windows".into(),
            input_mode: "edges".into(),
            shared_monitor: false,
            manage_displays: false,
            protect_windows_fullscreen: true,
            protect_mac_fullscreen: true,
            webcam_enabled: true,
            windows_monitor: String::new(),
            mac_monitor_uuid: String::new(),
            mac_input: 0,
            pc_input: 0,
        }
    }
}
impl Preferences {
    pub fn validate(&self) -> io::Result<()> {
        if !["edges", "to_mac", "to_pc"].contains(&self.input_mode.as_str()) {
            return Err(io::Error::other("invalid keyboard and mouse mode"));
        }
        for (text, maximum) in [
            (&self.local_name, 64),
            (&self.peer_name, 64),
            (&self.windows_monitor, 512),
            (&self.mac_monitor_uuid, 64),
        ] {
            if text.chars().count() > maximum || text.chars().any(char::is_control) {
                return Err(io::Error::other("invalid option text"));
            }
        }
        if self.mac_input > 255 || self.pc_input > 255 {
            return Err(io::Error::other("invalid monitor input"));
        }
        if self.shared_monitor
            && self.input_mode == "edges"
            && (self.windows_monitor.is_empty()
                || self.mac_monitor_uuid.is_empty()
                || self.mac_input == 0
                || self.pc_input == 0
                || self.mac_input == self.pc_input)
        {
            return Err(io::Error::other(
                "select a monitor on both computers and calibrate two distinct inputs",
            ));
        }
        Ok(())
    }
    pub fn load(root: &Path) -> Self {
        let read = |name: &str| {
            fs::read(root.join(name))
                .ok()
                .filter(|b| b.len() < 8192)
                .and_then(|b| serde_json::from_slice::<Self>(&b).ok())
                .filter(|v| v.validate().is_ok())
        };
        #[cfg(windows)]
        if let Some(options) = read("peer-preferences.json") {
            return options;
        }
        read("preferences.json").unwrap_or_default()
    }
    pub fn valid_file(root: &Path) -> bool {
        #[cfg(windows)]
        let path = if root.join("peer-preferences.json").exists() {
            root.join("peer-preferences.json")
        } else {
            root.join("preferences.json")
        };
        #[cfg(not(windows))]
        let path = root.join("preferences.json");
        if !path.exists() {
            return true;
        }
        fs::read(path)
            .ok()
            .filter(|b| b.len() < 8192)
            .and_then(|b| serde_json::from_slice::<Self>(&b).ok())
            .is_some_and(|p| p.validate().is_ok())
    }
    pub fn save(&self, root: &Path) -> io::Result<()> {
        self.validate()?;
        fs::create_dir_all(root)?;
        crate::integration::atomic_preferences(root, "preferences.json", self)
    }
}

/// Only user-facing options can cross the authenticated control channel.
pub(crate) fn patch_at(root: &Path, patch: &serde_json::Value) -> io::Result<serde_json::Value> {
    let mut value = serde_json::to_value(Preferences::load(root))?;
    let fields = patch
        .as_object()
        .ok_or_else(|| io::Error::other("options object required"))?;
    for (key, next) in fields {
        if value.get(key).is_none() {
            return Err(io::Error::other("unknown option"));
        }
        value[key] = next.clone();
    }
    let options: Preferences = serde_json::from_value(value)?;
    options.save(root)?;
    Ok(serde_json::json!({"ok":true,"options":options}))
}
pub(crate) fn rpc(op: &str, request: &serde_json::Value) -> io::Result<serde_json::Value> {
    let root = crate::integration::preferences_root();
    match op {
        "settings.show" => Ok(serde_json::to_value(Preferences::load(&root))?),
        "settings.update" => {
            #[cfg(windows)]
            return Err(io::Error::other("update the shared options on the Mac"));
            #[cfg(not(windows))]
            patch_at(&root, &request["patch"])
        }
        "settings.input" => {
            let enabled = request["enabled"]
                .as_bool()
                .ok_or_else(|| io::Error::other("enabled must be boolean"))?;
            set_input_at(&root, enabled)?;
            Ok(serde_json::json!({"ok":true}))
        }
        "settings.restore" => {
            patch_at(&root, &serde_json::json!({"manage_displays":false}))?;
            crate::integration::atomic_preferences(
                &root,
                "display-restore-request.json",
                &serde_json::json!({"time":crate::integration::now_ms()}),
            )?;
            Ok(serde_json::json!({"ok":true}))
        }
        _ => Err(io::Error::other("unsupported settings command")),
    }
}
fn set_input_at(root: &Path, enabled: bool) -> io::Result<()> {
    fs::create_dir_all(root)?;
    let path = root.join("manual-pause");
    if enabled {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    } else {
        fs::write(path, b"paused")
    }
}

pub fn dispatch() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some("settings") {
        return None;
    }
    let result = (|| -> io::Result<serde_json::Value> {
        let arg = |key: &str| args.windows(2).find(|v| v[0] == key).map(|v| v[1].as_str());
        match args.get(1).map(String::as_str) {
            Some("show") | Some("update") | Some("input") | Some("restore") => {
                let verb = args[1].as_str();
                let mut request = serde_json::json!({"op":format!("settings.{verb}")});
                if verb == "update" {
                    request["patch"] = serde_json::from_str(
                        arg("--patch").ok_or_else(|| io::Error::other("patch required"))?,
                    )?;
                }
                if verb == "input" {
                    request["enabled"] = serde_json::json!(
                        arg("--enabled")
                            .ok_or_else(|| io::Error::other("enabled required"))?
                            .parse::<bool>()
                            .map_err(io::Error::other)?
                    );
                }
                if let Some(peer) = arg("--peer") {
                    crate::webcam::remote_command(peer, request)
                } else {
                    rpc(&format!("settings.{verb}"), &request)
                }
            }
            Some("bridge") => {
                let enabled = arg("--enabled")
                    .ok_or_else(|| io::Error::other("enabled required"))?
                    .parse::<bool>()
                    .map_err(io::Error::other)?;
                let peer = arg("--peer").ok_or_else(|| io::Error::other("paired peer required"))?;
                let root = crate::integration::preferences_root();
                if !enabled {
                    set_input_at(&root, false)?;
                }
                let reply = crate::webcam::remote_command(
                    peer,
                    serde_json::json!({"op":"settings.input","enabled":enabled}),
                )?;
                if let Some(error) = reply["error"].as_str() {
                    return Err(io::Error::other(error.to_owned()));
                }
                if enabled {
                    set_input_at(&root, true)?;
                }
                Ok(serde_json::json!({"ok":true}))
            }
            Some("shutdown") => {
                let pid = arg("--pid")
                    .ok_or_else(|| io::Error::other("pid required"))?
                    .parse::<u32>()
                    .map_err(io::Error::other)?;
                if pid < 2 {
                    return Err(io::Error::other("invalid daemon pid"));
                }
                let root = crate::integration::preferences_root();
                let state: serde_json::Value =
                    serde_json::from_slice(&fs::read(root.join("engine-status.json"))?)?;
                if state["pid"] != pid {
                    return Err(io::Error::other("daemon changed"));
                }
                fs::write(root.join(format!("engine-stop-{pid}")), b"stop")?;
                Ok(serde_json::json!({"ok":true}))
            }
            Some("monitors") => match arg("--peer") {
                Some(peer) => {
                    crate::webcam::remote_command(peer, serde_json::json!({"op":"monitors"}))
                }
                None => Ok(crate::integration::monitor_catalog()),
            },
            Some("layout") => {
                let position =
                    arg("--position").ok_or_else(|| io::Error::other("position required"))?;
                if let Some(peer) = arg("--peer") {
                    crate::webcam::remote_command(
                        peer,
                        serde_json::json!({"op":"layout","position":position}),
                    )
                } else {
                    set_layout(position)
                }
            }
            _ => Err(io::Error::other(
                "use settings show, settings monitors [--peer IP], or settings layout --position left/right/top/bottom [--peer IP]",
            )),
        }
    })();
    match result {
        Ok(value) => {
            println!("{value}");
            Some(0)
        }
        Err(e) => {
            println!("{}", serde_json::json!({"error":e.to_string()}));
            Some(1)
        }
    }
}
pub(crate) fn set_layout(value: &str) -> io::Result<serde_json::Value> {
    use lan_mouse_ipc::{FrontendRequest, Position};
    let position = match value {
        "left" => Position::Left,
        "right" => Position::Right,
        "top" => Position::Top,
        "bottom" => Position::Bottom,
        _ => return Err(io::Error::other("invalid position")),
    };
    let (_, mut writer) = lan_mouse_ipc::try_connect().map_err(io::Error::other)?;
    writer.request(FrontendRequest::UpdatePosition(0, position))?;
    writer.request(FrontendRequest::SaveConfiguration)?;
    Ok(serde_json::json!({"ok":true}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_patch_preserves_pairing_and_rejects_unknown_or_mistyped_fields() {
        let root = std::env::temp_dir().join(format!("ponte-ui-options-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("observer.json"), b"preserved private pairing").unwrap();
        Preferences::default().save(&root).unwrap();
        assert!(patch_at(&root, &serde_json::json!({"key":"not an option"})).is_err());
        assert!(
            patch_at(
                &root,
                &serde_json::json!({"protect_mac_fullscreen":"false"})
            )
            .is_err()
        );
        patch_at(
            &root,
            &serde_json::json!({"webcam_enabled":false,"peer_name":"Desktop"}),
        )
        .unwrap();
        let saved = Preferences::load(&root);
        assert!(!saved.webcam_enabled);
        assert!(saved.protect_mac_fullscreen);
        assert_eq!(saved.peer_name, "Desktop");
        assert_eq!(
            fs::read(root.join("observer.json")).unwrap(),
            b"preserved private pairing"
        );
        fs::create_dir_all(root.join("agent-leases")).unwrap();
        fs::write(root.join("agent-leases/other-agent.json"), b"preserve").unwrap();
        set_input_at(&root, false).unwrap();
        assert!(root.join("manual-pause").exists());
        set_input_at(&root, true).unwrap();
        set_input_at(&root, true).unwrap();
        assert!(root.join("agent-leases/other-agent.json").exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn new_setup_does_not_require_monitor_or_ddc() {
        let p = Preferences::default();
        assert!(!p.shared_monitor);
        assert!(!p.manage_displays);
        assert!(p.protect_mac_fullscreen && p.protect_windows_fullscreen);
        p.validate().unwrap();
    }
    #[test]
    fn shared_monitor_requires_explicit_distinct_inputs_and_both_identities() {
        let mut p = Preferences {
            shared_monitor: true,
            ..Preferences::default()
        };
        assert!(p.validate().is_err());
        p.windows_monitor = "display-id".into();
        p.mac_monitor_uuid = "screen-uuid".into();
        p.mac_input = 17;
        p.pc_input = 18;
        assert!(p.validate().is_ok());
        p.pc_input = 17;
        assert!(p.validate().is_err());
    }
    #[test]
    fn malformed_options_are_not_treated_as_a_valid_setup() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("ponte-options-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        assert!(Preferences::valid_file(&root));
        fs::write(root.join("preferences.json"), b"{invalid").unwrap();
        assert!(!Preferences::valid_file(&root));
        Preferences::default().save(&root).unwrap();
        assert!(Preferences::valid_file(&root));
        fs::remove_dir_all(root).unwrap();
    }
}
