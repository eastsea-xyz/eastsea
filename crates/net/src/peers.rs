//! Current connections, observed without retaining their transport handles.
//!
//! Install a [`PeerTracker`] with `Endpoint::builder(...).hooks(tracker.clone())`
//! or [`crate::bind_tracked`]. The handshake hook covers incoming and outgoing
//! connections of every protocol, including RPC and validator tunnels.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use iroh::endpoint::{AfterHandshakeOutcome, Connection, EndpointHooks, WeakConnectionHandle};
use iroh::EndpointId;
use serde_json::{json, Value};

/// Bound the observation table even when a remote opens many connections.
const MAX_TRACKED_CONNECTIONS: usize = 4_096;

#[derive(Clone, Default)]
pub struct PeerTracker {
    connections: Arc<Mutex<HashMap<usize, TrackedConnection>>>,
}

struct TrackedConnection {
    handle: WeakConnectionHandle,
    node_id: EndpointId,
    connected_since: u64,
    last_seen: u64,
    received_datagrams: u64,
    path: &'static str,
}

struct PeerSnapshot {
    connected_since: u64,
    last_seen: u64,
    path: &'static str,
}

impl std::fmt::Debug for PeerTracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PeerTracker")
    }
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn selected_path(conn: &Connection) -> &'static str {
    match conn.paths().iter().find(|p| p.is_selected()) {
        Some(p) if p.is_ip() => "direct",
        Some(p) if p.is_relay() => "relay",
        _ => "unknown",
    }
}

impl PeerTracker {
    pub fn new() -> Self {
        Self::default()
    }

    fn refresh(connections: &mut HashMap<usize, TrackedConnection>, now: u64) {
        connections.retain(|_, tracked| {
            let Some(conn) = tracked.handle.upgrade() else {
                return false;
            };
            if conn.close_reason().is_some() {
                return false;
            }
            let received = conn.stats().udp_rx.datagrams;
            if received != tracked.received_datagrams {
                tracked.received_datagrams = received;
                tracked.last_seen = now.max(tracked.last_seen);
            }
            tracked.path = selected_path(&conn);
            true
        });
    }

    fn observe(&self, conn: &Connection) {
        let now = unix_seconds();
        let mut connections = self.connections.lock().expect("peer tracker lock");
        Self::refresh(&mut connections, now);
        let id = conn.stable_id();
        if connections.contains_key(&id) {
            return;
        }
        if connections.len() >= MAX_TRACKED_CONNECTIONS {
            if let Some(oldest) = connections
                .iter()
                .min_by_key(|(_, c)| c.last_seen)
                .map(|(id, _)| *id)
            {
                connections.remove(&oldest);
            }
        }
        connections.insert(
            id,
            TrackedConnection {
                handle: conn.weak_handle(),
                node_id: conn.remote_id(),
                connected_since: now,
                last_seen: now,
                received_datagrams: conn.stats().udp_rx.datagrams,
                path: selected_path(conn),
            },
        );
    }

    /// One row per connected node, deduplicating parallel protocol connections.
    /// `last_seen` is when a received packet was last observed by this tracker;
    /// the selected path is re-read on every snapshot. Times are Unix seconds.
    /// The node may fill validator roles from its pinned roster; individual
    /// build versions are no longer advertised by the presence protocol.
    /// Addresses and relay URLs never enter the returned rows.
    pub fn snapshot(&self) -> Vec<Value> {
        let mut connections = self.connections.lock().expect("peer tracker lock");
        Self::refresh(&mut connections, unix_seconds());
        let mut peers: HashMap<EndpointId, PeerSnapshot> = HashMap::new();
        for c in connections.values() {
            let p = peers.entry(c.node_id).or_insert(PeerSnapshot {
                connected_since: c.connected_since,
                last_seen: c.last_seen,
                path: c.path,
            });
            p.connected_since = p.connected_since.min(c.connected_since);
            p.last_seen = p.last_seen.max(c.last_seen);
            // Multiple connections can select different paths. Report direct
            // if any is direct, otherwise relay if any is relayed.
            if c.path == "direct" || (p.path == "unknown" && c.path == "relay") {
                p.path = c.path;
            }
        }
        let mut rows: Vec<_> = peers
            .into_iter()
            .map(|(id, p)| {
                json!({
                    "node_id": hex::encode(id.as_bytes()),
                    "role": null,
                    "version": null,
                    "connected_since": p.connected_since,
                    "last_seen": p.last_seen,
                    "path": p.path,
                })
            })
            .collect();
        rows.sort_by(|a, b| a["node_id"].as_str().cmp(&b["node_id"].as_str()));
        rows
    }

    /// Connected endpoint ids for presence gossip, without parsing RPC rows.
    pub fn connected_ids(&self) -> Vec<EndpointId> {
        let mut connections = self.connections.lock().expect("peer tracker lock");
        Self::refresh(&mut connections, unix_seconds());
        let mut ids: Vec<_> = connections.values().map(|c| c.node_id).collect();
        ids.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        ids.dedup();
        ids
    }
}

impl EndpointHooks for PeerTracker {
    fn after_handshake<'a>(
        &'a self,
        conn: &'a Connection,
    ) -> impl Future<Output = AfterHandshakeOutcome> + Send + 'a {
        async move {
            self.observe(conn);
            AfterHandshakeOutcome::accept()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::{bind_local, EndpointAddr, SecretKey, TransportAddr, ALPN_P2P, ALPN_RPC};

    /// Both directions, different ALPNs, and close-on-drop are exercised over
    /// actual QUIC connections. Every socket is loopback; no relay or lookup.
    #[tokio::test]
    async fn tracks_both_directions_deduplicates_and_drops_closed_peers() {
        tokio::time::timeout(Duration::from_secs(10), async {
            let server_peers = PeerTracker::new();
            let client_peers = PeerTracker::new();
            let server = bind_local(
                SecretKey::generate(),
                "127.0.0.1:0".parse().unwrap(),
                server_peers.clone(),
            )
            .await
            .unwrap();
            let client = bind_local(
                SecretKey::generate(),
                "127.0.0.1:0".parse().unwrap(),
                client_peers.clone(),
            )
            .await
            .unwrap();
            server.set_alpns(vec![ALPN_RPC.to_vec(), ALPN_P2P.to_vec()]);
            assert!(server.bound_sockets().iter().all(|a| a.ip().is_loopback()));
            let addr = EndpointAddr::from_parts(
                server.id(),
                server.bound_sockets().into_iter().map(TransportAddr::Ip),
            );
            let accepting = tokio::spawn({
                let server = server.clone();
                async move {
                    let mut connections = Vec::new();
                    for _ in 0..2 {
                        connections.push(server.accept().await.unwrap().await.unwrap());
                    }
                    connections
                }
            });
            let mut outgoing = vec![
                client.connect(addr.clone(), ALPN_RPC).await.unwrap(),
                client.connect(addr, ALPN_P2P).await.unwrap(),
            ];
            let incoming = accepting.await.unwrap();

            for (tracker, expected_id) in
                [(&server_peers, client.id()), (&client_peers, server.id())]
            {
                let rows = tracker.snapshot();
                assert_eq!(rows.len(), 1, "parallel connections count as one Mac");
                assert_eq!(tracker.connected_ids(), vec![expected_id]);
                let row = &rows[0];
                assert_eq!(row["node_id"], hex::encode(expected_id.as_bytes()));
                assert!(row["connected_since"].as_u64().unwrap() > 0);
                assert!(
                    row["last_seen"].as_u64().unwrap() >= row["connected_since"].as_u64().unwrap()
                );
                assert!(row["role"].is_null() && row["version"].is_null());
                assert_eq!(row.as_object().unwrap().len(), 6);
                assert!(["direct", "relay", "unknown"].contains(&row["path"].as_str().unwrap()));
                let encoded = row.to_string();
                assert!(
                    !encoded.contains("127.0.0.1")
                        && !encoded.contains("http")
                        && !encoded.contains("::1")
                );
            }

            // Put last_seen behind its known RX counter, then deliver new
            // stream data. A snapshot must observe it, rather than keeping the
            // timestamp frozen at the handshake.
            {
                let mut tracked = server_peers.connections.lock().unwrap();
                for c in tracked.values_mut() {
                    c.received_datagrams = c.handle.upgrade().unwrap().stats().udp_rx.datagrams;
                    c.last_seen = 0;
                }
            }
            let (received, echoed) = tokio::join!(
                async {
                    let (mut send, mut recv) = incoming[0].accept_bi().await.unwrap();
                    let bytes = recv.read_to_end(16).await.unwrap();
                    send.write_all(&bytes).await.unwrap();
                    send.finish().unwrap();
                    bytes
                },
                async {
                    let (mut send, mut recv) = outgoing[0].open_bi().await.unwrap();
                    send.write_all(b"traffic").await.unwrap();
                    send.finish().unwrap();
                    recv.read_to_end(16).await.unwrap()
                },
            );
            assert_eq!(received, echoed);
            let row = &server_peers.snapshot()[0];
            assert!(row["last_seen"].as_u64().unwrap() >= row["connected_since"].as_u64().unwrap());
            assert_eq!(row["path"], "direct");

            let first = outgoing.remove(0);
            first.close(0u32.into(), b"test complete");
            incoming[0].closed().await;
            assert_eq!(
                server_peers.snapshot().len(),
                1,
                "another live ALPN connection remains"
            );
            assert_eq!(client_peers.snapshot().len(), 1);
            drop(first);

            let closed = outgoing[0].weak_handle().closed();
            drop(outgoing);
            closed.await;
            incoming[1].closed().await;
            assert!(
                client_peers.snapshot().is_empty(),
                "the tracker did not retain the last strong handle"
            );
            assert!(server_peers.snapshot().is_empty());
            server.close().await;
            client.close().await;
        })
        .await
        .expect("local peer test completed");
    }

    #[tokio::test]
    async fn local_bind_rejects_public_or_wildcard_addresses() {
        for addr in ["0.0.0.0:0", "[::]:0", "203.0.113.1:0"] {
            assert!(bind_local(
                SecretKey::generate(),
                addr.parse().unwrap(),
                PeerTracker::new()
            )
            .await
            .is_err());
        }
    }
}
