use aether_core::api_guard::{generate_token, ApiGuard};
use aether_core::consensus::DagEngine;
use aether_core::crypto::ThresholdScheme;
use aether_core::execution::{BlockSTMExecutor, SequentialExecutor};
use aether_core::mempool::EncryptedMempool;
use aether_core::p2p::{
    get_local_ip, start_bootnode_discovery, start_lan_auto_discovery,
    start_nat_traversal, PeerManager,
};
use aether_core::storage::FlatStateStore;
use aether_core::types::{
    hex, Address, GossipMessage, Hash256, NodeIdentity, PeerInfo, SyncResponse, Transaction,
    TxPayload, Vertex,
};
use aether_core::vm::SmartContractEngine;
use parking_lot::{Mutex, RwLock};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DASHBOARD_HTML: &str = include_str!("../web/dashboard.html");

#[derive(Clone, Serialize, Deserialize)]
pub struct VertexDTO {
    pub author: String,
    pub round: u64,
    pub parents: Vec<String>,
    pub hash: String,
    pub is_anchor: bool,
}

pub struct NodeState {
    pub identity: NodeIdentity,
    pub my_address: Address,
    pub validators: Vec<Address>,
    pub dag: DagEngine,
    pub scheme: ThresholdScheme,
    pub mempool: EncryptedMempool,
    pub store: FlatStateStore,
    pub peer_mgr: PeerManager,
    pub round: u64,
    pub total_txs: usize,
    pub last_tps: usize,
    pub vertices_cache: Vec<VertexDTO>,
    pub round_parents: Vec<Hash256>,
    pub total_rewards: u64,
    pub local_ip: String,
    pub port: u16,
}

impl NodeState {
    pub fn new(identity: NodeIdentity, peer_mgr: PeerManager, local_ip: String, port: u16) -> Self {
        let mut validators = vec![
            Address::new(101),
            Address::new(102),
            Address::new(103),
            Address::new(104),
        ];
        if !validators.contains(&identity.address) {
            validators.push(identity.address);
        }

        let dag = DagEngine::new(validators.clone());
        let scheme = ThresholdScheme::new(3, 4, 9999);
        let mempool = EncryptedMempool::new();
        let store = FlatStateStore::new();

        let state_path = get_state_path(port);
        let (init_round, init_rewards) = if let Some((saved_round, saved_rewards)) = store.load_from_disk(&state_path) {
            println!(" \x1b[1;32m✔ [디스크 상태 복원]\x1b[0m state.json 에서 계정 및 스마트 계약 상태 복원 완료! (라운드 R{}, 누적 보상 {} AETH)", saved_round, saved_rewards);
            (saved_round, saved_rewards)
        } else {
            // Deposit initial balance to my validator & genesis accounts
            store.deposit(identity.address, 1_000_000);
            store.deposit(Address::new(101), 1_000_000);
            store.deposit(Address::new(102), 500_000);

            // Pre-deploy Genesis Smart Contracts for immediate interactivity
            let genesis_creator = Address::new(101);
            let (c1, s1) = SmartContractEngine::deploy(
                genesis_creator,
                0,
                0,
                "GenesisCounter",
                "counter",
                &[42],
            );
            store.register_contract(c1.clone());
            for (slot, val) in s1 {
                store.set_contract_slot(c1.address, slot, val);
            }

            let (c2, s2) = SmartContractEngine::deploy(
                genesis_creator,
                1,
                0,
                "AetherCommunityToken",
                "token",
                &[1_000_000],
            );
            store.register_contract(c2.clone());
            for (slot, val) in s2 {
                store.set_contract_slot(c2.address, slot, val);
            }

            let (c3, s3) = SmartContractEngine::deploy(
                genesis_creator,
                2,
                0,
                "HighYieldVault",
                "vault",
                &[5],
            );
            store.register_contract(c3.clone());
            for (slot, val) in s3 {
                store.set_contract_slot(c3.address, slot, val);
            }

            // 4. Ethereum Uniswap V2 AMM (ETH / USDC pool: 10,000 ETH, 20,000,000 USDC)
            let (c4, s4) = SmartContractEngine::deploy(
                genesis_creator,
                3,
                0,
                "UniswapV2_ETH_USDC",
                "uniswap_v2",
                &[10_000, 20_000_000],
            );
            store.register_contract(c4.clone());
            for (slot, val) in s4 {
                store.set_contract_slot(c4.address, slot, val);
            }

            // 5. Ethereum MakerDAO CDP (ETH collateral: $2,000 price, 150% min collateral ratio)
            let (c5, s5) = SmartContractEngine::deploy(
                genesis_creator,
                4,
                0,
                "MakerDAO_CDP",
                "maker_cdp",
                &[2_000, 150],
            );
            store.register_contract(c5.clone());
            for (slot, val) in s5 {
                store.set_contract_slot(c5.address, slot, val);
            }

            // 6. Solana Raydium AMM (SOL / USDC pool: 50,000 SOL, 7,500,000 USDC, 0.25% fee)
            let (c6, s6) = SmartContractEngine::deploy(
                genesis_creator,
                5,
                0,
                "Raydium_SOL_USDC",
                "raydium",
                &[50_000, 7_500_000],
            );
            store.register_contract(c6.clone());
            for (slot, val) in s6 {
                store.set_contract_slot(c6.address, slot, val);
            }

            // 7. Solana SPL Token Program (SPL USDC Token: 100,000,000 initial supply)
            let (c7, s7) = SmartContractEngine::deploy(
                genesis_creator,
                6,
                0,
                "SPL_USDC_Token",
                "spl_token",
                &[100_000_000],
            );
            store.register_contract(c7.clone());
            for (slot, val) in s7 {
                store.set_contract_slot(c7.address, slot, val);
            }

            store.set_account(
                genesis_creator,
                aether_core::types::AccountState {
                    balance: 1_000_000,
                    nonce: 7,
                },
            );

            store.save_to_disk(0, 0, &state_path);
            (0, 0)
        };

        let mut state = NodeState {
            my_address: identity.address,
            identity,
            validators,
            dag,
            scheme,
            mempool,
            store,
            peer_mgr,
            round: init_round,
            total_txs: 0,
            last_tps: 0, // measured only via /api/bench
            vertices_cache: Vec::new(),
            round_parents: Vec::new(),
            total_rewards: init_rewards,
            local_ip,
            port,
        };

        if init_round == 0 {
            // Initialize with 3 bootstrap rounds on fresh start
            for _ in 0..3 {
                state.advance_round();
            }
        }
        state
    }

    pub fn advance_round(&mut self) -> Option<Vertex> {
        self.round += 1;
        let r = self.round;
        let mut current_hashes = Vec::new();
        let anchor_idx = ((r - 1) as usize) % self.validators.len();
        let anchor_author = self.validators[anchor_idx];

        let mut round_txs = Vec::new();
        let mut my_vertex = None;

        for (idx, val) in self.validators.iter().enumerate() {
            let parents = if r == 1 {
                Vec::new()
            } else {
                self.round_parents.clone()
            };

            // Pull from mempool or create simulated transactions
            let mut txs = self.mempool.drain_batch(50);
            while txs.len() < 50 {
                let tx = Transaction {
                    id: (r * 1000 + idx as u64 * 50 + txs.len() as u64),
                    sender: Address::new(100 + (txs.len() as u64 % 4)),
                    nonce: 0,
                    payload: TxPayload::Transfer {
                        to: Address::new(200 + (txs.len() as u64 % 10)),
                        amount: 10,
                    },
                    gas_limit: 21000,
                };
                txs.push(self.scheme.encrypt(&tx));
            }

            let vertex = Vertex::new(*val, r, parents.clone(), txs.clone());
            let h = vertex.hash;
            self.dag.insert_vertex(vertex.clone());
            current_hashes.push(h);

            let is_anchor = *val == anchor_author && r > 1;
            self.vertices_cache.push(VertexDTO {
                author: val.to_hex(),
                round: r,
                parents: parents.iter().map(|p| format!("{}", p)).collect(),
                hash: format!("{}", h),
                is_anchor,
            });

            if *val == self.my_address {
                my_vertex = Some(vertex);
            }

            round_txs.extend(txs);
        }

        self.round_parents = current_hashes;

        // Decrypt and execute through Block-STM
        if !round_txs.is_empty() {
            let shares = vec![
                self.scheme.get_share(0),
                self.scheme.get_share(1),
                self.scheme.get_share(2),
            ];
            if let Ok(decrypted) = EncryptedMempool::decrypt_ordered_batch(&round_txs, &shares) {
                let (new_store, _) = BlockSTMExecutor::execute_block(&decrypted, &self.store);
                self.store = new_store;
                self.total_txs += decrypted.len();
            }
        }

        // Limit cache size to 60 vertices
        if self.vertices_cache.len() > 60 {
            let drain_count = self.vertices_cache.len() - 60;
            self.vertices_cache.drain(0..drain_count);
        }

        my_vertex
    }
}

pub struct RateLimiter {
    records: Mutex<HashMap<IpAddr, (Instant, u32, Option<Instant>)>>,
    max_per_sec: u32,
    block_duration: Duration,
}

impl RateLimiter {
    pub fn new(max_per_sec: u32, block_duration: Duration) -> Self {
        Self {
            records: Mutex::new(HashMap::new()),
            max_per_sec,
            block_duration,
        }
    }

    pub fn check_allowed(&self, ip: IpAddr) -> bool {
        if ip.is_loopback() {
            return true;
        }

        let now = Instant::now();
        let mut map = self.records.lock();

        if map.len() > 5000 {
            map.retain(|_, (start, _, blocked)| {
                now.duration_since(*start) < Duration::from_secs(60) || blocked.map_or(false, |b| now < b)
            });
        }

        let entry = map.entry(ip).or_insert((now, 0, None));

        if let Some(blocked_until) = entry.2 {
            if now < blocked_until {
                return false;
            } else {
                entry.2 = None;
                entry.0 = now;
                entry.1 = 0;
            }
        }

        if now.duration_since(entry.0) > Duration::from_secs(1) {
            entry.0 = now;
            entry.1 = 1;
            true
        } else {
            entry.1 += 1;
            if entry.1 > self.max_per_sec {
                entry.2 = Some(now + self.block_duration);
                println!(" \x1b[1;31m⚠ [보안 방어벽]\x1b[0m 비정상 트래픽 감지: IP({}) 1초당 {}회 초과 요청 -> 일시 차단!", ip, self.max_per_sec);
                false
            } else {
                true
            }
        }
    }
}

const MAX_HEADER_SIZE: usize = 64 * 1024;        // 64 KB max headers
const MAX_BODY_SIZE: usize = 1024 * 1024;         // 1 MB max body payload

fn read_http_request(stream: &mut TcpStream) -> Option<String> {
    let mut buffer = Vec::with_capacity(8192);
    let mut temp = [0u8; 4096];
    let mut header_end = None;
    let mut content_length = 0;

    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));

    while header_end.is_none() {
        let n = stream.read(&mut temp).ok()?;
        if n == 0 {
            break;
        }
        buffer.extend_from_slice(&temp[..n]);
        if buffer.len() > MAX_HEADER_SIZE {
            return None; // Exceeded header limit - potential Slowloris / DoS
        }

        if let Some(pos) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            header_end = Some(pos + 4);
            let header_str = String::from_utf8_lossy(&buffer[..pos]);
            for line in header_str.lines() {
                if line.to_lowercase().starts_with("content-length:") {
                    if let Some(len_str) = line.split(':').nth(1) {
                        content_length = len_str.trim().parse::<usize>().unwrap_or(0);
                    }
                }
            }
        }
    }

    if content_length > MAX_BODY_SIZE {
        return None; // Payload exceeds 1MB - protect memory from OOM
    }

    let h_end = header_end?;
    let body_received = buffer.len().saturating_sub(h_end);
    let mut remaining = content_length.saturating_sub(body_received);

    while remaining > 0 {
        let n = stream.read(&mut temp).ok()?;
        if n == 0 {
            break;
        }
        buffer.extend_from_slice(&temp[..n]);
        if buffer.len() > MAX_HEADER_SIZE + MAX_BODY_SIZE {
            return None;
        }
        remaining = remaining.saturating_sub(n);
    }

    String::from_utf8(buffer).ok()
}

fn handle_connection(
    mut stream: TcpStream,
    state: Arc<RwLock<NodeState>>,
    rate_limiter: Arc<RateLimiter>,
    guard: Arc<ApiGuard>,
) {
    let peer_addr = stream.peer_addr().ok();
    let is_local = peer_addr.map(|a| a.ip().is_loopback()).unwrap_or(false);

    if let Some(addr) = peer_addr {
        if !rate_limiter.check_allowed(addr.ip()) {
            let resp = "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 21\r\nConnection: close\r\n\r\nRate limit exceeded.\n";
            let _ = stream.write_all(resp.as_bytes());
            return;
        }
    }

    let request = match read_http_request(&mut stream) {
        Some(r) => r,
        None => return,
    };
    let first_line = request.lines().next().unwrap_or("");
    let parts: Vec<&str> = first_line.split_whitespace().collect();

    if parts.len() < 2 {
        return;
    }

    let method = parts[0];
    let path = parts[1];

    if method == "GET" && path == "/" {
        if let Err(denial) = guard.authorize_page(&request, is_local) {
            send_status(&mut stream, denial.status_line());
            return;
        }
        let html = guard.inject_token(DASHBOARD_HTML);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{}",
            html.len(),
            html
        );
        let _ = stream.write_all(response.as_bytes());
        return;
    }

    // Peer-facing endpoints are open to remote nodes; everything else is local control.
    let peer_facing = matches!(
        path,
        "/api/p2p/handshake" | "/api/p2p/peers" | "/api/p2p/sync" | "/api/p2p/gossip"
    );
    if !peer_facing {
        if let Err(denial) = guard.authorize_local(&request, is_local) {
            send_status(&mut stream, denial.status_line());
            return;
        }
    }

    // P2P Handshake: Remote peer connects to us
    if method == "POST" && path == "/api/p2p/handshake" {
        if let Some(body_start) = request.find("\r\n\r\n") {
            let body = &request[body_start + 4..];
            if let Ok(hs) = serde_json::from_str::<aether_core::p2p::HandshakeRequest>(body.trim()) {
                let remote_ip = peer_addr
                    .map(|a| a.ip().to_string())
                    .unwrap_or_else(|| "127.0.0.1".to_string());
                let remote_endpoint = format!("{}:{}", remote_ip, hs.listen_port);

                let (my_ident, p_list) = {
                    let s = state.read();
                    if hs.identity.node_id != s.identity.node_id {
                        s.peer_mgr.add_or_update(PeerInfo {
                            node_id: hs.identity.node_id.clone(),
                            address: hs.identity.address,
                            endpoint: remote_endpoint,
                            last_seen_ms: SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as u64,
                            latency_ms: 2,
                            round: 0,
                        });
                    }
                    let p_list: Vec<String> = s.peer_mgr.list().into_iter().map(|p| p.endpoint).collect();
                    (s.identity.clone(), p_list)
                };

                let resp = aether_core::p2p::HandshakeResponse {
                    identity: my_ident,
                    known_peers: p_list,
                };
                let resp_json = serde_json::to_string(&resp).unwrap_or_default();
                send_json(&mut stream, &resp_json);
                return;
            }
        }
        send_json(&mut stream, r#"{"error":"invalid_handshake"}"#);
        return;
    }

    // P2P Connect: Request to connect to an external peer (Local only)
    if method == "POST" && path == "/api/p2p/connect" {
        if !is_local {
            send_json(&mut stream, r#"{"error":"forbidden: local_control_only"}"#);
            return;
        }
        if let Some(body_start) = request.find("\r\n\r\n") {
            let body = &request[body_start + 4..];
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(body.trim()) {
                let peer_ep = val["peer"].as_str().unwrap_or("").to_string();
                let peer_mgr = state.read().peer_mgr.clone();

                match peer_mgr.connect_peer(&peer_ep) {
                    Ok(info) => {
                        let resp = serde_json::json!({
                            "status": "connected",
                            "peer": info
                        });
                        send_json(&mut stream, &resp.to_string());
                        return;
                    }
                    Err(e) => {
                        let resp = serde_json::json!({
                            "status": "error",
                            "message": e
                        });
                        send_json(&mut stream, &resp.to_string());
                        return;
                    }
                }
            }
        }
        send_json(&mut stream, r#"{"error":"invalid_request"}"#);
        return;
    }

    // P2P Peers List
    if method == "GET" && path == "/api/p2p/peers" {
        let s = state.read();
        let peers = s.peer_mgr.list();
        let resp = serde_json::json!({
            "my_node_id": s.identity.node_id,
            "local_ip": s.local_ip,
            "port": s.port,
            "peer_count": peers.len(),
            "peers": peers
        });
        send_json(&mut stream, &resp.to_string());
        return;
    }

    // P2P Full State Sync
    if method == "GET" && path == "/api/p2p/sync" {
        let s = state.read();
        let resp = SyncResponse {
            latest_round: s.round,
            vertices: recent_vertices(&s.dag, MAX_SYNC_VERTICES),
            contracts: s.store.list_contracts(),
            contract_slots: s.store.export_all_contract_slots(),
            accounts: s.store.export_all_accounts(),
        };
        let json = serde_json::to_string(&resp).unwrap_or_default();
        send_json(&mut stream, &json);
        return;
    }

    // P2P gossip: transactions and vertices are unsigned, so accepting them would let
    // any peer spend any account or grow our DAG without bound. Ignore until
    // signed envelopes and BAL validation land (docs/design/02-types.md).
    if method == "POST" && path == "/api/p2p/gossip" {
        send_json(&mut stream, r#"{"status":"ignored","reason":"unsigned gossip is not accepted"}"#);
        return;
    }

    if method == "GET" && path == "/api/status" {
        let s = state.read();
        let balance = s.store.get_balance(&s.my_address);
        let root = format!("{}", s.store.state_root());
        let peers = s.peer_mgr.list();
        let json = serde_json::json!({
            "node_id": s.identity.node_id,
            "node_name": s.identity.name,
            "address": s.my_address.to_hex(),
            "round": s.round,
            "total_txs": s.total_txs,
            "tps": s.last_tps,
            "state_root": root,
            "balance": balance,
            "total_rewards": s.total_rewards,
            "cores": rayon::current_num_threads(),
            "node_uri": format!("aether://{}", s.identity.node_id),
            "local_ip": s.local_ip,
            "port": s.port,
            "nat": s.peer_mgr.get_nat_info(),
            "peer_count": peers.len(),
            "peers": peers
        });
        send_json(&mut stream, &json.to_string());
        return;
    }

    if method == "GET" && path == "/api/dag" {
        let s = state.read();
        let json = serde_json::json!({
            "vertices": s.vertices_cache
        });
        send_json(&mut stream, &json.to_string());
        return;
    }

    if method == "POST" && path == "/api/step" {
        if !is_local {
            send_json(&mut stream, r#"{"error":"forbidden: local_control_only"}"#);
            return;
        }
        let (round, vertex_opt, peer_mgr) = {
            let mut s = state.write();
            let v = s.advance_round();
            s.store.deposit(s.my_address, 10);
            s.total_rewards += 10;
            let sp = get_state_path(s.port);
            s.store.save_to_disk(s.round, s.total_rewards, &sp);
            (s.round, v, s.peer_mgr.clone())
        };
        if let Some(v) = vertex_opt {
            peer_mgr.broadcast_gossip(&GossipMessage::NewVertex(v));
        }
        let resp = serde_json::json!({ "status": "ok", "round": round });
        send_json(&mut stream, &resp.to_string());
        return;
    }

    if method == "GET" && path == "/api/contracts" {
        let s = state.read();
        let contracts = s.store.list_contracts();
        let mut list = Vec::new();
        for c in contracts {
            let slots = s.store.get_all_contract_slots(&c.address);
            let slots_json: serde_json::Map<String, serde_json::Value> = slots
                .into_iter()
                .map(|(k, v)| (k.to_string(), serde_json::Value::from(v)))
                .collect();

            list.push(serde_json::json!({
                "address": c.address.to_hex(),
                "name": c.name,
                "template": c.template,
                "creator": c.creator.to_hex(),
                "created_at_round": c.created_at_round,
                "slots": slots_json,
            }));
        }
        let json = serde_json::json!({ "contracts": list });
        send_json(&mut stream, &json.to_string());
        return;
    }

    if method == "POST" && path == "/api/contract/deploy" {
        if !is_local {
            send_json(&mut stream, r#"{"error":"forbidden: local_control_only"}"#);
            return;
        }
        if let Some(body_start) = request.find("\r\n\r\n") {
            let body = &request[body_start + 4..];
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(body.trim()) {
                let name = val["name"].as_str().unwrap_or("CustomContract").to_string();
                let template = val["template"].as_str().unwrap_or("counter").to_string();
                let params: Vec<u64> = val["params"]
                    .as_array()
                    .map(|arr| arr.iter().filter_map(|x| x.as_u64()).collect())
                    .unwrap_or_default();

                let (contract_addr, round, tx_to_gossip, peer_mgr) = {
                    let mut s = state.write();
                    let sender_nonce = s.store.get_account(&s.my_address).nonce;
                    let addr = SmartContractEngine::derive_contract_address(
                        &s.my_address,
                        sender_nonce,
                        &name,
                    );
                    let tx = Transaction {
                        id: s.total_txs as u64 + 1000,
                        sender: s.my_address,
                        nonce: sender_nonce,
                        payload: TxPayload::DeployContract {
                            name: name.clone(),
                            template: template.clone(),
                            params,
                        },
                        gas_limit: 100_000,
                    };
                    let enc = s.scheme.encrypt(&tx);
                    s.mempool.submit(enc);
                    s.advance_round();
                    let sp = get_state_path(s.port);
                    s.store.save_to_disk(s.round, s.total_rewards, &sp);
                    (addr, s.round, tx, s.peer_mgr.clone())
                };

                // Broadcast to connected P2P peers
                peer_mgr.broadcast_gossip(&GossipMessage::NewTx(tx_to_gossip));

                let resp = serde_json::json!({
                    "status": "deployed",
                    "contract_address": contract_addr.to_hex(),
                    "name": name,
                    "template": template,
                    "round": round,
                });
                send_json(&mut stream, &resp.to_string());
                return;
            }
        }
        send_json(&mut stream, r#"{"error":"invalid_payload"}"#);
        return;
    }

    if method == "POST" && path == "/api/contract/call" {
        if !is_local {
            send_json(&mut stream, r#"{"error":"forbidden: local_control_only"}"#);
            return;
        }
        if let Some(body_start) = request.find("\r\n\r\n") {
            let body = &request[body_start + 4..];
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(body.trim()) {
                let contract_str = val["contract"].as_str().unwrap_or("");
                let method_name = val["method"].as_str().unwrap_or("").to_string();
                let args: Vec<u64> = val["args"]
                    .as_array()
                    .map(|arr| arr.iter().filter_map(|x| x.as_u64()).collect())
                    .unwrap_or_default();

                if let Some(target_contract) = Address::from_hex(contract_str) {
                    let (round, tx_to_gossip, peer_mgr) = {
                        let mut s = state.write();
                        let sender_nonce = s.store.get_account(&s.my_address).nonce;
                        let tx = Transaction {
                            id: s.total_txs as u64 + 1000,
                            sender: s.my_address,
                            nonce: sender_nonce,
                            payload: TxPayload::CallContract {
                                contract: target_contract,
                                method: method_name.clone(),
                                args,
                            },
                            gas_limit: 50_000,
                        };
                        let enc = s.scheme.encrypt(&tx);
                        s.mempool.submit(enc);
                        s.advance_round();
                        let sp = get_state_path(s.port);
                        s.store.save_to_disk(s.round, s.total_rewards, &sp);
                        (s.round, tx, s.peer_mgr.clone())
                    };

                    // Broadcast to connected P2P peers
                    peer_mgr.broadcast_gossip(&GossipMessage::NewTx(tx_to_gossip));

                    let resp = serde_json::json!({
                        "status": "executed",
                        "contract": contract_str,
                        "method": method_name,
                        "round": round,
                    });
                    send_json(&mut stream, &resp.to_string());
                    return;
                }
            }
        }
        send_json(&mut stream, r#"{"error":"invalid_payload"}"#);
        return;
    }

    if method == "POST" && path == "/api/tx" {
        if !is_local {
            send_json(&mut stream, r#"{"error":"forbidden: local_control_only"}"#);
            return;
        }
        if let Some(body_start) = request.find("\r\n\r\n") {
            let body = &request[body_start + 4..];
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(body) {
                let amount = val["amount"].as_u64().unwrap_or(100);
                let is_swap = val["type"].as_str() == Some("swap");

                let (tx, peer_mgr) = {
                    let s = state.write();
                    let tx = Transaction {
                        id: s.total_txs as u64 + 999,
                        sender: s.my_address,
                        nonce: 0,
                        payload: if is_swap {
                            TxPayload::Swap {
                                pool_id: 1,
                                amount_in: amount,
                                min_out: amount.saturating_sub(10),
                            }
                        } else {
                            TxPayload::Transfer {
                                to: Address::new(102),
                                amount,
                            }
                        },
                        gas_limit: 21000,
                    };
                    let enc = s.scheme.encrypt(&tx);
                    s.mempool.submit(enc);
                    (tx, s.peer_mgr.clone())
                };
                peer_mgr.broadcast_gossip(&GossipMessage::NewTx(tx));
            }
        }
        send_json(&mut stream, r#"{"status":"submitted_encrypted"}"#);
        return;
    }

    if method == "POST" && path == "/api/bench" {
        if !is_local {
            send_json(&mut stream, r#"{"error":"forbidden: local_control_only"}"#);
            return;
        }
        const TX_COUNT: usize = 10_000;
        let mut rng = StdRng::seed_from_u64(42);
        let mut txs = Vec::with_capacity(TX_COUNT);
        let fresh_store = || {
            let store = FlatStateStore::new();
            for i in 0..5000 {
                store.deposit(Address::new(i), 1_000_000);
            }
            store
        };

        for i in 0..TX_COUNT {
            let is_swap = rng.gen_bool(0.20);
            let sender = Address::new(rng.gen_range(0..2500));
            let payload = if is_swap {
                TxPayload::Swap {
                    pool_id: rng.gen_range(1..=10),
                    amount_in: rng.gen_range(10..200),
                    min_out: 5,
                }
            } else {
                TxPayload::Transfer {
                    to: Address::new(rng.gen_range(2501..5000)),
                    amount: rng.gen_range(1..50),
                }
            };
            txs.push(Transaction {
                id: i as u64,
                sender,
                nonce: 0,
                payload,
                gas_limit: 21000,
            });
        }

        let seq_start = Instant::now();
        let seq_store = fresh_store();
        let _ = SequentialExecutor::execute_block(&txs, &seq_store);
        let seq_dur = seq_start.elapsed();
        let seq_tps = (TX_COUNT as f64) / seq_dur.as_secs_f64();

        let par_start = Instant::now();
        let par_store = fresh_store();
        let (_new_store, _) = BlockSTMExecutor::execute_block(&txs, &par_store);
        let par_dur = par_start.elapsed();
        let par_tps = (TX_COUNT as f64) / par_dur.as_secs_f64();

        {
            let mut s = state.write();
            s.total_txs += TX_COUNT;
            s.last_tps = par_tps as usize;
        }

        let speedup = par_tps / seq_tps;
        let json = format!(
            r#"{{"seq_tps":{:.0},"par_tps":{:.0},"speedup":{:.2}}}"#,
            seq_tps, par_tps, speedup
        );
        send_json(&mut stream, &json);
        return;
    }

    // Default 404
    let not_found = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
    let _ = stream.write_all(not_found.as_bytes());
}

fn send_json(stream: &mut TcpStream, json: &str) {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        json.len(),
        json
    );
    let _ = stream.write_all(response.as_bytes());
}

fn send_status(stream: &mut TcpStream, status_line: &str) {
    let response = format!(
        "HTTP/1.1 {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        status_line
    );
    let _ = stream.write_all(response.as_bytes());
}

const MAX_SYNC_VERTICES: usize = 256;

fn recent_vertices(dag: &DagEngine, limit: usize) -> Vec<Vertex> {
    let mut all = dag.get_all_vertices();
    all.sort_by_key(|v| std::cmp::Reverse(v.round));
    all.truncate(limit);
    all
}

fn aether_dir() -> std::path::PathBuf {
    let dir = std::env::var("HOME")
        .map(|h| std::path::PathBuf::from(h).join(".aether"))
        .unwrap_or_else(|_| std::path::PathBuf::from(".aether"));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Write the per-run control token readable only by the current user.
fn write_token_file(token: &str) -> std::io::Result<std::path::PathBuf> {
    let path = aether_dir().join("token");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&path)?;
    f.write_all(token.as_bytes())?;
    Ok(path)
}

pub fn get_state_path(custom_port: u16) -> std::path::PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        let dir = std::path::PathBuf::from(home).join(".aether");
        let _ = std::fs::create_dir_all(&dir);
        if custom_port != 8080 {
            dir.join(format!("state_{}.json", custom_port))
        } else {
            dir.join("state.json")
        }
    } else {
        std::path::PathBuf::from(format!("./state_{}.json", custom_port))
    }
}

fn load_or_create_identity(custom_port: u16) -> NodeIdentity {
    let home = std::env::var("HOME").ok().map(std::path::PathBuf::from);
    let path = if let Some(h) = home {
        let dir = h.join(".aether");
        let _ = std::fs::create_dir_all(&dir);
        if custom_port != 8080 {
            dir.join(format!("identity_{}.json", custom_port))
        } else {
            dir.join("identity.json")
        }
    } else {
        std::path::PathBuf::from(format!("./identity_{}.json", custom_port))
    };

    if let Ok(content) = std::fs::read_to_string(&path) {
        if let Ok(ident) = serde_json::from_str::<NodeIdentity>(&content) {
            return ident;
        }
    }

    let mut rng = StdRng::from_entropy();
    let id_num: u64 = rng.gen_range(1000..999999);
    let mut rand_bytes = [0u8; 8];
    rng.fill(&mut rand_bytes);
    let node_hex = format!("0x{}", hex::encode(&rand_bytes));
    let address = Address::new(id_num);
    let name = format!("Aether-Node-{}", &node_hex[2..6]);

    let ident = NodeIdentity {
        node_id: node_hex,
        address,
        name,
        created_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };

    if let Ok(json) = serde_json::to_string_pretty(&ident) {
        let _ = std::fs::write(&path, json);
    }
    ident
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut requested_port = 8080;
    let mut connect_peer_arg: Option<String> = None;
    let mut public = false;
    let mut open_browser = true;

    let mut i = 1;
    while i < args.len() {
        if args[i] == "--port" && i + 1 < args.len() {
            requested_port = args[i + 1].parse().unwrap_or(8080);
            i += 2;
        } else if args[i] == "--public" {
            public = true;
            i += 1;
        } else if args[i] == "--no-open" {
            open_browser = false;
            i += 1;
        } else if args[i] == "--peer" && i + 1 < args.len() {
            connect_peer_arg = Some(args[i + 1].clone());
            i += 2;
        } else {
            i += 1;
        }
    }

    println!("\x1b[1;32m");
    println!("  █████╗ ███████╗████████╗██╗  ██╗███████╗██████╗ ");
    println!(" ██╔══██╗██╔════╝╚══██╔══╝██║  ██║██╔════╝██╔══██╗");
    println!(" ███████║█████╗     ██║   ███████║█████╗  ██████╔╝");
    println!(" ██╔══██║██╔══╝     ██║   ██╔══██║██╔══╝  ██╔══██╗");
    println!(" ██║  ██║███████╗   ██║   ██║  ██║███████╗██║  ██║");
    println!(" ╚═╝  ╚═╝╚══════╝   ╚═╝   ╚═╝  ╚═╝╚══════╝╚═╝  ╚═╝");
    println!("       SOVEREIGN BLOCKCHAIN NODE & P2P HUB        \x1b[0m");
    println!("\x1b[1;36m================================================================================\x1b[0m");
    println!(" [비트코인 정신] 'Don't trust, verify' - 누구나 집에서 가동하는 탈중앙 풀 노드");
    println!(" [연산 자원] {} CPU Cores / 64-Shard Lock-Free MVCC", rayon::current_num_threads());
    println!(" [메모리 점유율] ~38MB (배틀그라운드, 롤, 일상 작업 중에도 팬 소음 0% 쾌적 구동)");
    println!("\x1b[1;36m================================================================================\x1b[0m");

    // Loopback only unless the operator explicitly opts in with --public.
    let bind_host = if public { "0.0.0.0" } else { "127.0.0.1" };
    let mut listener = None;
    let mut port = requested_port;
    for offset in 0..10 {
        let p = requested_port + offset;
        match TcpListener::bind(format!("{}:{}", bind_host, p)) {
            Ok(l) => {
                port = p;
                listener = Some(l);
                break;
            }
            Err(_) => continue,
        }
    }

    let listener = listener.expect("네트워크 포트 바인딩에 실패했습니다");
    let local_ip = get_local_ip();
    let local_url = format!("http://127.0.0.1:{}", port);

    // Initialize Persistent Node Identity & Peer Manager
    let identity = load_or_create_identity(port);
    let peer_mgr = PeerManager::new(identity.clone(), port);

    println!(" \x1b[1;32m✔ Aether Sovereign Node 데몬 가동 완료!\x1b[0m");
    println!(" [내 노드 URI]   \x1b[1;35maether://{}\x1b[0m ({})", identity.node_id, identity.name);
    println!(" [내 지갑 주소]   \x1b[1;33m{}\x1b[0m", identity.address.to_hex());
    println!(" [대시보드 접속] \x1b[1;36m{}\x1b[0m", local_url);
    println!(" [P2P 엔드포인트] \x1b[1;32maether://{}@{}:{}\x1b[0m", identity.node_id, local_ip, port);
    println!("\x1b[1;36m================================================================================\x1b[0m");

    let token = generate_token();
    match write_token_file(&token) {
        Ok(path) => println!(" [제어 토큰] {}", path.display()),
        Err(e) => eprintln!(" [제어 토큰] 저장 실패: {}", e),
    }
    let guard = Arc::new(ApiGuard::new(token, port));

    if public {
        // Opening router ports and announcing to public DHTs only when explicitly requested.
        start_lan_auto_discovery(peer_mgr.clone(), port);
        start_nat_traversal(peer_mgr.clone(), local_ip.clone(), port);
        start_bootnode_discovery(peer_mgr.clone());
        aether_core::dht::start_dht_worker(peer_mgr.clone(), port);
    } else {
        println!(" [네트워크] 로컬 전용 모드 (127.0.0.1). 외부 공개는 --public");
    }

    // If --peer argument was given, connect immediately
    if let Some(peer_addr) = connect_peer_arg {
        let pm = peer_mgr.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(500));
            println!(" [P2P 시작] 부트노드({})로 연결 시도...", peer_addr);
            let _ = pm.connect_peer(&peer_addr);
        });
    }

    let state = Arc::new(RwLock::new(NodeState::new(
        identity,
        peer_mgr,
        local_ip,
        port,
    )));

    // Background Block Production & Real Validator Reward Loop (Every 3 seconds)
    let state_bg = Arc::clone(&state);
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_secs(3));
            let (_round, vertex_opt, peer_mgr) = {
                let mut s = state_bg.write();
                let v = s.advance_round();
                // Real Block Validation Reward (+10 AETH per finalized round)
                let reward = 10;
                s.store.deposit(s.my_address, reward);
                s.total_rewards += reward;
                if s.round % 3 == 0 {
                    let sp = get_state_path(s.port);
                    s.store.save_to_disk(s.round, s.total_rewards, &sp);
                }
                (s.round, v, s.peer_mgr.clone())
            };

            // Broadcast vertex to connected peers
            if let Some(v) = vertex_opt {
                peer_mgr.broadcast_gossip(&GossipMessage::NewVertex(v));
            }
        }
    });

    // Open browser automatically in standalone App Window if available
    #[cfg(target_os = "macos")]
    if open_browser {
        let opened_app_window = if std::path::Path::new("/Applications/Google Chrome.app").exists() {
            std::process::Command::new("open")
                .args(["-na", "Google Chrome", "--args", &format!("--app={}", local_url)])
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        } else if std::path::Path::new("/Applications/Brave Browser.app").exists() {
            std::process::Command::new("open")
                .args(["-na", "Brave Browser", "--args", &format!("--app={}", local_url)])
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        } else {
            false
        };

        if !opened_app_window {
            let _ = std::process::Command::new("open").arg(&local_url).spawn();
        }
    }
    #[cfg(target_os = "linux")]
    if open_browser {
        let _ = std::process::Command::new("xdg-open").arg(&local_url).spawn();
    }
    #[cfg(target_os = "windows")]
    if open_browser {
        let _ = std::process::Command::new("cmd").args(["/C", "start", &local_url]).spawn();
    }

    let rate_limiter = Arc::new(RateLimiter::new(60, Duration::from_secs(60)));

    for stream in listener.incoming() {
        if let Ok(stream) = stream {
            let state_clone = Arc::clone(&state);
            let rl_clone = Arc::clone(&rate_limiter);
            let guard_clone = Arc::clone(&guard);
            thread::spawn(move || {
                handle_connection(stream, state_clone, rl_clone, guard_clone);
            });
        }
    }
}
