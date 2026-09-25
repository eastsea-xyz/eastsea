use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::thread;
use std::time::Duration;
use sha1::{Digest, Sha1};
use crate::p2p::PeerManager;

pub const BOOTSTRAP_ROUTERS: &[(&str, u16)] = &[
    ("dht.transmissionbt.com", 6881),
    ("dht.libtorrent.org", 25401),
    ("router.bittorrent.com", 6881),
];

pub fn get_aether_info_hash() -> [u8; 20] {
    let mut hasher = Sha1::new();
    hasher.update(b"aether-mesh-network-v1");
    let result = hasher.finalize();
    let mut hash = [0u8; 20];
    hash.copy_from_slice(&result);
    hash
}

pub fn generate_node_id() -> [u8; 20] {
    let mut id = [0u8; 20];
    for byte in id.iter_mut() {
        *byte = rand::random::<u8>();
    }
    // Prefix with 'AETH' for identification
    id[0] = b'A';
    id[1] = b'E';
    id[2] = b'T';
    id[3] = b'H';
    id
}

/// Construct KRPC get_peers query
pub fn make_get_peers_msg(node_id: &[u8; 20], info_hash: &[u8; 20]) -> Vec<u8> {
    let mut msg = Vec::with_capacity(128);
    msg.extend_from_slice(b"d1:ad2:id20:");
    msg.extend_from_slice(node_id);
    msg.extend_from_slice(b"9:info_hash20:");
    msg.extend_from_slice(info_hash);
    msg.extend_from_slice(b"e1:q9:get_peers1:t2:aa1:y1:qe");
    msg
}

/// Construct KRPC announce_peer query
pub fn make_announce_msg(
    node_id: &[u8; 20],
    info_hash: &[u8; 20],
    port: u16,
    token: &[u8],
) -> Vec<u8> {
    let mut msg = Vec::with_capacity(160);
    msg.extend_from_slice(b"d1:ad2:id20:");
    msg.extend_from_slice(node_id);
    msg.extend_from_slice(b"9:info_hash20:");
    msg.extend_from_slice(info_hash);
    let port_str = format!("4:porti{}e5:token{}:", port, token.len());
    msg.extend_from_slice(port_str.as_bytes());
    msg.extend_from_slice(token);
    msg.extend_from_slice(b"e1:q13:announce_peer1:t2:ab1:y1:qe");
    msg
}

/// Extract token from bencoded KRPC response
pub fn extract_token(buf: &[u8]) -> Option<Vec<u8>> {
    let key = b"5:token";
    let pos = buf.windows(key.len()).position(|w| w == key)?;
    let rest = &buf[pos + key.len()..];
    let colon_pos = rest.iter().position(|&b| b == b':')?;
    let len_str = std::str::from_utf8(&rest[..colon_pos]).ok()?;
    let tlen: usize = len_str.parse().ok()?;
    let start = colon_pos + 1;
    if start + tlen <= rest.len() {
        Some(rest[start..start + tlen].to_vec())
    } else {
        None
    }
}

/// Extract peer endpoints (IP:Port) from 'values' in bencoded response
pub fn extract_values(buf: &[u8]) -> Vec<String> {
    let mut peers = Vec::new();
    let key = b"6:valuesl";
    if let Some(pos) = buf.windows(key.len()).position(|w| w == key) {
        let mut rest = &buf[pos + key.len()..];
        while rest.starts_with(b"6:") && rest.len() >= 8 {
            let chunk = &rest[2..8];
            let ip = format!("{}.{}.{}.{}", chunk[0], chunk[1], chunk[2], chunk[3]);
            let port = u16::from_be_bytes([chunk[4], chunk[5]]);
            peers.push(format!("{}:{}", ip, port));
            rest = &rest[8..];
        }
    }
    peers
}

/// Extract closest DHT nodes from 'nodes' in bencoded response
pub fn extract_nodes(buf: &[u8]) -> Vec<SocketAddr> {
    let mut nodes = Vec::new();
    let key = b"5:nodes";
    if let Some(pos) = buf.windows(key.len()).position(|w| w == key) {
        let rest = &buf[pos + key.len()..];
        if let Some(colon_pos) = rest.iter().position(|&b| b == b':') {
            if let Ok(len_str) = std::str::from_utf8(&rest[..colon_pos]) {
                if let Ok(nlen) = len_str.parse::<usize>() {
                    let start = colon_pos + 1;
                    if start + nlen <= rest.len() {
                        let nodes_bytes = &rest[start..start + nlen];
                        let count = nlen / 26;
                        for i in 0..count {
                            let chunk = &nodes_bytes[i * 26..(i + 1) * 26];
                            let ip = std::net::Ipv4Addr::new(
                                chunk[20], chunk[21], chunk[22], chunk[23],
                            );
                            let port = u16::from_be_bytes([chunk[24], chunk[25]]);
                            nodes.push(SocketAddr::V4(std::net::SocketAddrV4::new(ip, port)));
                        }
                    }
                }
            }
        }
    }
    nodes
}

/// Query BitTorrent Mainline DHT to discover other Aether nodes
pub fn discover_dht_peers() -> Vec<String> {
    let socket = match UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let _ = socket.set_read_timeout(Some(Duration::from_millis(1500)));
    let _ = socket.set_write_timeout(Some(Duration::from_millis(1000)));

    let my_id = generate_node_id();
    let info_hash = get_aether_info_hash();
    let query_msg = make_get_peers_msg(&my_id, &info_hash);

    let mut candidate_nodes: Vec<SocketAddr> = Vec::new();

    // 1. Resolve and query bootstrap routers
    for &(host, port) in BOOTSTRAP_ROUTERS {
        if let Ok(addrs) = (host, port).to_socket_addrs() {
            for addr in addrs {
                let _ = socket.send_to(&query_msg, addr);
                let mut buf = [0u8; 4096];
                if let Ok((len, _)) = socket.recv_from(&mut buf) {
                    let nodes = extract_nodes(&buf[..len]);
                    candidate_nodes.extend(nodes);
                    let peers = extract_values(&buf[..len]);
                    if !peers.is_empty() {
                        return peers;
                    }
                }
            }
        }
    }

    // 2. Query DHT candidate neighbors for values (peers)
    let mut discovered_peers = Vec::new();
    let sample = candidate_nodes.into_iter().take(12);
    for addr in sample {
        let _ = socket.send_to(&query_msg, addr);
        let mut buf = [0u8; 4096];
        if let Ok((len, _)) = socket.recv_from(&mut buf) {
            let peers = extract_values(&buf[..len]);
            for p in peers {
                if !discovered_peers.contains(&p) {
                    discovered_peers.push(p);
                }
            }
        }
    }

    discovered_peers
}

/// Announce our node's public port to the global BitTorrent Mainline DHT
pub fn announce_to_dht(listen_port: u16) -> usize {
    let socket = match UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => s,
        Err(_) => return 0,
    };
    let _ = socket.set_read_timeout(Some(Duration::from_millis(1500)));
    let _ = socket.set_write_timeout(Some(Duration::from_millis(1000)));

    let my_id = generate_node_id();
    let info_hash = get_aether_info_hash();
    let get_peers_msg = make_get_peers_msg(&my_id, &info_hash);

    let mut candidate_nodes: Vec<SocketAddr> = Vec::new();

    // 1. Query bootstrap routers to find closest nodes
    for &(host, port) in BOOTSTRAP_ROUTERS {
        if let Ok(addrs) = (host, port).to_socket_addrs() {
            for addr in addrs {
                let _ = socket.send_to(&get_peers_msg, addr);
                let mut buf = [0u8; 4096];
                if let Ok((len, _)) = socket.recv_from(&mut buf) {
                    let nodes = extract_nodes(&buf[..len]);
                    candidate_nodes.extend(nodes);
                }
            }
        }
    }

    // 2. Query nodes to obtain tokens and send announce_peer
    let mut successful_announces = 0;
    let sample = candidate_nodes.into_iter().take(15);
    for addr in sample {
        let _ = socket.send_to(&get_peers_msg, addr);
        let mut buf = [0u8; 4096];
        if let Ok((len, _)) = socket.recv_from(&mut buf) {
            if let Some(token) = extract_token(&buf[..len]) {
                let ann_msg = make_announce_msg(&my_id, &info_hash, listen_port, &token);
                let _ = socket.send_to(&ann_msg, addr);
                let mut resp_buf = [0u8; 1024];
                if let Ok((resp_len, _)) = socket.recv_from(&mut resp_buf) {
                    if resp_len > 0 {
                        successful_announces += 1;
                        if successful_announces >= 5 {
                            break;
                        }
                    }
                }
            }
        }
    }

    successful_announces
}

/// Continuous background worker for BitTorrent DHT Free-Riding:
/// 1. Periodically announces ourselves to the global swarm.
/// 2. Discovers existing Aether nodes and automatically connects.
pub fn start_dht_worker(peer_mgr: PeerManager, listen_port: u16) {
    thread::spawn(move || {
        println!(" \x1b[1;36m⚡ [BitTorrent DHT]\x1b[0m 글로벌 토렌트 분산 해시 테이블(BEP 5) 무임승차 디스커버리 모듈 활성화");
        
        // Initial delay for UPnP and port readiness
        thread::sleep(Duration::from_secs(3));

        loop {
            // Step 1: Announce our existence to the DHT swarm
            let announces = announce_to_dht(listen_port);
            if announces > 0 {
                println!(" \x1b[1;32m✔ [BitTorrent DHT 무임승차]\x1b[0m 전 세계 {}개 토렌트 노드에 Aether 포트({}) 등록 완료!", announces, listen_port);
            }

            // Step 2: Discover peers
            let dht_peers = discover_dht_peers();
            for ep in dht_peers {
                if !peer_mgr.contains_endpoint(&ep) {
                    if let Ok(info) = peer_mgr.connect_peer(&ep) {
                        println!(" \x1b[1;35m🌐 [BitTorrent DHT 피어링]\x1b[0m 토렌트 망에서 발견한 피어({}: {})와 연결 성공!", ep, info.node_id);
                    }
                }
            }

            // Sleep 5 minutes before re-announcing / discovering, or wake faster if 0 peers
            let sleep_secs = if peer_mgr.count() == 0 { 60 } else { 300 };
            thread::sleep(Duration::from_secs(sleep_secs));
        }
    });
}
