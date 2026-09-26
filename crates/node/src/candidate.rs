//! A Mac as a voting-node candidate (docs/design/07-consensus.md, "open voting
//! nodes"): its keys, and the unattended beacon that proves it is alive each
//! epoch (and builds its contribution streak). No person acts after the owner
//! registers the Mac once.

use crate::chain::Chain;
use crate::faucet::Faucet;
use crate::follow::Upstream;
use crate::roster::LocalKeys;
use aether_execution::registry::{self, encode_beacon, REGISTRY};
use aether_execution::EvmCall;
use aether_types::{Address, U256};
use commonware_cryptography::Signer as _;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

/// A candidate's keys: voting key and iroh node key (`validator.key`) and the
/// node's own account (`node-account.key`), which pays for and sends beacons.
pub struct CandidateKeys {
    pub keys: LocalKeys,
    pub account: Faucet,
}

impl CandidateKeys {
    /// Load `<dir>`'s keys, creating them the first time (never overwritten).
    pub fn load_or_create(dir: &Path) -> Result<Self, String> {
        let keys = match LocalKeys::load(dir) {
            Ok(k) => k,
            Err(_) => {
                let k = LocalKeys::generate();
                k.save(dir)?;
                k
            }
        };
        let account_path = dir.join("node-account.key");
        if !account_path.exists() {
            Faucet::generate(&account_path)?;
        }
        Ok(CandidateKeys { keys, account: Faucet::load(&account_path)? })
    }

    pub fn validator_key(&self) -> [u8; 32] {
        self.keys.signer.public_key().as_ref().try_into().expect("ed25519 key is 32 bytes")
    }

    pub fn node_id(&self) -> [u8; 32] {
        *self.keys.node_secret.public().as_bytes()
    }

    pub fn beaconer(&self) -> Address {
        self.account.address
    }
}

/// Send one beacon per epoch once this candidate is registered. Runs forever.
pub async fn beacon_loop(chain: Chain, upstream: Arc<Upstream>, keys: CandidateKeys) {
    let me = keys.validator_key();
    let mut sent_for = u64::MAX;
    loop {
        let (height, state, cfg) = {
            let g = chain.lock();
            (g.finalized.height, g.finalized.state.clone(), g.cfg.clone())
        };
        let epoch = height / registry::epoch_blocks(&state);
        match registry::candidates(&state).into_iter().find(|c| c.validator_key == me) {
            None => {}
            Some(c) if c.last_epoch >= epoch || sent_for == epoch => {}
            Some(_) => {
                let nonce = state.nonce(&keys.beaconer());
                let base = Chain::next_base_fee(&cfg, &chain.lock().finalized);
                let call = EvmCall { to: Some(REGISTRY), value: U256::ZERO, input: encode_beacon(me), gas_limit: 100_000, delegate: None };
                match keys.account.sign_tx(cfg.chain_id, nonce, &call, base) {
                    Ok(tx) => match upstream.call("aether_sendTransaction", json!([tx])).await {
                        Ok(_) => {
                            sent_for = epoch;
                            info!(epoch, "voting-node beacon sent");
                        }
                        Err(e) => warn!(%e, "beacon not accepted"),
                    },
                    Err(e) => warn!(%e, "beacon not signed"),
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}
