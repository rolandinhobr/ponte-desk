//! Authenticate an existing paired peer and exchange Ping/Pong only.
//! Never sends Enter, Leave, mouse or keyboard input and never changes pairing.
use lan_mouse_proto::{MAX_EVENT_SIZE, ProtoEvent};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};
use tokio::{
    net::UdpSocket,
    time::{sleep, timeout},
};
use webrtc_dtls::{
    config::{Config, ExtendedMasterSecretType},
    conn::DTLSConn,
    crypto::Certificate,
};
use webrtc_util::Conn;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(
        args.next()
            .ok_or("usage: kvm_link_probe CONFIG_DIR PEER_IP:4242")?,
    );
    let addr: SocketAddr = args.next().ok_or("missing peer address")?.parse()?;
    let config: toml::Value = toml::from_str(&std::fs::read_to_string(root.join("config.toml"))?)?;
    let pins: HashSet<String> = config
        .get("authorized_fingerprints")
        .and_then(toml::Value::as_table)
        .ok_or("missing authorized fingerprints")?
        .keys()
        .map(|s| s.to_lowercase())
        .collect();
    let cert = Certificate::from_pem(&std::fs::read_to_string(root.join("ponte-desk.pem"))?)?;
    let socket = Arc::new(UdpSocket::bind("0.0.0.0:0").await?);
    socket.connect(addr).await?;
    // Existing peers use self-signed identities. Pin the presented leaf before
    // sending any protocol message; the DTLS handshake proves key possession.
    let cfg = Config {
        certificates: vec![cert],
        server_name: "ignored".into(),
        insecure_skip_verify: true,
        extended_master_secret: ExtendedMasterSecretType::Require,
        ..Default::default()
    };
    let conn = timeout(
        Duration::from_secs(8),
        DTLSConn::new(socket, cfg, true, None),
    )
    .await??;
    let state = conn.connection_state().await;
    let cert = state
        .peer_certificates
        .first()
        .ok_or("peer certificate missing")?;
    let fingerprint = Sha256::digest(cert)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":");
    if !pins.contains(&fingerprint) {
        return Err("peer certificate not paired".into());
    }
    let (packet, len): ([u8; MAX_EVENT_SIZE], usize) = ProtoEvent::Ping.into();
    let mut pongs = 0;
    let mut available = false;
    for _ in 0..5 {
        conn.send(&packet[..len]).await?;
        let mut buf = [0; MAX_EVENT_SIZE];
        timeout(Duration::from_secs(3), conn.recv(&mut buf)).await??;
        if let Ok(ProtoEvent::Pong(enabled)) = buf.try_into() {
            pongs += 1;
            available = enabled;
        } else {
            return Err("unexpected response to Ping".into());
        }
        sleep(Duration::from_millis(500)).await;
    }
    let _ = timeout(Duration::from_secs(1), conn.close()).await;
    println!(
        "{}",
        serde_json::json!({"paired_certificate_verified":true,"dtls_handshake":true,
        "ping_pong_round_trips":pongs,"remote_emulation_available":available,"input_sent":false})
    );
    Ok(())
}
