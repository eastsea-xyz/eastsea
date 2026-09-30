//! A Mac as a voting-node candidate (docs/design/07-consensus.md, "open voting
//! nodes"): its keys, and the unattended beacon that proves it is alive each
//! epoch (and builds its contribution streak). No person acts after the owner
//! registers the Mac once.
//!
//! On a network with node rewards (docs/design/15-node-rewards.md) the node
//! answers the epoch's four beacon slots instead of sending a paid `beacon()`
//! transaction: answers go in blocks for free, so a Mac with a zero balance
//! takes part, and each answer also counts as the epoch's liveness.

use crate::chain::Chain;
use crate::faucet::Faucet;
use crate::follow::{Upstream, BEHIND_MARGIN};
use crate::roster::LocalKeys;
use aether_execution::registry::{self, encode_beacon, REGISTRY};
use aether_execution::EvmCall;
use aether_light::block::{BeaconAnswer, Reattestation};
use aether_rewards::{beacons, registry_v3};
use aether_types::{Address, U256};
use commonware_codec::Encode as _;
use commonware_cryptography::Signer as _;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

/// In the candidate's data dir: a fresh DeviceCheck token (base64) the app
/// writes for the node, used for the daily re-attestation.
pub const DEVICE_TOKEN_FILE: &str = "devicecheck-token";
/// Written by the Mac app before sleep/power-off and on wake/power return.
pub const AVAILABILITY_FILE: &str = "availability-state";

/// Where re-attestation requests go when set (the registrar's RPC URL);
/// otherwise a follower asks its upstream.
pub const REGISTRAR_RPC_ENV: &str = "AETHER_REGISTRAR_RPC";

/// The process exits with this code when this Mac's identity is unreadable
/// (red team #5): a registered Mac must never vote under a replacement key,
/// so voting stays off until the real key file is restored.
pub const EXIT_IDENTITY: i32 = 6;

/// Whether `<dir>` holds — or ever held — this Mac's identity: the key files
/// themselves, a committee's files, or the moved-aside remains of either
/// (`stale-*` from a network reset, `corrupt-*` from a store heal). In such a
/// directory a key that is gone or will not parse is a loss to report, never
/// a reason to generate a different identity the chain does not know.
pub fn registered_identity(dir: &Path) -> bool {
    if dir.with_extension("identity").exists() {
        return true;
    }
    let marks = [
        crate::roster::KEY_FILE,
        crate::roster::PUBLIC_FILE,
        "node-account.key",
        "network.json",
        "threshold.json",
    ];
    if marks.iter().any(|m| dir.join(m).exists()) {
        return true;
    }
    std::fs::read_dir(dir)
        .map(|entries| {
            entries.flatten().any(|e| {
                let n = e.file_name();
                let n = n.to_string_lossy();
                n.starts_with("stale-") || n.starts_with("corrupt-")
            })
        })
        .unwrap_or(false)
}

/// A candidate's keys: voting key and iroh node key (`validator.key`) and the
/// node's own account (`node-account.key`), which pays for and sends beacons.
pub struct CandidateKeys {
    pub keys: LocalKeys,
    pub account: Faucet,
    /// The data dir the keys live in (and the app's DeviceCheck token).
    pub dir: PathBuf,
}

/// Never the key material itself (`expect_err` in tests wants Debug).
impl std::fmt::Debug for CandidateKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CandidateKeys(..)")
    }
}

impl CandidateKeys {
    /// Load `<dir>`'s keys, creating them the first time (never overwritten).
    /// A key that is gone or unreadable in a directory that ever held an
    /// identity is refused, not replaced (red team #5): the chain knows the
    /// registered key, and a fresh one would silently vote as someone else.
    pub fn load_or_create(dir: &Path) -> Result<Self, String> {
        let (keys, first_install) = match LocalKeys::load(dir) {
            Ok(k) => (k, false),
            Err(e) => {
                if registered_identity(dir) {
                    return Err(format!(
                        "{e}; this Mac already had an identity, so no new key is generated. \
                         Voting stays off until {}/{} is restored from a backup (or this Mac \
                         is unregistered and a new identity is registered on purpose)",
                        dir.display(),
                        crate::roster::KEY_FILE
                    ));
                }
                let k = LocalKeys::generate();
                k.save(dir)?;
                (k, true)
            }
        };
        let account_path = dir.join("node-account.key");
        if !account_path.exists() {
            if !first_install {
                return Err(format!("{} is missing from an existing identity; restore it from a backup instead of replacing it", account_path.display()));
            }
            Faucet::generate(&account_path)?;
        }
        let candidate = CandidateKeys { keys, account: Faucet::load(&account_path)?, dir: dir.to_path_buf() };
        // This sibling survives deletion of the entire node data directory.
        // Without it, that loss looks exactly like a first install and would
        // silently mint a new registered identity (red team #5).
        let marker = dir.with_extension("identity");
        let fingerprint = format!("{}:{}", hex::encode(candidate.validator_key()), candidate.beaconer());
        match std::fs::read_to_string(&marker) {
            Ok(saved) if saved == fingerprint => {}
            Ok(_) => return Err(format!("{} does not match this Mac's original identity; restore the original keys", marker.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if let Err(write) = crate::atomic::create(&marker, fingerprint.as_bytes(), 0o600) {
                    if std::fs::read_to_string(&marker).ok().as_deref() != Some(fingerprint.as_str()) {
                        return Err(format!("{}: {write}", marker.display()));
                    }
                }
            }
            Err(e) => return Err(format!("{}: {e}", marker.display())),
        }
        Ok(candidate)
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

    /// The voting key's signature asking the registrar to register it for `operator`.
    pub fn ownership(&self, chain_id: u64, operator: Address) -> Vec<u8> {
        let msg = registry::attestation_message(chain_id, operator, self.validator_key(), self.node_id(), self.beaconer());
        self.keys.signer.sign(crate::devicecheck::OWNERSHIP_NAMESPACE, &msg).encode().to_vec()
    }

    /// The voting key's request to be re-attested for `period`.
    pub fn reattest_request(&self, chain_id: u64, period: u64) -> Vec<u8> {
        let msg = beacons::reattest_message(chain_id, &self.validator_key(), period);
        self.keys.signer.sign(crate::devicecheck::OWNERSHIP_NAMESPACE, &msg).encode().to_vec()
    }
}

/// Where beacons go: a follower forwards to validators; a validator gossips its own.
pub enum Outbox {
    Upstream(Arc<Upstream>),
    Local(tokio::sync::mpsc::UnboundedSender<aether_types::TxEnvelope>),
}

impl Outbox {
    async fn send(&self, chain: &Chain, tx: aether_types::TxEnvelope) -> Result<(), String> {
        match self {
            Outbox::Upstream(u) => u.call("aether_sendTransaction", json!([tx])).await.map(|_| ()),
            Outbox::Local(gossip) => {
                if chain.add_to_mempool(tx.clone())? {
                    let _ = gossip.send(tx);
                }
                Ok(())
            }
        }
    }

    async fn send_answer(&self, chain: &Chain, a: BeaconAnswer) -> Result<(), String> {
        match self {
            Outbox::Upstream(u) => u.call("aether_sendBeacon", json!([a])).await.map(|_| ()),
            Outbox::Local(_) => chain.submit_beacon(a).map(|_| ()),
        }
    }

    /// Ask the registrar (or, on a follower, the upstream) for a re-attestation.
    async fn reattest(&self, params: Value) -> Result<Value, String> {
        if let Ok(url) = std::env::var(REGISTRAR_RPC_ENV) {
            let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "aether_reattest", "params": params });
            let v: Value = reqwest::Client::new()
                .post(url)
                .json(&body)
                .timeout(Duration::from_secs(30))
                .send()
                .await
                .map_err(|e| e.to_string())?
                .json()
                .await
                .map_err(|e| e.to_string())?;
            return match v.get("error") {
                Some(e) => Err(e.to_string()),
                None => Ok(v["result"].clone()),
            };
        }
        match self {
            Outbox::Upstream(u) => u.first("aether_reattest", params).await,
            Outbox::Local(_) => Err(format!("no registrar to re-attest with (set {REGISTRAR_RPC_ENV})")),
        }
    }
}

/// What the loop remembers between rounds.
#[derive(Default)]
struct Answering {
    /// (epoch, slot) answered.
    sent: BTreeSet<(u64, u64)>,
    /// Re-attestations fetched, by period.
    attested: BTreeMap<u64, Reattestation>,
}

/// Answer every open slot of this Mac's registration (node rewards networks).
async fn answer_slots(chain: &Chain, outbox: &Outbox, keys: &CandidateKeys, st: &mut Answering) {
    let (height, state, chain_id, head_hash) = {
        let g = chain.lock();
        let f = &g.finalized;
        (f.height, f.state.clone(), g.cfg.chain_id, f.digest.as_ref().try_into().expect("32-byte digest"))
    };
    let me = keys.validator_key();
    let Some(c) = registry::candidates(&state).into_iter().find(|c| c.validator_key == me) else {
        return;
    };
    let next = height + 1;
    let view = crate::beacons::next_view(&state, next, head_hash);
    for due in beacons::due(&view, next, &c) {
        if st.sent.contains(&(due.epoch, due.slot)) {
            continue;
        }
        let attest = match (due.needs_attestation, st.attested.get(&due.period)) {
            (false, _) => None,
            (true, Some(r)) => Some(r.clone()),
            (true, None) => {
                let params = match std::fs::read_to_string(keys.dir.join(DEVICE_TOKEN_FILE)) {
                    Ok(token) => json!([token.trim(), hex::encode(me), due.period, hex::encode(keys.reattest_request(chain_id, due.period))]),
                    Err(e) => {
                        warn!(%e, period = due.period, "daily re-attestation due, but the app left no DeviceCheck token: this Mac earns nothing until it re-attests");
                        continue;
                    }
                };
                match outbox.reattest(params).await.and_then(|v| serde_json::from_value::<Reattestation>(v).map_err(|e| e.to_string())) {
                    Ok(r) => {
                        info!(period = due.period, "re-attested with a fresh DeviceCheck token");
                        st.attested.insert(due.period, r.clone());
                        Some(r)
                    }
                    Err(e) => {
                        warn!(%e, "re-attestation refused");
                        continue;
                    }
                }
            }
        };
        let answer = crate::beacons::sign(&keys.keys.signer, chain_id, c.index, &due, attest);
        match outbox.send_answer(chain, answer).await {
            Ok(()) => {
                st.sent.insert((due.epoch, due.slot));
                info!(epoch = due.epoch, slot = due.slot, "beacon slot answered");
            }
            Err(e) => warn!(%e, "beacon answer not accepted"),
        }
    }
    let epoch = next / registry::epoch_blocks(&state);
    st.sent.retain(|(e, _)| *e >= epoch);
    let keep = beacons::period(&view, epoch, 0);
    st.attested.retain(|p, _| *p >= keep);
}

/// Send one beacon per epoch once this candidate is registered (on a node
/// rewards network: answer the slots). Runs forever, also while the Mac
/// votes: a voting node that stopped beaconing would drop out of the next selection.
pub async fn beacon_loop(chain: Chain, outbox: Outbox, keys: CandidateKeys) {
    let me = keys.validator_key();
    let mut sent_for = u64::MAX;
    let mut answering = Answering::default();
    #[cfg(unix)]
    let mut wake = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::user_defined1()).expect("SIGUSR1 handler");
    loop {
        // No beacons while catching up — and none before any height is known:
        // one is a claim this Mac is current, and it would be checked against
        // a state this node has not reached (a follower or a validator still
        // catching up before it votes). An unknown height is not "0 behind"
        // (2026-09-29): wait until the network says where we are.
        let Some(behind) = chain.behind_known() else {
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        };
        if behind > BEHIND_MARGIN {
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }
        let (height, state, cfg) = {
            let g = chain.lock();
            (g.finalized.height, g.finalized.state.clone(), g.cfg.clone())
        };
        if aether_rewards::enabled(&state) {
            let free_voting = registry_v3::is_v3(&state);
            let leaving = free_voting && std::fs::read_to_string(keys.dir.join(AVAILABILITY_FILE)).ok().is_some_and(|v| v.trim() == "leaving");
            if free_voting {
                if let Some(c) = registry::candidates(&state).into_iter().find(|c| c.validator_key == me) {
                    let on_chain = registry_v3::availability(&state, c.index).is_some_and(|(_, v)| v);
                    if on_chain != leaving {
                        let signal = crate::beacons::sign_availability(&keys.keys.signer, cfg.chain_id, c.index, height + 1, leaving);
                        if let Err(e) = outbox.send_answer(&chain, signal).await {
                            warn!(%e, leaving, "availability announcement not accepted");
                        }
                    }
                }
            }
            if !leaving {
                answer_slots(&chain, &outbox, &keys, &mut answering).await;
            }
            #[cfg(unix)]
            tokio::select! { _ = tokio::time::sleep(Duration::from_secs(1)) => {}, _ = wake.recv() => {} }
            #[cfg(not(unix))]
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }
        let epoch = height / registry::epoch_blocks(&state);
        match registry::candidates(&state).into_iter().find(|c| c.validator_key == me) {
            None => {}
            Some(c) if c.last_epoch >= epoch || sent_for == epoch => {}
            Some(_) => {
                let nonce = state.nonce(&keys.beaconer());
                let base = Chain::next_base_fee(&cfg, &chain.lock().finalized);
                let call = EvmCall { to: Some(REGISTRY), value: U256::ZERO, input: encode_beacon(me), gas_limit: 100_000, delegate: None };
                match keys.account.sign_tx(cfg.chain_id, nonce, &call, base) {
                    Ok(tx) => match outbox.send(&chain, tx).await {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aether-candidate-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A first install creates its keys; a re-run loads the very same ones.
    #[test]
    fn a_first_install_creates_keys_and_keeps_them() {
        let dir = tmp("fresh");
        let a = CandidateKeys::load_or_create(&dir).unwrap();
        let again = CandidateKeys::load_or_create(&dir).unwrap();
        assert_eq!(a.validator_key(), again.validator_key(), "the identity never changes");
        assert!(dir.join(crate::roster::KEY_FILE).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Red team #5: the key file vanishing (or failing to parse) in a
    /// directory that ever held an identity must not mint a new identity —
    /// the chain knows the old key, and a fresh one would vote as someone
    /// else. Every marker of a past identity refuses the same way.
    #[test]
    fn a_lost_or_broken_key_is_never_replaced_by_a_new_identity() {
        for (name, marker) in [
            ("key-gone", None),
            ("pub-left", Some(crate::roster::PUBLIC_FILE)),
            ("network-left", Some("network.json")),
            ("share-left", Some("threshold.json")),
            ("account-left", Some("node-account.key")),
            ("moved-aside", Some("stale-123")),
        ] {
            let dir = tmp(name);
            // A real first install, then the incident.
            CandidateKeys::load_or_create(&dir).unwrap();
            let was = dir.join(crate::roster::KEY_FILE);
            match marker {
                None => {
                    std::fs::remove_file(&was).unwrap();
                }
                Some(m) if m == crate::roster::PUBLIC_FILE => {
                    std::fs::remove_file(&was).unwrap();
                }
                Some(m) if m.starts_with("stale-") => {
                    let gone = dir.parent().unwrap().join(format!("aether-candidate-gone-{name}-{}", std::process::id()));
                    std::fs::rename(&dir, &gone).unwrap();
                    std::fs::create_dir_all(&dir).unwrap();
                    std::fs::create_dir_all(dir.join(m)).unwrap();
                    let _ = std::fs::remove_dir_all(&gone);
                }
                Some(m) => {
                    std::fs::remove_file(&was).unwrap();
                    std::fs::write(dir.join(m), b"leftovers of an installed Mac").unwrap();
                }
            }
            let before: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
            let err = CandidateKeys::load_or_create(&dir).expect_err("a registered identity refuses to regenerate");
            assert!(err.contains("no new key is generated"), "{name}: {err}");
            assert!(
                !dir.join(crate::roster::KEY_FILE).exists(),
                "{name}: no replacement key was written"
            );
            let after: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
            assert_eq!(before.len(), after.len(), "{name}: the directory is untouched");
        }
    }

    /// A key file damaged in place (torn write, disk corruption) is the same
    /// refusal — parsing failure is loss, not first install.
    #[test]
    fn a_key_that_no_longer_parses_is_refused_too() {
        let dir = tmp("corrupt");
        CandidateKeys::load_or_create(&dir).unwrap();
        std::fs::write(dir.join(crate::roster::KEY_FILE), b"{\"consensus\": \"zz").unwrap();
        let err = CandidateKeys::load_or_create(&dir).expect_err("a broken key is not a fresh Mac");
        assert!(err.contains("no new key is generated"), "{err}");
        assert_eq!(
            std::fs::read(dir.join(crate::roster::KEY_FILE)).unwrap(),
            b"{\"consensus\": \"zz",
            "the damaged bytes stay for an operator to inspect"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_beacon_account_is_not_silently_replaced() {
        let dir = tmp("account-gone");
        let original = CandidateKeys::load_or_create(&dir).unwrap().beaconer();
        std::fs::remove_file(dir.join("node-account.key")).unwrap();
        let err = CandidateKeys::load_or_create(&dir).expect_err("an existing account is not regenerated");
        assert!(err.contains("restore it from a backup"), "{err}");
        assert!(!dir.join("node-account.key").exists());
        assert!(!original.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_deleted_data_directory_cannot_be_mistaken_for_first_install() {
        let parent = tmp("whole-directory-gone");
        let data = parent.join("node");
        let original = CandidateKeys::load_or_create(&data).unwrap();
        let identity = original.validator_key();
        let key = std::fs::read(data.join(crate::roster::KEY_FILE)).unwrap();
        let account = std::fs::read(data.join("node-account.key")).unwrap();
        assert!(data.with_extension("identity").exists(), "the guard lives outside the data directory");
        std::fs::remove_dir_all(&data).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        assert!(CandidateKeys::load_or_create(&data).is_err(), "a whole-directory loss cannot mint a replacement");
        assert!(!data.join(crate::roster::KEY_FILE).exists());
        std::fs::write(data.join(crate::roster::KEY_FILE), key).unwrap();
        std::fs::write(data.join("node-account.key"), account).unwrap();
        assert_eq!(CandidateKeys::load_or_create(&data).unwrap().validator_key(), identity, "the original backup works");
        std::fs::remove_file(data.join(crate::roster::KEY_FILE)).unwrap();
        crate::roster::LocalKeys::generate().save(&data).unwrap();
        let err = CandidateKeys::load_or_create(&data).expect_err("a different restored key is not the original");
        assert!(err.contains("original identity"), "{err}");
        let _ = std::fs::remove_dir_all(&parent);
    }
}
