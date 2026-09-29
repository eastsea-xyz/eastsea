#![no_main]

use aether_execution::{validate_stateless, EvmCall};
use aether_types::{TxEnvelope, TxPayload};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 64 << 10 {
        return;
    }
    if let Ok(tx) = serde_json::from_slice::<TxEnvelope>(data) {
        if let TxPayload::Plain(body) = &tx.payload {
            let _ = EvmCall::decode(body);
        }
        let _ = validate_stateless(&tx, 7_777);
    }
});
