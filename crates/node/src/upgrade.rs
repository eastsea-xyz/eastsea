//! Protocol upgrades signed by the committee (docs/design/12-launch-plan.md step 5).
//!
//! An upgrade names a protocol version, the height it activates at, and the
//! release artifacts (with BLAKE3 hashes) that implement it. It takes effect only
//! with a threshold signature of the committee: each validator signs with its
//! key share (`sign_partial`), any `threshold` partials combine into one
//! signature (`combine`), and anyone checks it against the committee identity
//! wallets already pin (`verify`). No single operator can change the rules.
//!
//! A node whose protocol is older than an activated upgrade stops before the
//! activation height instead of forking off with old rules; wallets and the app
//! use the same signed file to know which release to trust.

use aether_light::Identity;
use commonware_codec::{DecodeExt, Encode};
use commonware_cryptography::bls12381::primitives::group::Share;
use commonware_cryptography::bls12381::primitives::ops;
use commonware_cryptography::bls12381::primitives::sharing::Sharing;
use commonware_cryptography::bls12381::primitives::variant::{MinSig, PartialSignature};
use commonware_parallel::Sequential;
use commonware_cryptography::{ed25519, Signer as _, Verifier as _};
use commonware_utils::{Faults as _, N3f1};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// The protocol this binary implements.
pub const PROTOCOL: u32 = 3;
const NAMESPACE: &[u8] = aether_light::UPGRADE_NAMESPACE;
const EMERGENCY_NAMESPACE: &[u8] = b"aether-upgrade-emergency-v1";
pub const MAINNET_NOTICE_BLOCKS: u64 = 604_800;

/// The protocol this binary runs (`PROTOCOL`). Drill builds — the only builds
/// with the `dev-drill` feature, see Cargo.toml — may claim a later protocol
/// with `AETHER_DEV_PROTOCOL=<n>` so `scripts/upgrade-drill.sh` can rehearse a
/// committee upgrade to a release this tree does not implement; the value must
/// be higher than `PROTOCOL` (a drill release is never older) and the rules it
/// runs are still this tree's, because no code keys on the higher number.
/// Shipped builds compile the override out and always report `PROTOCOL`.
pub fn implements() -> u32 {
    #[cfg(feature = "dev-drill")]
    {
        static CACHE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
        *CACHE.get_or_init(|| parse_dev_protocol(std::env::var("AETHER_DEV_PROTOCOL").ok().as_deref()).unwrap_or(PROTOCOL))
    }
    #[cfg(not(feature = "dev-drill"))]
    {
        PROTOCOL
    }
}

/// `AETHER_DEV_PROTOCOL` is the claimed protocol as plain decimal, accepted
/// only above `PROTOCOL`; anything else falls back to `PROTOCOL`.
#[cfg(feature = "dev-drill")]
fn parse_dev_protocol(value: Option<&str>) -> Option<u32> {
    let n: u32 = value?.trim().parse().ok()?;
    (n > PROTOCOL).then_some(n)
}

/// A rehearsal's replacement for the 604,800-block mainnet notice, from
/// `AETHER_DEV_UPGRADE_NOTICE=<blocks>` (`admissible_upgrade` reads it on the
/// scheduled path only). Drill builds only: shipped builds always get `None`
/// and keep the mainnet notice.
pub fn dev_notice() -> Option<u64> {
    #[cfg(feature = "dev-drill")]
    {
        static CACHE: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
        *CACHE.get_or_init(|| parse_dev_notice(std::env::var("AETHER_DEV_UPGRADE_NOTICE").ok().as_deref()))
    }
    #[cfg(not(feature = "dev-drill"))]
    {
        None
    }
}

/// `AETHER_DEV_UPGRADE_NOTICE` is the notice in blocks, positive; anything
/// else means "no override" and the mainnet notice stands.
#[cfg(feature = "dev-drill")]
fn parse_dev_notice(value: Option<&str>) -> Option<u64> {
    value?.trim().parse::<u64>().ok().filter(|n| *n > 0)
}

pub use aether_light::block::{Release, SignedUpgrade, Upgrade};

/// A protocol activation on chain: from height `at`, protocol `protocol`
/// (and a new registrar key, if the upgrade names one).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activation {
    pub protocol: u32,
    pub at: u64,
    /// Always serialized: snapshots use postcard, which cannot skip fields.
    #[serde(default)]
    pub registrar: Option<(aether_types::B256, aether_types::B256)>,
}

/// Activation schedule on chain, ascending in protocol and height.
pub type Schedule = Vec<Activation>;

/// The protocol whose rules apply at `height`.
pub fn protocol_at(schedule: &[Activation], height: u64) -> u32 {
    schedule.iter().filter(|a| a.at <= height).map(|a| a.protocol).max().unwrap_or(1)
}

/// Size bounds of an upgrade carried in a block.
pub const MAX_RELEASES: usize = 16;
pub const MAX_FIELD: usize = 512;

fn message(upgrade: &Upgrade) -> Vec<u8> {
    serde_json::to_vec(upgrade).expect("upgrade serializes")
}

/// One validator's share of the signature.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PartialUpgrade {
    pub upgrade: Upgrade,
    /// Codec bytes (hex) of the partial signature.
    pub partial: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emergency_approval: Option<(String, String)>,
}

pub fn sign_partial(upgrade: &Upgrade, share: &Share) -> PartialUpgrade {
    crate::key_binding::check_process();
    let p = ops::threshold::sign_message::<MinSig>(share, NAMESPACE, &message(upgrade));
    PartialUpgrade { upgrade: upgrade.clone(), partial: hex::encode(p.encode()), emergency_approval: None }
}

pub fn sign_emergency_partial(upgrade: &Upgrade, share: &Share, key: &ed25519::PrivateKey) -> PartialUpgrade {
    let mut partial = sign_partial(upgrade, share);
    crate::key_binding::check_process();
    let signature = key.sign(EMERGENCY_NAMESPACE, &message(upgrade));
    partial.emergency_approval = Some((hex::encode(key.public_key().encode()), hex::encode(signature.encode())));
    partial
}

/// Combine partials (all for the same upgrade, each checked) into the committee signature.
pub fn combine(sharing: &Sharing<MinSig>, partials: &[PartialUpgrade]) -> Result<SignedUpgrade, String> {
    let first = partials.first().ok_or("no partial signatures")?;
    let msg = message(&first.upgrade);
    let mut decoded = Vec::new();
    for p in partials {
        if p.upgrade != first.upgrade {
            return Err("partials are for different upgrades".into());
        }
        let bytes = hex::decode(&p.partial).map_err(|e| e.to_string())?;
        let ps = PartialSignature::<MinSig>::decode(bytes.as_slice()).map_err(|e| format!("partial: {e:?}"))?;
        ops::threshold::verify_message::<MinSig>(sharing, NAMESPACE, &msg, &ps).map_err(|_| format!("invalid partial from signer {}", ps.index))?;
        decoded.push(ps);
    }
    let sig = ops::threshold::recover::<MinSig, _>(sharing, &decoded, &Sequential).map_err(|e| format!("need {} valid partials: {e:?}", sharing.required()))?;
    let emergency_approvals = if first.upgrade.emergency {
        partials.iter().filter_map(|p| p.emergency_approval.clone()).collect()
    } else {
        Vec::new()
    };
    Ok(SignedUpgrade { upgrade: first.upgrade.clone(), signature: hex::encode(sig.encode()), emergency_approvals })
}

/// New-genesis emergencies need n-f independent approvals from current voting
/// keys, using the committee's BFT fault model. Legacy 7780 retains unanimity
/// here; its consensus admission still refuses the emergency flag entirely.
pub fn verify_emergency(s: &SignedUpgrade, committee: &[(String, String)]) -> Result<(), String> {
    if s.upgrade.chain_id == 7_780 && (committee.is_empty() || s.emergency_approvals.len() != committee.len()) {
        return Err("emergency upgrade needs every current committee member".into());
    }
    if committee.is_empty() {
        return Err("emergency upgrade needs a current committee".into());
    }
    let members = u32::try_from(committee.len()).map_err(|_| "committee is too large")?;
    let required = if s.upgrade.chain_id == 7_780 { members } else { N3f1::quorum(members) } as usize;
    if s.emergency_approvals.len() < required || s.emergency_approvals.len() > committee.len() {
        return Err(format!("emergency upgrade needs at least {required} current committee members"));
    }
    let mut remaining: std::collections::HashSet<_> = committee.iter().map(|m| m.0.as_str()).collect();
    if remaining.len() != committee.len() { return Err("duplicate committee key".into()); }
    for (key, signature) in &s.emergency_approvals {
        if !remaining.remove(key.as_str()) { return Err("duplicate or foreign emergency signer".into()); }
        let pk = hex::decode(key).map_err(|_| "emergency signer key is not hex")?;
        let pk = ed25519::PublicKey::decode(pk.as_slice()).map_err(|_| "invalid emergency signer key")?;
        let sig = hex::decode(signature).map_err(|_| "emergency signature is not hex")?;
        let sig = ed25519::Signature::decode(sig.as_slice()).map_err(|_| "invalid emergency signature")?;
        if !pk.verify(EMERGENCY_NAMESPACE, &message(&s.upgrade), &sig) {
            return Err("emergency approval does not verify".into());
        }
    }
    Ok(())
}

/// Check the committee signature.
pub fn verify(identity: &Identity, s: &SignedUpgrade) -> Result<(), String> {
    aether_light::verify_upgrade(identity, s)
}

/// Verified upgrades for `chain_id` from `dir/*.json` (unsigned or foreign files are skipped, with a reason).
pub fn load(dir: &Path, identity: &Identity, chain_id: u64) -> (Vec<SignedUpgrade>, Vec<String>) {
    let (mut ok, mut skipped) = (Vec::new(), Vec::new());
    let Ok(entries) = std::fs::read_dir(dir) else { return (ok, skipped) };
    for e in entries.flatten() {
        let path = e.path();
        if path.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let parsed = std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| serde_json::from_slice::<SignedUpgrade>(&b).map_err(|e| e.to_string()));
        match parsed.and_then(|s| verify(identity, &s).map(|_| s)) {
            Ok(s) if s.upgrade.chain_id == chain_id => ok.push(s),
            Ok(_) => skipped.push(format!("{}: another chain", path.display())),
            Err(e) => skipped.push(format!("{}: {e}", path.display())),
        }
    }
    ok.sort_by_key(|s| s.upgrade.activate_at);
    (ok, skipped)
}

/// The protocol the chain requires at `height`, given verified upgrades.
pub fn required_protocol(upgrades: &[SignedUpgrade], height: u64) -> u32 {
    upgrades.iter().filter(|s| s.upgrade.activate_at <= height).map(|s| s.upgrade.protocol).max().unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upgrade(protocol: u32, at: u64) -> Upgrade {
        Upgrade {
            chain_id: 7_778,
            protocol,
            activate_at: at,
            emergency: false,
            releases: vec![Release { platform: "macos-arm64-dmg".into(), version: "0.2.0".into(), blake3: "ab".repeat(32), url: "https://x".into() }],
            notes: "test".into(),
            registrar: None,
        }
    }

    #[test]
    fn threshold_of_the_committee_signs_and_anyone_verifies() {
        let (_, sharing, shares) = aether_light::devnet_threshold(4);
        let identity = *sharing.public();
        let u = upgrade(2, 1_000);
        assert!(!serde_json::to_string(&u).unwrap().contains("emergency"), "legacy signing bytes stay unchanged");
        let partials: Vec<_> = shares.iter().take(3).map(|(_, s)| sign_partial(&u, s)).collect();
        let signed = combine(&sharing, &partials).unwrap();
        verify(&identity, &signed).unwrap();

        // Fewer than the threshold cannot sign.
        assert!(combine(&sharing, &partials[..2]).is_err());
        // Changing anything breaks the signature.
        let mut tampered = signed.clone();
        tampered.upgrade.activate_at = 10;
        assert!(verify(&identity, &tampered).is_err());
        tampered = signed.clone();
        tampered.upgrade.releases[0].blake3 = "cd".repeat(32);
        assert!(verify(&identity, &tampered).is_err());
        // Partials for different upgrades do not mix.
        let other = sign_partial(&upgrade(3, 1_000), &shares[3].1);
        assert!(combine(&sharing, &[partials[0].clone(), partials[1].clone(), other]).is_err());
        // Another committee's key does not verify it.
        let (_, other_sharing, _) = aether_light::devnet_threshold(7);
        assert!(verify(other_sharing.public(), &signed).is_err());
    }

    #[test]
    fn required_protocol_follows_activation_heights() {
        let (_, sharing, shares) = aether_light::devnet_threshold(4);
        let sign = |u: Upgrade| combine(&sharing, &shares.iter().take(3).map(|(_, s)| sign_partial(&u, s)).collect::<Vec<_>>()).unwrap();
        let ups = vec![sign(upgrade(2, 100)), sign(upgrade(3, 500))];
        assert_eq!(required_protocol(&ups, 99), 1);
        assert_eq!(required_protocol(&ups, 100), 2);
        assert_eq!(required_protocol(&ups, 10_000), 3);

        let dir = std::env::temp_dir().join(format!("aether-upgrades-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.json"), serde_json::to_vec(&ups[0]).unwrap()).unwrap();
        let mut forged = ups[1].clone();
        forged.upgrade.protocol = 9;
        std::fs::write(dir.join("b.json"), serde_json::to_vec(&forged).unwrap()).unwrap();
        let (ok, skipped) = load(&dir, sharing.public(), 7_778);
        assert_eq!(ok.len(), 1);
        assert_eq!(skipped.len(), 1, "a forged upgrade is ignored");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn emergency_requires_a_current_committee_quorum_and_binds_the_upgrade() {
        let (_, sharing, shares) = aether_light::devnet_threshold(4);
        let keys: Vec<_> = (1..=4).map(aether_light::devnet_validator_key).collect();
        let committee: Vec<_> = keys.iter().map(|key| (hex::encode(key.public_key().encode()), String::new())).collect();
        let mut u = upgrade(4, 100);
        u.emergency = true;
        let partials: Vec<_> = shares.iter().zip(&keys).map(|((_, share), key)| sign_emergency_partial(&u, share, key)).collect();
        let all = combine(&sharing, &partials).unwrap();
        verify(sharing.public(), &all).unwrap();
        verify_emergency(&all, &committee).unwrap();
        let three = combine(&sharing, &partials[..3]).unwrap();
        verify_emergency(&three, &committee).unwrap();
        assert!(combine(&sharing, &partials[..2]).is_err(), "two BLS shares cannot sign");
        let mut two = three.clone();
        two.emergency_approvals.truncate(2);
        assert!(verify_emergency(&two, &committee).is_err(), "two independent approvals are insufficient even with a valid BLS signature");
        assert!(verify_emergency(&three, &[]).is_err());
        let mut changed = all.clone();
        changed.emergency_approvals[0].1 = all.emergency_approvals[1].1.clone();
        assert!(verify_emergency(&changed, &committee).is_err());
        changed = all.clone();
        changed.emergency_approvals[0].0 = all.emergency_approvals[1].0.clone();
        assert!(verify_emergency(&changed, &committee).is_err());
        changed = three.clone();
        changed.upgrade.activate_at += 1;
        assert!(verify_emergency(&changed, &committee).is_err());
        let mut duplicate_committee = committee.clone();
        duplicate_committee[3] = duplicate_committee[0].clone();
        assert!(verify_emergency(&three, &duplicate_committee).is_err());
        let mut foreign_committee = committee.clone();
        foreign_committee[0].0 = hex::encode(aether_light::devnet_validator_key(5).public_key().encode());
        assert!(verify_emergency(&three, &foreign_committee).is_err());
    }

    #[test]
    fn legacy_7780_emergency_verification_still_requires_every_member() {
        let (_, sharing, shares) = aether_light::devnet_threshold(4);
        let keys: Vec<_> = (1..=4).map(aether_light::devnet_validator_key).collect();
        let committee: Vec<_> = keys.iter().map(|key| (hex::encode(key.public_key().encode()), String::new())).collect();
        let mut u = upgrade(4, 100);
        u.chain_id = 7_780;
        u.emergency = true;
        let partials: Vec<_> = shares.iter().zip(&keys).map(|((_, share), key)| sign_emergency_partial(&u, share, key)).collect();
        verify_emergency(&combine(&sharing, &partials).unwrap(), &committee).unwrap();
        assert_eq!(verify_emergency(&combine(&sharing, &partials[..3]).unwrap(), &committee), Err("emergency upgrade needs every current committee member".into()));
    }

    #[test]
    #[cfg(feature = "dev-drill")]
    fn dev_overrides_only_accept_a_later_protocol_and_a_positive_notice() {
        // `implements`/`dev_notice` cache their first read, so exercise the
        // parsers directly instead of setting environment variables.
        assert_eq!(parse_dev_protocol(None), None);
        assert_eq!(parse_dev_protocol(Some("3")), None, "not lower or equal");
        assert_eq!(parse_dev_protocol(Some("2")), None);
        assert_eq!(parse_dev_protocol(Some("x")), None);
        assert_eq!(parse_dev_protocol(Some(" 4 ")), Some(4));
        assert_eq!(parse_dev_protocol(Some("9")), Some(9));
        assert_eq!(parse_dev_notice(None), None);
        assert_eq!(parse_dev_notice(Some("0")), None);
        assert_eq!(parse_dev_notice(Some("-5")), None);
        assert_eq!(parse_dev_notice(Some(" 30 ")), Some(30));
    }

    #[test]
    #[cfg(not(feature = "dev-drill"))]
    fn shipped_builds_have_no_drill_overrides() {
        // A shipped binary ignores the variables entirely: same protocol,
        // same notice, whatever the environment carries.
        std::env::set_var("AETHER_DEV_PROTOCOL", "9");
        std::env::set_var("AETHER_DEV_UPGRADE_NOTICE", "10");
        assert_eq!(implements(), PROTOCOL);
        assert_eq!(dev_notice(), None);
        std::env::remove_var("AETHER_DEV_PROTOCOL");
        std::env::remove_var("AETHER_DEV_UPGRADE_NOTICE");
    }
}
