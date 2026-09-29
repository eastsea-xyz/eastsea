#![no_main]

use aether_light::block::{Block, Payload};
use commonware_codec::DecodeExt;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 1 << 20 {
        return;
    }
    if let Ok(block) = Block::decode_cfg(data, &Block::codec_config(1 << 20)) {
        let _ = block.payload();
    }
    let _ = Payload::from_bytes(data);
});
