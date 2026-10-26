//! One in-process coordinator: authenticated LAN state, topology and leases.
use super::{Policy, now_ms};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::{
    collections::HashMap,
    fs, io,
    net::{SocketAddr, UdpSocket},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PlatformState {
    pub monitor_present: bool,
    pub fullscreen: bool,
    pub input: Option<u32>,
    pub input_age_ms: Option<u64>,
    pub asleep: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(flatten)]
    pub platform: PlatformState,
    pub agent: bool,
    pub manual_pause: bool,
    pub capture: bool,
    pub emulation: bool,
    pub ack: u64,
    #[serde(default)]
    pub direct_supported: bool,
    #[serde(default)]
    pub direct_active: bool,
    #[serde(default)]
    pub preferences: Option<crate::preferences::Preferences>,
}
#[derive(Clone, Deserialize)]
pub struct Settings {
    pub side: String,
    pub key: String,
    pub peers: Vec<String>,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_bind")]
    pub bind_address: String,
}
fn default_port() -> u16 {
    4243
}
fn default_bind() -> String {
    "0.0.0.0".into()
}
#[derive(Serialize, Deserialize)]
struct Body {
    v: u8,
    side: String,
    session: String,
    seq: u64,
    time_ms: u64,
    state: State,
}
#[derive(Serialize, Deserialize)]
struct Envelope {
    body: Body,
    tag: String,
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
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
struct Auth {
    key: Vec<u8>,
    side: String,
    session: String,
    seq: u64,
    seen: HashMap<String, (u64, Instant)>,
}
impl Auth {
    fn new(config: &Settings) -> io::Result<Self> {
        let key = unhex(&config.key)
            .filter(|k| k.len() == 32)
            .ok_or_else(|| io::Error::other("invalid pairing key"))?;
        // OS RNG through the crypto provider already used by the KVM transport.
        let mut random = [0u8; 16];
        rustls::crypto::ring::default_provider()
            .secure_random
            .fill(&mut random)
            .map_err(|_| io::Error::other("OS random unavailable"))?;
        Ok(Self {
            key,
            side: config.side.clone(),
            session: hex(&random),
            seq: 0,
            seen: HashMap::new(),
        })
    }
    fn encode(&mut self, state: State) -> Vec<u8> {
        self.seq += 1;
        let body = Body {
            v: 4,
            side: self.side.clone(),
            session: self.session.clone(),
            seq: self.seq,
            time_ms: now_ms(),
            state,
        };
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key).expect("HMAC key");
        mac.update(&serde_json::to_vec(&body).expect("state serialization"));
        serde_json::to_vec(&Envelope {
            body,
            tag: hex(&mac.finalize().into_bytes()),
        })
        .expect("envelope")
    }
    fn decode(&mut self, data: &[u8]) -> Option<State> {
        if data.len() > 4096 {
            return None;
        }
        let envelope: Envelope = serde_json::from_slice(data).ok()?;
        let body = envelope.body;
        let other = if self.side == "mac" { "pc" } else { "mac" };
        if body.v != 4
            || body.side != other
            || body.session.len() != 32
            || body.time_ms.abs_diff(now_ms()) > 30_000
        {
            return None;
        }
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key).ok()?;
        mac.update(&serde_json::to_vec(&body).ok()?);
        mac.verify_slice(&unhex(&envelope.tag)?).ok()?;
        self.seen
            .retain(|_, (_, seen)| seen.elapsed() < Duration::from_secs(31));
        if body.seq <= self.seen.get(&body.session).map_or(0, |(seq, _)| *seq) {
            return None;
        }
        if self.seen.len() >= 32 && !self.seen.contains_key(&body.session) {
            return None;
        }
        self.seen.insert(body.session, (body.seq, Instant::now()));
        if body
            .state
            .preferences
            .as_ref()
            .is_some_and(|p| p.validate().is_err())
        {
            return None;
        }
        Some(body.state)
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
pub fn atomic_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec(value)?)?;
    fs::rename(tmp, path)
}
pub fn lease_active(root: &Path) -> bool {
    let now = now_ms() as f64 / 1000.;
    fs::read_dir(root.join("agent-leases"))
        .into_iter()
        .flatten()
        .flatten()
        .any(|entry| {
            let Ok(data) = fs::read(entry.path()) else {
                return false;
            };
            if data.len() > 2048 {
                return false;
            }
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(&data) else {
                return false;
            };
            value["expires"]
                .as_f64()
                .is_some_and(|expiry| expiry > now && expiry <= now + 600.)
        })
}
pub fn choose_policy(
    side: &str,
    local: &State,
    peer: Option<&State>,
    age: Duration,
    _cfg: &Settings,
) -> Policy {
    let Some(peer) = peer.filter(|_| age < Duration::from_secs(6)) else {
        return Policy::local("rede local: aguardando a outra máquina");
    };
    if local.manual_pause || peer.manual_pause {
        return Policy::local("compartilhamento pausado no painel");
    }
    let (mac, pc) = if side == "mac" {
        (local, peer)
    } else {
        (peer, local)
    };
    let options = mac.preferences.clone().unwrap_or_default();
    let fixed = options.input_mode != "edges";
    let target = if options.input_mode == "to_mac" {
        "mac"
    } else {
        "pc"
    };
    let source_agent = if target == "mac" { pc.agent } else { mac.agent };
    if (local.agent || peer.agent) && (!fixed || source_agent) {
        return Policy::local("agente usando o controle: máquinas isoladas");
    }
    if fixed && (!local.direct_supported || !peer.direct_supported) {
        return Policy::local("Atualize os dois computadores para usar periféricos fixos");
    }
    if fixed {
        let receiving = side == target;
        return Policy {
            capture: !receiving && peer.emulation && !peer.capture && !local.emulation,
            emulation: receiving,
            direct: !receiving,
            reason: if receiving {
                "Periféricos do outro computador conectados aqui"
            } else if peer.emulation {
                "Teclado e mouse fixos no outro computador"
            } else {
                "Aguardando o destino liberar os periféricos"
            }
            .into(),
            expires_ms: now_ms() + 8_000,
        };
    }
    if options.protect_windows_fullscreen && pc.platform.fullscreen {
        return Policy::local("Windows em tela cheia: controle local protegido");
    }
    if !options.shared_monitor {
        let capture = !(side == "mac" && options.protect_mac_fullscreen && mac.platform.fullscreen);
        return Policy {
            capture,
            direct: false,
            emulation: true,
            reason: if capture {
                "Teclado e mouse disponíveis"
            } else {
                "Mac em tela cheia: saída protegida"
            }
            .into(),
            expires_ms: now_ms() + 8_000,
        };
    }
    if options.validate().is_err() || pc.platform.input_age_ms.is_none_or(|age| age > 25_000) {
        return Policy::local("Entrada do monitor indisponível: controle local");
    }
    let value = pc.platform.input.map(|v| v & 255);
    let (capture, emulation, reason) = if value == Some(options.mac_input) {
        if side == "mac" {
            (false, true, "Monitor compartilhado no Mac")
        } else {
            (true, false, "Monitor no Mac: envio permitido")
        }
    } else if value == Some(options.pc_input) {
        if side == "mac" && options.protect_mac_fullscreen && mac.platform.fullscreen {
            (false, true, "Mac em tela cheia: saída protegida")
        } else {
            (true, true, "Monitor compartilhado no Windows")
        }
    } else {
        return Policy::local("Entradas do monitor não identificadas: controle local");
    };
    Policy {
        capture,
        direct: false,
        emulation,
        reason: reason.into(),
        expires_ms: now_ms() + 8_000,
    }
}

pub struct Coordinator {
    pub root: PathBuf,
    pub local: State,
    config: Settings,
    socket: UdpSocket,
    auth: Auth,
    peer: Option<State>,
    peer_seen: Instant,
    learned: Option<SocketAddr>,
    last_probe: Instant,
    last_send: Instant,
    last_status: Instant,
    candidate: Policy,
    candidate_since: Instant,
    applied: Policy,
}
impl Coordinator {
    pub fn new() -> io::Result<Self> {
        let root = root();
        let config: Settings = serde_json::from_slice(&fs::read(root.join("observer.json"))?)?;
        if !["mac", "pc"].contains(&config.side.as_str()) {
            return Err(io::Error::other("invalid side"));
        }
        let auth = Auth::new(&config)?;
        let socket = UdpSocket::bind((config.bind_address.as_str(), config.port))?;
        socket.set_nonblocking(true)?;
        fs::create_dir_all(root.join("agent-leases"))?;
        let old = Instant::now() - Duration::from_secs(60);
        let options = crate::preferences::Preferences::load(&root);
        Ok(Self {
            root,
            local: State {
                preferences: Some(options),
                direct_supported: true,
                ..Default::default()
            },
            config,
            socket,
            auth,
            peer: None,
            peer_seen: old,
            learned: None,
            last_probe: old,
            last_send: old,
            last_status: old,
            candidate: Policy::local("iniciando"),
            candidate_since: Instant::now(),
            applied: Policy::local("iniciando"),
        })
    }
    pub fn refresh(&mut self) -> Policy {
        let mut data = [0u8; 4097];
        for _ in 0..32 {
            let Ok((length, addr)) = self.socket.recv_from(&mut data) else {
                break;
            };
            if let Some(state) = self.auth.decode(&data[..length]) {
                self.peer = Some(state);
                self.peer_seen = Instant::now();
                self.learned = Some(addr);
            }
        }
        #[cfg(windows)]
        if let Some(options) = self
            .peer
            .as_ref()
            .and_then(|peer| peer.preferences.as_ref())
        {
            if self.local.preferences.as_ref() != Some(options) {
                let _ = atomic_json(&self.root.join("peer-preferences.json"), options);
                self.local.preferences = Some(options.clone());
                super::windows::request_refresh();
            }
        }
        if self.last_probe.elapsed() >= Duration::from_millis(500) {
            #[cfg(not(windows))]
            {
                self.local.preferences = Some(crate::preferences::Preferences::load(&self.root));
            }
            self.local.platform = super::platform_probe();
            self.last_probe = Instant::now();
        }
        self.local.agent = lease_active(&self.root);
        self.local.manual_pause = self.root.join("manual-pause").exists()
            || !crate::preferences::Preferences::valid_file(&self.root);
        let candidate = choose_policy(
            &self.config.side,
            &self.local,
            self.peer.as_ref(),
            self.peer_seen.elapsed(),
            &self.config,
        );
        if candidate.capture != self.candidate.capture
            || candidate.emulation != self.candidate.emulation
            || candidate.reason != self.candidate.reason
        {
            self.candidate = candidate.clone();
            self.candidate_since = Instant::now();
        }
        let protecting = (!candidate.capture && self.applied.capture)
            || (!candidate.emulation && self.applied.emulation);
        if protecting
            || !(candidate.capture || candidate.emulation)
            || self.candidate_since.elapsed() >= Duration::from_secs(1)
        {
            self.applied = candidate;
        }
        if self.last_send.elapsed() >= Duration::from_millis(500) {
            let packet = self.auth.encode(self.local.clone());
            for host in &self.config.peers {
                if let Ok(ip) = host.parse::<std::net::IpAddr>() {
                    let _ = self.socket.send_to(&packet, (ip, self.config.port));
                }
            }
            if let Some(addr) = self.learned {
                let _ = self.socket.send_to(&packet, addr);
            }
            self.last_send = Instant::now();
        }
        if self.last_status.elapsed() >= Duration::from_millis(500) {
            let peer = self.peer.as_ref();
            let pc = if self.config.side == "pc" {
                Some(&self.local)
            } else {
                peer
            };
            let status = serde_json::json!({"timestamp":now_ms() as f64/1000.,"pid":std::process::id(),"side":self.config.side,"peer_paused":self.peer.as_ref().is_some_and(|p|p.manual_pause),"peer_address":self.learned.map(|a|a.ip().to_string()),
                "local":self.local,"peer_age":if peer.is_some(){Some(self.peer_seen.elapsed().as_secs_f64())}else{None},
                "peer_engine":peer.map(|p|serde_json::json!({"capture_installed":p.capture,"emulation_enabled":p.emulation,"direct_active":p.direct_active,"ack":p.ack})),
                "input":{"mode":self.local.preferences.as_ref().map(|p|p.input_mode.as_str()),"target_local":self.local.preferences.as_ref().is_some_and(|p|(p.input_mode=="to_mac"&&self.config.side=="mac")||(p.input_mode=="to_pc"&&self.config.side=="pc"))},
                "policy":[self.applied.capture,self.applied.emulation,self.applied.reason],
                "display":{"automatic":self.local.preferences.as_ref().is_some_and(|p|p.manage_displays && p.shared_monitor && p.input_mode == "edges"),"input":pc.and_then(|p|p.platform.input),"input_age_ms":pc.and_then(|p|p.platform.input_age_ms),"peer_fresh":self.peer_seen.elapsed()<Duration::from_secs(6),"mac_input":self.local.preferences.as_ref().map_or(0,|p|p.mac_input),"pc_input":self.local.preferences.as_ref().map_or(0,|p|p.pc_input)}});
            let _ = atomic_json(&self.root.join("observer-status.json"), &status);
            self.last_status = Instant::now();
        }
        self.applied.clone()
    }
    pub fn actual(&mut self, capture: bool, emulation: bool, direct_active: bool) {
        if self.local.capture != capture || self.local.emulation != emulation {
            self.last_send = Instant::now() - Duration::from_secs(1);
        }
        self.local.direct_active = direct_active;
        self.local.capture = capture;
        self.local.emulation = emulation;
        // Remote ACK echoes a monotonically increasing peer sequence only after
        // actual native hooks have been removed (not merely a desired policy).
        self.local.ack = if (self.local.agent || self.peer.as_ref().is_some_and(|p| p.agent))
            && ((!capture && !emulation)
                || self.local.preferences.as_ref().is_some_and(|p| {
                    let target = if p.input_mode == "to_mac" {
                        "mac"
                    } else {
                        "pc"
                    };
                    p.input_mode != "edges"
                        && if self.config.side == target {
                            !capture && emulation
                        } else {
                            capture && !emulation && direct_active && !self.local.agent
                        }
                })) {
            now_ms()
        } else {
            0
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cfg(side: &str) -> Settings {
        Settings {
            side: side.into(),
            key: "42".repeat(32),
            peers: vec![],
            port: 4243,
            bind_address: "0.0.0.0".into(),
        }
    }
    #[test]
    fn auth_rejects_replay_tamper_and_wrong_key() {
        let mut a = Auth::new(&cfg("mac")).unwrap();
        let mut b = Auth::new(&cfg("pc")).unwrap();
        let packet = a.encode(State::default());
        assert!(b.decode(&packet).is_some());
        assert!(b.decode(&packet).is_none());
        let mut bad = packet.clone();
        bad[20] ^= 1;
        assert!(b.decode(&bad).is_none());
        let mut wrong = cfg("pc");
        wrong.key = "41".repeat(32);
        assert!(
            Auth::new(&wrong)
                .unwrap()
                .decode(&a.encode(State::default()))
                .is_none()
        );
    }
    fn fixed_pair(target: &str) -> (State, State) {
        let mut mac = State {
            direct_supported: true,
            preferences: Some(crate::preferences::Preferences {
                input_mode: target.into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut pc = State {
            direct_supported: true,
            ..Default::default()
        };
        if target == "to_mac" {
            mac.emulation = true
        } else {
            pc.emulation = true
        }
        (mac, pc)
    }
    #[test]
    fn fixed_modes_use_one_way_input_without_monitor_metadata() {
        for mode in ["to_mac", "to_pc"] {
            let (mac, pc) = fixed_pair(mode);
            let target = if mode == "to_mac" { "mac" } else { "pc" };
            for (side, local, peer) in [("mac", &mac, &pc), ("pc", &pc, &mac)] {
                let policy = choose_policy(side, local, Some(peer), Duration::ZERO, &cfg(side));
                assert_eq!(
                    (policy.capture, policy.emulation, policy.direct),
                    if side == target {
                        (false, true, false)
                    } else {
                        (true, false, true)
                    }
                );
            }
        }
    }
    #[test]
    fn target_agent_keeps_the_physical_route_but_source_agent_isolates_it() {
        let (mut mac, mut pc) = fixed_pair("to_mac");
        mac.agent = true;
        assert!(choose_policy("pc", &pc, Some(&mac), Duration::ZERO, &cfg("pc")).capture);
        assert!(choose_policy("mac", &mac, Some(&pc), Duration::ZERO, &cfg("mac")).emulation);
        pc.agent = true;
        assert!(!choose_policy("pc", &pc, Some(&mac), Duration::ZERO, &cfg("pc")).capture);
        assert!(!choose_policy("mac", &mac, Some(&pc), Duration::ZERO, &cfg("mac")).emulation);
    }
    #[test]
    fn fixed_input_requires_new_peer_readiness_and_preserves_safety_stops() {
        let (mac, mut pc) = fixed_pair("to_mac");
        pc.direct_supported = false;
        assert!(!choose_policy("mac", &mac, Some(&pc), Duration::ZERO, &cfg("mac")).emulation);
        pc.direct_supported = true;
        pc.manual_pause = true;
        assert!(!choose_policy("pc", &pc, Some(&mac), Duration::ZERO, &cfg("pc")).capture);
        pc.manual_pause = false;
        pc.platform.fullscreen = true;
        assert!(choose_policy("pc", &pc, Some(&mac), Duration::ZERO, &cfg("pc")).capture);
        pc.platform.fullscreen = false;
        assert!(!choose_policy("pc", &pc, Some(&mac), Duration::from_secs(7), &cfg("pc")).capture);
        let mut not_ready = mac;
        not_ready.emulation = false;
        assert!(!choose_policy("pc", &pc, Some(&not_ready), Duration::ZERO, &cfg("pc")).capture);
    }
    #[test]
    fn routing_and_protections_use_ddc_not_connected_cable() {
        let cfg = cfg("mac");
        let mut mac = State {
            preferences: Some(crate::preferences::Preferences {
                shared_monitor: true,
                manage_displays: true,
                mac_input: 19,
                pc_input: 15,
                windows_monitor: "test-monitor".into(),
                mac_monitor_uuid: "test-uuid".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        mac.platform.monitor_present = true;
        let mut pc = State::default();
        pc.platform.input = Some(15);
        pc.platform.input_age_ms = Some(0);
        assert!(choose_policy("mac", &mac, Some(&pc), Duration::ZERO, &cfg).capture);
        pc.platform.asleep = true;
        // An idle Windows display must not prevent shared physical input from waking it.
        assert!(choose_policy("mac", &mac, Some(&pc), Duration::ZERO, &cfg).capture);
        pc.platform.input = Some(19);
        assert!(!choose_policy("mac", &mac, Some(&pc), Duration::ZERO, &cfg).capture);
        assert!(choose_policy("mac", &mac, Some(&pc), Duration::ZERO, &cfg).emulation);
        pc.agent = true;
        assert!(!choose_policy("mac", &mac, Some(&pc), Duration::ZERO, &cfg).emulation);
        pc.agent = false;
        pc.platform.fullscreen = true;
        assert!(!choose_policy("mac", &mac, Some(&pc), Duration::ZERO, &cfg).emulation);
        assert!(!choose_policy("mac", &mac, Some(&pc), Duration::from_secs(7), &cfg).emulation);
    }
    #[test]
    fn independent_displays_work_without_ddc_and_keep_safety_invariants() {
        let cfg = cfg("mac");
        let mut mac = State::default();
        let mut pc = State::default();
        assert!(choose_policy("mac", &mac, Some(&pc), Duration::ZERO, &cfg).capture);
        assert!(choose_policy("pc", &pc, Some(&mac), Duration::ZERO, &cfg).emulation);
        pc.agent = true;
        assert!(!choose_policy("mac", &mac, Some(&pc), Duration::ZERO, &cfg).capture);
        pc.agent = false;
        pc.platform.fullscreen = true;
        assert!(!choose_policy("mac", &mac, Some(&pc), Duration::ZERO, &cfg).capture);
        mac.preferences = Some(crate::preferences::Preferences {
            protect_windows_fullscreen: false,
            ..Default::default()
        });
        assert!(choose_policy("mac", &mac, Some(&pc), Duration::ZERO, &cfg).capture);
        assert!(!choose_policy("mac", &mac, Some(&pc), Duration::from_secs(7), &cfg).capture);
    }
}
