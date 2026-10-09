//! One bounded JSON-RPC read per iroh bidirectional stream. On wasm32-unknown-
//! unknown, iroh 1.2 uses browser WebSockets for relays (no UDP or HTTP RPC
//! gateway). PkarrResolver verifies the signed packet against the requested
//! node key before its addresses become available to the endpoint.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
};

use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayMap, RelayMode,
    address_lookup::PkarrResolver,
    endpoint::{Connection, presets},
};
use n0_future::time::{Duration, timeout};
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

const ALPN_READ: &[u8] = b"eastsea/read/1";
const MAX_REQUEST: usize = 64 * 1024;
const MAX_RESPONSE: usize = 16 * 1024 * 1024;
const MAX_PEERS: usize = 32;
const MAX_CALLS: usize = 32;
const IO_TIMEOUT: Duration = Duration::from_secs(15);
// Public DHT HTTP bridges, from pubky/pkarr's relays.txt. These only provide
// signed routing records; chain identity always comes from the bundled pins.
const PKARR_RELAYS: &[&str] = &[
    "https://pkarr.pubky.app",
    "https://pkarr.pubky.org",
    "https://relay.pkarr.org",
];

fn fail(message: impl std::fmt::Display) -> JsError {
    JsError::new(&message.to_string())
}

fn urls(input: &str) -> Result<Vec<String>, JsError> {
    let value: Value = serde_json::from_str(input).map_err(fail)?;
    let list = match &value {
        Value::Null => return Ok(Vec::new()),
        Value::Array(list) => list,
        _ => return Err(fail("relay configuration must be a JSON array")),
    };
    if list.len() > 8 {
        return Err(fail("at most eight relay URLs are allowed"));
    }
    list.iter()
        .map(|v| {
            let url = v
                .as_str()
                .ok_or_else(|| fail("relay URL must be a string"))?;
            if url.len() > 2048 || !(url.starts_with("https://") || url.starts_with("http://")) {
                return Err(fail("relay URLs must use http or https"));
            }
            Ok(url.to_owned())
        })
        .collect()
}

fn read_method(method: &str) -> bool {
    let method = method
        .strip_prefix("eastsea_")
        .map(|s| format!("aether_{s}"))
        .unwrap_or_else(|| method.to_owned());
    matches!(
        method.as_str(),
        "aether_status"
            | "aether_getFinalized"
            | "aether_getBlock"
            | "aether_recentBlocks"
            | "aether_getReceiptProof"
            | "aether_getAccount"
            | "aether_getStorage"
            | "aether_getCodeHash"
            | "aether_readPeers"
            | "aether_presence"
    )
}

struct PeerState {
    generation: u64,
    active: usize,
    connection: Option<Connection>,
}

struct CallPermit<'a> {
    calls: &'a Cell<usize>,
    peers: &'a RefCell<HashMap<EndpointId, PeerState>>,
    node: EndpointId,
}

impl Drop for CallPermit<'_> {
    fn drop(&mut self) {
        self.calls.set(self.calls.get() - 1);
        let mut peers = self.peers.borrow_mut();
        if let Some(peer) = peers.get_mut(&self.node) {
            peer.active -= 1;
            if peer.active == 0 && peer.connection.is_none() {
                peers.remove(&self.node);
            }
        }
    }
}

/// Browser iroh endpoint with replaceable public relay and pkarr lookup lists.
/// A result still needs the light verifier before it can be displayed.
#[wasm_bindgen]
pub struct PublicReadTransport {
    endpoint: Endpoint,
    peers: RefCell<HashMap<EndpointId, PeerState>>,
    closed: Cell<bool>,
    calls: Cell<usize>,
    next_id: Cell<u32>,
}

#[wasm_bindgen]
impl PublicReadTransport {
    /// Empty lists select n0's public WebSocket relays and public pkarr DHT
    /// HTTP bridges. No discovery publisher or private gateway is installed.
    pub async fn create(
        relays_json: &str,
        pkarr_relays_json: &str,
    ) -> Result<PublicReadTransport, JsError> {
        let relays = urls(relays_json)?;
        let mut lookups = urls(pkarr_relays_json)?;
        if lookups.is_empty() {
            lookups = PKARR_RELAYS.iter().map(|s| (*s).to_owned()).collect();
        }
        let mode = if relays.is_empty() {
            RelayMode::Default
        } else {
            RelayMode::Custom(
                RelayMap::try_from_iter(relays.iter().map(String::as_str)).map_err(fail)?,
            )
        };
        let mut builder = Endpoint::builder(presets::Minimal).relay_mode(mode);
        for relay in lookups {
            builder = builder.address_lookup(PkarrResolver::builder(relay.parse().map_err(fail)?));
        }
        let endpoint = timeout(IO_TIMEOUT, builder.bind())
            .await
            .map_err(|_| fail("iroh bind timed out"))?
            .map_err(fail)?;
        Ok(Self {
            endpoint,
            peers: RefCell::new(HashMap::new()),
            closed: Cell::new(false),
            calls: Cell::new(0),
            next_id: Cell::new(0),
        })
    }

    /// `peer_json`: `{node: hex EndpointId, relay?: https URL}`. Known IDs
    /// without a relay are resolved by the signature-verifying HTTP lookups.
    pub async fn call(
        &self,
        peer_json: &str,
        method: &str,
        params_json: &str,
    ) -> Result<String, JsError> {
        if self.closed.get() {
            return Err(fail("public read transport is closed"));
        }
        if !read_method(method) {
            return Err(fail("method is not available on the read-only transport"));
        }
        if peer_json.len() > 4096 || params_json.len() > MAX_REQUEST {
            return Err(fail("public read request is too large"));
        }
        let peer: Value = serde_json::from_str(peer_json).map_err(fail)?;
        let node: EndpointId = peer["node"]
            .as_str()
            .ok_or_else(|| fail("peer node id"))?
            .trim_start_matches("0x")
            .parse()
            .map_err(fail)?;
        let mut address = EndpointAddr::new(node);
        if let Some(relay) = peer.get("relay").filter(|v| !v.is_null()) {
            let relay = relay.as_str().ok_or_else(|| fail("peer relay URL"))?;
            if relay.len() > 2048
                || !(relay.starts_with("https://") || relay.starts_with("http://"))
            {
                return Err(fail("peer relay must use http or https"));
            }
            address = address.with_relay_url(relay.parse().map_err(fail)?);
        }
        let params: Value = serde_json::from_str(params_json).map_err(fail)?;
        if !params.is_array() {
            return Err(fail("public read parameters must be an array"));
        }
        let id = self.next_id.get().wrapping_add(1);
        self.next_id.set(id);
        let request = serde_json::to_vec(
            &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
        )
        .map_err(fail)?;
        if request.len() > MAX_REQUEST {
            return Err(fail("public read request is too large"));
        }
        if self.calls.get() >= MAX_CALLS {
            return Err(fail("too many public read calls in flight"));
        }
        let (generation, existing) = {
            let mut peers = self.peers.borrow_mut();
            if !peers.contains_key(&node) && peers.len() >= MAX_PEERS {
                return Err(fail("public read peer pool is full"));
            }
            let peer = peers.entry(node).or_insert(PeerState {
                generation: 0,
                active: 0,
                connection: None,
            });
            peer.active += 1;
            (
                peer.generation,
                peer.connection
                    .as_ref()
                    .filter(|p| p.close_reason().is_none())
                    .cloned(),
            )
        };
        self.calls.set(self.calls.get() + 1);
        let _permit = CallPermit {
            calls: &self.calls,
            peers: &self.peers,
            node,
        };
        let operation = async {
            let connection = match existing {
                Some(connection) => connection,
                None => {
                    let connection = self
                        .endpoint
                        .connect(address, ALPN_READ)
                        .await
                        .map_err(fail)?;
                    if self.closed.get()
                        || self
                            .peers
                            .borrow()
                            .get(&node)
                            .is_none_or(|p| p.generation != generation)
                    {
                        connection.close(0u32.into(), b"peer dropped");
                        return Err(fail("public read peer was dropped"));
                    }
                    self.peers
                        .borrow_mut()
                        .get_mut(&node)
                        .expect("call holds peer slot")
                        .connection = Some(connection.clone());
                    connection
                }
            };
            let (mut send, mut receive) = connection.open_bi().await.map_err(fail)?;
            send.write_all(&request).await.map_err(fail)?;
            send.finish().map_err(fail)?;
            let bytes = receive.read_to_end(MAX_RESPONSE).await.map_err(fail)?;
            if self.closed.get()
                || self
                    .peers
                    .borrow()
                    .get(&node)
                    .is_none_or(|p| p.generation != generation)
            {
                return Err(fail("public read peer was dropped"));
            }
            let response: Value = serde_json::from_slice(&bytes).map_err(fail)?;
            if response["jsonrpc"].as_str() != Some("2.0") {
                return Err(fail("invalid public read JSON-RPC response"));
            }
            let error = response.get("error").filter(|v| !v.is_null());
            if response["id"] != json!(id) && !(response["id"].is_null() && error.is_some()) {
                return Err(fail("public read response has another request id"));
            }
            if let Some(error) = error {
                return Err(fail(format!(
                    "public read RPC {}: {}",
                    error["code"],
                    error["message"].as_str().unwrap_or("peer refused request")
                )));
            }
            Ok(response
                .get("result")
                .ok_or_else(|| fail("public read response has no result"))?
                .to_string())
        };
        let result = timeout(IO_TIMEOUT, operation)
            .await
            .map_err(|_| fail("public read timed out"))
            .and_then(|result| result);
        if result.is_err() {
            if let Some(connection) = self
                .peers
                .borrow_mut()
                .get_mut(&node)
                .filter(|p| p.generation == generation)
                .and_then(|p| p.connection.take())
            {
                connection.close(0u32.into(), b"read failed");
            }
        }
        result
    }

    /// Close a dropped peer, including connection attempts already in flight.
    #[wasm_bindgen(js_name = closePeer)]
    pub fn close_peer(&self, node: &str) -> Result<(), JsError> {
        let node: EndpointId = node.trim_start_matches("0x").parse().map_err(fail)?;
        let mut peers = self.peers.borrow_mut();
        if let Some(peer) = peers.get_mut(&node) {
            peer.generation = peer.generation.wrapping_add(1);
            if let Some(connection) = peer.connection.take() {
                connection.close(0u32.into(), b"peer dropped");
            }
            if peer.active == 0 {
                peers.remove(&node);
            }
        }
        Ok(())
    }

    /// Release all connections and the browser relay endpoint.
    pub async fn close(&self) {
        self.closed.set(true);
        for (_, peer) in self.peers.borrow_mut().drain() {
            if let Some(connection) = peer.connection {
                connection.close(0u32.into(), b"transport closed");
            }
        }
        self.endpoint.close().await;
    }
}
