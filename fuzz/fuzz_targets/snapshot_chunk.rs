#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 64 << 10 {
        return;
    }
    if let Ok(response) = serde_json::from_slice(data) {
        let expected = response["expected"]
            .as_u64()
            .map(|n| n as usize)
            .or_else(|| response["data"].as_str().map(|s| s.len() / 2))
            .unwrap_or(0);
        let _ = aether_node::follow::decode_snapshot_chunk(&response, expected);
    }
    let _ = aether_node::snapshot::Snapshot::from_bytes(data);
});
