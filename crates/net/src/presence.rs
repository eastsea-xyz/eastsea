//! Versioned, bounded transport for signed live-presence packets.
//!
//! Packet verification and table management belong to the node. This module
//! limits transport work before handing any received bytes to that verifier.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use iroh::endpoint::{Connection, RecvStream, SendStream};
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::{Endpoint, EndpointAddr, EndpointId};

use crate::RpcGate;

/// A separate ALPN lets nodes without presence support reject it safely.
pub const ALPN_PRESENCE: &[u8] = b"aether/presence/1";
pub const MAX_PRESENCE_MESSAGE: usize = 32 * 1024;
const PRESENCE_IO: Duration = Duration::from_secs(5);
const MAX_PRESENCE_STREAMS: usize = 64;
const PRESENCE_STREAMS_PER_PEER: usize = 2;
const PRESENCE_BURST: u32 = 4;
const PRESENCE_RATE_PER_SEC: u32 = 1;

/// The authenticated transport peer and a bounded packet, returning a bounded
/// reply. Implementations must keep verification and reply generation small.
pub type PresenceCallback = Arc<dyn Fn(EndpointId, &[u8]) -> Vec<u8> + Send + Sync>;

#[derive(Clone)]
pub struct PresenceProtocol {
    callback: PresenceCallback,
    gate: Arc<RpcGate>,
}

impl std::fmt::Debug for PresenceProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PresenceProtocol")
    }
}

impl PresenceProtocol {
    pub fn new(callback: PresenceCallback) -> Self {
        Self {
            callback,
            gate: Arc::new(RpcGate::new(
                MAX_PRESENCE_STREAMS,
                PRESENCE_STREAMS_PER_PEER,
                PRESENCE_BURST,
                PRESENCE_RATE_PER_SEC,
            )),
        }
    }
}

fn refuse(send: &mut SendStream, recv: &mut RecvStream) {
    let _ = recv.stop(0u32.into());
    let _ = send.reset(0u32.into());
}

impl ProtocolHandler for PresenceProtocol {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let remote = conn.remote_id();
        let peer = self.gate.peer(remote);
        loop {
            let Ok((mut send, mut recv)) = conn.accept_bi().await else {
                break;
            };
            // No task, read, decode or callback for an over-budget stream.
            let Some(guard) = self.gate.enter(&peer) else {
                refuse(&mut send, &mut recv);
                continue;
            };
            let callback = self.callback.clone();
            tokio::spawn(async move {
                let _guard = guard;
                let result = tokio::time::timeout(PRESENCE_IO, async {
                    let bytes = recv.read_to_end(MAX_PRESENCE_MESSAGE).await?;
                    let response = callback(remote, &bytes);
                    if response.len() > MAX_PRESENCE_MESSAGE {
                        bail!("presence reply exceeds size limit");
                    }
                    send.write_all(&response).await?;
                    send.finish()?;
                    Ok::<_, anyhow::Error>(())
                })
                .await;
                if !matches!(result, Ok(Ok(()))) {
                    refuse(&mut send, &mut recv);
                }
            });
        }
        Ok(())
    }
}

/// Exchange one presence packet. The complete connect/write/read sequence is
/// bounded by five seconds; unsupported ALPNs and oversized packets are errors
/// that gossip callers can ignore until the next ping.
pub async fn presence_exchange(
    endpoint: &Endpoint,
    remote: EndpointAddr,
    bytes: &[u8],
) -> Result<Vec<u8>> {
    if bytes.len() > MAX_PRESENCE_MESSAGE {
        bail!("presence request exceeds size limit");
    }
    tokio::time::timeout(PRESENCE_IO, async {
        let conn = endpoint
            .connect(remote, ALPN_PRESENCE)
            .await
            .map_err(|e| anyhow!("presence connect: {e}"))?;
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(bytes).await?;
        send.finish()?;
        let response = recv.read_to_end(MAX_PRESENCE_MESSAGE).await?;
        Ok(response)
    })
    .await
    .context("presence exchange timed out")?
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::{
        bind_local, peers::PeerTracker, serve_rpc, serve_with_presence, SecretKey, TransportAddr,
    };

    async fn local_endpoint() -> Endpoint {
        bind_local(
            SecretKey::generate(),
            "127.0.0.1:0".parse().unwrap(),
            PeerTracker::new(),
        )
        .await
        .unwrap()
    }

    fn local_addr(endpoint: &Endpoint) -> EndpointAddr {
        EndpointAddr::from_parts(
            endpoint.id(),
            endpoint.bound_sockets().into_iter().map(TransportAddr::Ip),
        )
    }

    async fn exchange_stream(conn: &Connection, bytes: &[u8]) -> Result<Vec<u8>> {
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(bytes).await?;
        send.finish()?;
        Ok(recv.read_to_end(MAX_PRESENCE_MESSAGE).await?)
    }

    #[tokio::test]
    async fn exchanges_multiple_streams_and_refuses_oversized_packets() {
        let server = local_endpoint().await;
        let client = local_endpoint().await;
        let addr = local_addr(&server);
        let client_id = client.id();
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let callback: PresenceCallback = Arc::new(move |peer, bytes| {
            assert_eq!(
                peer, client_id,
                "the callback receives the authenticated peer"
            );
            counted.fetch_add(1, Ordering::SeqCst);
            if bytes == b"oversized reply" {
                vec![0; MAX_PRESENCE_MESSAGE + 1]
            } else {
                bytes.to_vec()
            }
        });
        let router = serve_with_presence(server, |req| async { req }, None, None, Some(callback));
        let conn = client.connect(addr.clone(), ALPN_PRESENCE).await.unwrap();
        for bytes in [b"first".as_slice(), b"second".as_slice()] {
            assert_eq!(exchange_stream(&conn, bytes).await.unwrap(), bytes);
        }
        assert!(exchange_stream(&conn, &vec![0; MAX_PRESENCE_MESSAGE + 1])
            .await
            .is_err());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "oversized input never reached the callback"
        );
        assert!(exchange_stream(&conn, b"oversized reply").await.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert!(
            presence_exchange(&client, addr, &vec![0; MAX_PRESENCE_MESSAGE + 1])
                .await
                .is_err()
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            3,
            "outbound input is bounded before connecting"
        );
        client.close().await;
        router.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn limits_before_reading_or_serving_and_releases_stalled_permits() {
        tokio::time::timeout(Duration::from_secs(10), async {
            let server = local_endpoint().await;
            let client = local_endpoint().await;
            let addr = local_addr(&server);
            let calls = Arc::new(AtomicUsize::new(0));
            let counted = calls.clone();
            let protocol = PresenceProtocol::new(Arc::new(move |_, bytes| {
                counted.fetch_add(1, Ordering::SeqCst);
                bytes.to_vec()
            }));
            let gate = protocol.gate.clone();
            let router = crate::Router::builder(server)
                .accept(ALPN_PRESENCE, protocol)
                .spawn();
            let conn = client.connect(addr, ALPN_PRESENCE).await.unwrap();
            let mut stalled = Vec::new();
            for _ in 0..PRESENCE_STREAMS_PER_PEER {
                let (mut send, recv) = conn.open_bi().await.unwrap();
                send.write_all(b"unfinished").await.unwrap();
                stalled.push((send, recv));
            }
            while gate.global.available_permits()
                != MAX_PRESENCE_STREAMS - PRESENCE_STREAMS_PER_PEER
            {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            assert!(exchange_stream(&conn, b"over limit").await.is_err());
            assert_eq!(
                calls.load(Ordering::SeqCst),
                0,
                "the over-limit stream was never served"
            );
            for (mut send, mut recv) in stalled {
                refuse(&mut send, &mut recv);
            }
            while gate.global.available_permits() != MAX_PRESENCE_STREAMS {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            assert_eq!(
                exchange_stream(&conn, b"recovered").await.unwrap(),
                b"recovered"
            );
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            client.close().await;
            router.shutdown().await.unwrap();
        })
        .await
        .expect("local limit test completed");
    }

    #[tokio::test]
    async fn a_peer_exhausting_its_burst_is_not_served() {
        let server = local_endpoint().await;
        let client = local_endpoint().await;
        let addr = local_addr(&server);
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let protocol = PresenceProtocol::new(Arc::new(move |_, bytes| {
            counted.fetch_add(1, Ordering::SeqCst);
            bytes.to_vec()
        }));
        // Freeze the refill clock so scheduler delays cannot grant extra tokens.
        protocol.gate.peer(client.id()).bucket.lock().unwrap().last =
            std::time::Instant::now() + Duration::from_secs(60);
        let router = crate::Router::builder(server)
            .accept(ALPN_PRESENCE, protocol)
            .spawn();
        let conn = client.connect(addr, ALPN_PRESENCE).await.unwrap();
        for _ in 0..PRESENCE_BURST {
            assert_eq!(exchange_stream(&conn, b"ping").await.unwrap(), b"ping");
        }
        assert!(exchange_stream(&conn, b"ping").await.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), PRESENCE_BURST as usize);
        client.close().await;
        router.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn public_helper_works_and_old_protocols_fail_gracefully() {
        let server = local_endpoint().await;
        let client = local_endpoint().await;
        let addr = local_addr(&server);
        let router = serve_with_presence(
            server,
            |req| async { req },
            None,
            None,
            Some(Arc::new(|_, b| b.to_vec())),
        );
        assert_eq!(
            presence_exchange(&client, addr, b"ping").await.unwrap(),
            b"ping"
        );
        router.shutdown().await.unwrap();

        let old_server = local_endpoint().await;
        let old_addr = local_addr(&old_server);
        let old_router = serve_rpc(old_server, |req| async { req });
        assert!(presence_exchange(&client, old_addr, b"ping").await.is_err());
        client.close().await;
        old_router.shutdown().await.unwrap();
    }
}
