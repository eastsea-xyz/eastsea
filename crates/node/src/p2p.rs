//! Validator p2p setup shared by the node and the DKG ceremony.
//!
//! Commonware `lookup` p2p listens on loopback. Over iroh (default) each remote
//! validator is a local link port whose TCP streams ride QUIC to that validator
//! (found by node id in the Mainline DHT). With explicit `--peers`, plain TCP.
//!
//! Binding guarantee: validator content signing checks immediately before each
//! signature. Retained transport keys instead have a startup check and a shared
//! one-second lifetime monitor: once verification is available, a proven
//! mismatch exits 15 even during catch-up with no content signing. Handshakes
//! and routing publications can still sign between checks, or while hardware
//! verification is unavailable. They authenticate transport only, never a vote,
//! beacon, transaction or ownership request (see the domain-separation test).
//!
//! The pinned Commonware generic signer hook requires implementing
//! `commonware_math::algebra::Random` (cryptography 2026.9.0::Signer), which
//! the node's direct dependencies do not publicly expose; wrapping it here
//! would expand the dependency surface. iroh 1.2.0's endpoint::Builder::secret_key
//! takes a concrete SecretKey and tls/resolver.rs::IrohSecretKey clones it into
//! a private TLS signer. iroh-mainline-address-lookup 0.5.0::publish likewise
//! signs its packet with a concrete SecretKey. These iroh APIs have no binding hook.
//! Keep this narrower transport guarantee explicit until signing hooks exist.

use crate::block::PublicKey;
use commonware_cryptography::{ed25519, Signer as _};
use commonware_p2p::{authenticated, authenticated::lookup, Address as PeerAddress};
use commonware_utils::{
    ordered::{Map, Set},
    union, TryCollect,
};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

const TRANSPORT_BINDING_INTERVAL: Duration = Duration::from_secs(1);
static TRANSPORT_BINDING_MONITOR: std::sync::OnceLock<()> = std::sync::OnceLock::new();

fn check_transport_binding(keys: &crate::roster::LocalKeys) {
    keys.check_binding();
    let Some(guard) = keys.binding.clone() else { return };
    // The CLI has one persisted validator identity; both public and reshare
    // transports share this monitor. Devnet/ephemeral keys must not consume it.
    TRANSPORT_BINDING_MONITOR.get_or_init(|| {
        std::thread::Builder::new()
            .name("transport-key-binding".into())
            .spawn(move || loop {
                // Unavailable verification waits with the Guard's F1 backoff
                // on this dedicated thread. It never becomes an exit-15 claim.
                guard.check_or_exit();
                std::thread::sleep(TRANSPORT_BINDING_INTERVAL);
            })
            .expect("start the persisted transport's key-binding monitor");
    });
}

#[derive(Clone, Debug)]
pub enum Transport {
    /// `<index>@<host:port>` for every other validator.
    Tcp(Vec<String>),
    /// Link to validator j listens on 127.0.0.1:(link_base + j).
    Iroh { link_base: u16 },
}

#[derive(Clone)]
pub struct P2pArgs {
    /// This validator's 1-based index in `roster`.
    pub index: u64,
    pub n: u64,
    pub roster: crate::roster::Roster,
    pub keys: crate::roster::LocalKeys,
    pub port: u16,
    pub transport: Transport,
    /// No public iroh endpoint (TCP peers only).
    pub offline: bool,
    pub max_message: u32,
}

pub fn validator_key(i: u64) -> ed25519::PrivateKey {
    aether_light::devnet_validator_key(i)
}

pub fn loopback(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
}

pub fn validators(n: u64) -> Set<PublicKey> {
    (1..=n).map(|i| validator_key(i).public_key()).try_collect().expect("unique validator keys")
}

impl P2pArgs {
    pub fn validators(&self) -> Set<PublicKey> {
        self.roster.validators()
    }
}

/// The socket address Commonware dials for every validator.
pub fn peer_addresses(a: &P2pArgs) -> Map<PublicKey, PeerAddress> {
    let mut peers: Vec<(PublicKey, PeerAddress)> = vec![(a.keys.signer.public_key(), PeerAddress::Symmetric(loopback(a.port)))];
    match &a.transport {
        Transport::Tcp(list) => {
            for p in list.iter().filter(|s| !s.is_empty()) {
                let (i, addr) = p.split_once('@').expect("peer is <index>@<host:port>");
                let addr: SocketAddr = addr.parse().expect("peer address");
                peers.push((a.roster.key(i.parse().expect("peer index")).clone(), PeerAddress::Symmetric(addr)));
            }
        }
        Transport::Iroh { link_base } => {
            for j in (1..=a.n).filter(|j| *j != a.index) {
                let ingress = loopback(link_base + j as u16);
                peers.push((a.roster.key(j).clone(), PeerAddress::Asymmetric { ingress: ingress.into(), egress: loopback(0) }));
            }
        }
    }
    peers.try_into().expect("unique validators")
}

/// Commonware p2p configuration for validator `a.index` under `namespace_suffix`.
pub fn config(a: &P2pArgs, namespace_suffix: &[u8]) -> lookup::Config<ed25519::PrivateKey> {
    check_transport_binding(&a.keys);
    let signer = a.keys.signer.clone();
    let max_peers = authenticated::peer_set_limit(&a.validators(), &signer.public_key());
    // Validators listen on loopback; the outside world reaches them only via iroh
    // links (or explicitly configured TCP peers).
    let listen = match a.transport {
        Transport::Tcp(_) => SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), a.port),
        Transport::Iroh { .. } => loopback(a.port),
    };
    let mut cfg = lookup::Config::local(signer, &union(aether_light::NAMESPACE, namespace_suffix), listen, max_peers, a.max_message);
    // Link traffic arrives from 127.0.0.1; identity is proven by the handshake.
    cfg.bypass_ip_check = true;
    cfg
}

/// Endpoint for a background reshare (`aether run`) and links to every other
/// participant over `aether/reshare/1`. On a running validator (`via_node`) the
/// node owns this Mac's public node id and forwards incoming reshare links to
/// us; we only dial out, from an unpublished id. Otherwise (a candidate) we
/// publish the node id ourselves and accept on `local`. Links are authenticated
/// by the p2p handshake (validator keys), not by the iroh id.
pub async fn open_reshare(a: &P2pArgs, via_node: bool, local: SocketAddr) -> Option<(aether_net::Endpoint, Option<aether_net::Router>)> {
    check_transport_binding(&a.keys);
    if a.offline {
        return None;
    }
    let (ep, router) = if via_node {
        (aether_net::bind(None, vec![]).await.ok()?, None)
    } else {
        let ep = aether_net::bind(Some(a.keys.node_secret.clone()), vec![aether_net::ALPN_RESHARE.to_vec()]).await.ok()?;
        (ep.clone(), Some(aether_net::serve_reshare(ep, local)))
    };
    if let Transport::Iroh { link_base } = a.transport {
        for j in (1..=a.n).filter(|j| *j != a.index) {
            aether_net::tunnel::Outbound::spawn_alpn(ep.clone(), a.roster.node(j), loopback(link_base + j as u16), aether_net::ALPN_RESHARE)
                .await
                .expect("bind reshare link port");
        }
    }
    Some((ep, router))
}

/// Bind the public iroh endpoint (published to the DHT) and open links to every
/// other validator. Returns None when offline.
pub async fn open_public(a: &P2pArgs) -> Option<aether_net::Endpoint> {
    open_public_tracked(a, aether_net::peers::PeerTracker::new()).await
}

pub async fn open_public_tracked(a: &P2pArgs, peers: aether_net::peers::PeerTracker) -> Option<aether_net::Endpoint> {
    check_transport_binding(&a.keys);
    if a.offline {
        return None;
    }
    let alpns = vec![aether_net::ALPN_RPC.to_vec(), aether_net::ALPN_P2P.to_vec(), aether_net::ALPN_RESHARE.to_vec(), aether_net::ALPN_APPS.to_vec()];
    let ep = match aether_net::bind_tracked(Some(a.keys.node_secret.clone()), alpns, peers).await {
        Ok(ep) => ep,
        Err(e) => {
            tracing::warn!(?e, "public endpoint unavailable");
            return None;
        }
    };
    let Transport::Iroh { link_base } = a.transport else { return Some(ep) };
    let mut outbound = Vec::new();
    for j in (1..=a.n).filter(|j| *j != a.index) {
        let link = aether_net::tunnel::Outbound::spawn(ep.clone(), a.roster.node(j), loopback(link_base + j as u16)).await.expect("bind link port");
        outbound.push((j, link));
    }
    tokio::spawn(async move {
        let mut last = String::new();
        loop {
            let mut line = Vec::new();
            for (j, l) in &outbound {
                line.push(format!("v{j}={}", l.path().await));
            }
            let line = line.join(" ");
            if line != last {
                tracing::info!(links = %line, "validator links");
                last = line;
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
    Some(ep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_codec::{DecodeExt as _, Encode as _};
    use commonware_cryptography::{handshake, Verifier as _};
    use rand::SeedableRng as _;
    use std::path::{Path, PathBuf};

    const CHILD_DIR: &str = "AETHER_TRANSPORT_BINDING_CHILD_DIR";
    const FIRST_HANDSHAKE: &str = "BOUND_TRANSPORT_FIRST_HANDSHAKE_VERIFIED";
    const AFTER_CHANGE: &str = "TRANSPORT_HANDSHAKE_AFTER_UUID_CHANGE";

    fn args() -> P2pArgs {
        P2pArgs {
            index: 1,
            n: 2,
            roster: crate::roster::Roster::devnet(2),
            keys: crate::roster::LocalKeys::devnet(1),
            port: 0,
            transport: Transport::Tcp(vec![]),
            offline: true,
            max_message: 4_096,
        }
    }

    /// Complete the real Commonware handshake without sockets, relay or DHT.
    fn handshake_signature(key: &ed25519::PrivateKey, namespace: &[u8]) -> ed25519::Signature {
        let peer = validator_key(2);
        let (dial, syn) = handshake::dial_start(
            rand::rngs::StdRng::seed_from_u64(1),
            handshake::Context::new(namespace, 5, 0..10, key.clone(), peer.public_key()),
        );
        let bytes = syn.encode();
        // Syn's public codec ends with its Ed25519 signature.
        let signature = ed25519::Signature::decode(&bytes[bytes.len() - 64..]).unwrap();
        let (listen, syn_ack) = handshake::listen_start(
            rand::rngs::StdRng::seed_from_u64(2),
            handshake::Context::new(namespace, 5, 0..10, peer, key.public_key()),
            syn,
        ).expect("peer verifies the real transport handshake");
        let (ack, _, _) = handshake::dial_end(dial, syn_ack).expect("dialer verifies peer");
        let _ = handshake::listen_end(listen, ack).expect("peer verifies handshake confirmation");
        signature
    }

    #[test]
    fn runtime_binding_mismatch_stops_transport_before_reconnect() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
        let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = root.join("tmp").join(format!("transport-binding-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "p2p::tests::runtime_transport_binding_child",
                "--nocapture",
            ])
            .env(CHILD_DIR, &dir)
            .env("AETHER_TEST_PLATFORM_UUID", "MAC-A")
            .output()
            .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(FIRST_HANDSHAKE), "the bound transport must first authenticate successfully: {stderr}");
        assert_eq!(output.status.code(), Some(crate::key_binding::EXIT_KEY_ELSEWHERE), "a proven runtime mismatch must stop idle/catch-up transport within its monitor interval: {stderr}");
        assert!(!stderr.contains(AFTER_CHANGE), "a reconnect after that interval must not sign: {stderr}");
    }

    #[test]
    fn runtime_transport_binding_child() {
        let Some(dir) = std::env::var_os(CHILD_DIR) else { return };
        let dir = PathBuf::from(dir);
        let mut a = args();
        // An earlier ephemeral configuration must not consume the singleton.
        let _ = config(&a, b"_P2P");
        use sha2::{Digest as _, Sha256};
        let hash = Sha256::digest([b"eastsea.bind".as_slice(), b"MAC-A"].concat());
        std::fs::write(dir.join(crate::key_binding::BINDING_FILE), serde_json::to_vec(&serde_json::json!({
            "validator_pub": hex::encode(a.keys.signer.public_key().as_ref()),
            "platform_uuid_hash": hex::encode(hash),
            "created_at": 1,
        })).unwrap()).unwrap();
        a.keys.binding = Some(crate::key_binding::Guard::for_validator(&dir, &a.keys.signer.public_key()));
        let cfg = config(&a, b"_P2P");
        let _ = handshake_signature(&cfg.crypto, &cfg.namespace);
        eprintln!("{FIRST_HANDSHAKE}");
        std::env::set_var("AETHER_TEST_PLATFORM_UUID", "MAC-B");
        // No content signing or process guard. A retained handshake key alone
        // must not keep the CLI alive indefinitely on a proven mismatch.
        std::thread::sleep(Duration::from_secs(3));
        let _ = handshake_signature(&cfg.crypto, &cfg.namespace);
        eprintln!("{AFTER_CHANGE}");
    }

    #[test]
    fn transport_signatures_cannot_authenticate_validator_content() {
        let a = args();
        let cfg = config(&a, b"_P2P");
        let message = aether_rewards::beacons::message(9, 1, 0, &[8; 32]);
        let public = a.keys.signer.public_key();
        let handshake = handshake_signature(&cfg.crypto, &cfg.namespace);
        assert!(!public.verify(crate::beacons::NAMESPACE, &message, &handshake), "a real Commonware handshake signature is not a beacon");
        // Even asking the transport key to sign the exact content message in
        // its configured domain cannot cross into the beacon domain.
        let transport = cfg.crypto.sign(&cfg.namespace, &message);
        assert!(public.verify(&cfg.namespace, &message, &transport));
        assert!(!public.verify(crate::beacons::NAMESPACE, &message, &transport));
        let routing = a.keys.node_secret.sign(&message);
        assert!(a.keys.node_secret.public().verify(&message, &routing).is_ok());
        let routing = ed25519::Signature::decode(routing.to_bytes().as_slice()).unwrap();
        assert!(!public.verify(crate::beacons::NAMESPACE, &message, &routing), "the concrete iroh routing key cannot authorize validator content");
    }
}
