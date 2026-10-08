//! Print a public, deterministic local devnet pin for the browser test harness.
use commonware_cryptography::Signer as _;
use serde_json::json;

fn main() {
    let n = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "3".into())
        .parse::<u64>()
        .expect("validator count");
    assert!(
        (3..=16).contains(&n),
        "test validator count must be between 3 and 16"
    );
    println!(
        "{}",
        json!({
            "chain_id": 7777,
            "identity": aether_light::ValidatorSet::devnet(n).identity_hex(),
            "validators": (1..=n).map(|i| json!({
                "key": aether_light::devnet_validator_key(i).public_key().to_string(),
                "node": aether_net::devnet_node_id(i).to_string(),
            })).collect::<Vec<_>>()
        })
    );
}
