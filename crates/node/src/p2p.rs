//! Validator p2p setup shared by the node and the DKG ceremony.
//!
//! Commonware `lookup` p2p listens on loopback. Over iroh (default) each remote
//! validator is a local link port whose TCP streams ride QUIC to that validator
//! (found by node id in the Mainline DHT). With explicit `--peers`, plain TCP.

use crate::block::PublicKey;
use commonware_cryptography::{ed25519, Signer as _};
use commonware_p2p::{authenticated, authenticated::lookup, Address as PeerAddress};
use commonware_utils::{
    ordered::{Map, Set},
    union, TryCollect,
};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

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
    a.keys.check_binding();
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
    a.keys.check_binding();
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
    a.keys.check_binding();
    if a.offline {
        return None;
    }
    let alpns = vec![aether_net::ALPN_RPC.to_vec(), aether_net::ALPN_P2P.to_vec(), aether_net::ALPN_RESHARE.to_vec()];
    let ep = match aether_net::bind(Some(a.keys.node_secret.clone()), alpns).await {
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
