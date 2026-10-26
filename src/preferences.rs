//! User-facing options, separate from private pairing identities.
use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub local_name: String,
    pub peer_name: String,
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

pub fn dispatch() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some("settings") {
        return None;
    }
    let result = (|| -> io::Result<serde_json::Value> {
        let arg = |key: &str| args.windows(2).find(|v| v[0] == key).map(|v| v[1].as_str());
        match args.get(1).map(String::as_str) {
            Some("show") => Ok(serde_json::to_value(Preferences::load(
                &crate::integration::preferences_root(),
            ))?),
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
