//! Handing the committee key to a new voting set without stopping the chain
//! (docs/design/07-consensus.md, "open voting nodes").
//!
//! The running set keeps building blocks while old and new members reshare the
//! key in the background. When the reshare succeeds, the running committee signs
//! a `Handoff` (new public sharing and roster) with its threshold key, and a
//! block carries it. The switch happens `DELAY` blocks later, at a height every
//! node reads from the finalized chain: the running set finalizes up to it and
//! stops; the new set starts there under the same identity. A reshare that fails
//! changes nothing, so the running set can never fork with the new one.

use crate::dkg::KeyFile;
use crate::block::PublicKey;
use aether_light::block::Handoff;
use aether_light::Identity;
use commonware_codec::{DecodeExt, Encode};
use commonware_cryptography::bls12381::primitives::group::Share;
use commonware_cryptography::bls12381::primitives::ops;
use commonware_cryptography::bls12381::primitives::sharing::Sharing;
use commonware_cryptography::bls12381::primitives::variant::{MinSig, PartialSignature, Variant};
use commonware_parallel::Sequential;
use serde::{Deserialize, Serialize};
use commonware_utils::ordered::Quorum as _;

/// Blocks between the handoff block and the first block of the new set: time
/// for every old member to learn the switch height and for new members to
/// catch up to it (about a minute of 1 s blocks).
pub const DELAY: u64 = 64;
pub const READY_FILE: &str = "reshare-ready.json";
const NAMESPACE: &[u8] = b"aether-handoff-v1";
const SEED_NAMESPACE: &[u8] = b"aether-committee-seed-v1";
const READY_NAMESPACE: &[u8] = b"aether-share-ready-v1";

/// Public receipts collected over the authenticated DKG channel during a
/// bounded readiness window. The relay writes this even for an old-only dealer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Readiness {
    pub round: u64,
    pub output: String,
    pub members: Vec<(String, String)>,
    /// Validator key (hex) -> new-sharing partial signature (hex).
    pub proofs: std::collections::BTreeMap<String, String>,
}

impl Readiness {
    /// Accept only a proof for a seat in this exact sharing. A complete
    /// record can end the relay without waiting out its failure deadline.
    pub fn accept_proof(&mut self, chain_id: u64, member: &str, proof: &str) -> Result<bool, String> {
        let member = member.to_lowercase();
        if !self.members.iter().any(|(key, _)| key.eq_ignore_ascii_case(&member)) {
            return Err("ready proof is for a non-member".into());
        }
        check_ready(chain_id, self.round, &self.output, &self.members, &member, proof)?;
        self.proofs.insert(member, proof.to_string());
        Ok(self.members.iter().all(|(key, _)| self.proofs.contains_key(&key.to_lowercase())))
    }

    /// The deterministic retry roster consists of seats with valid receipts.
    /// A handoff is never signed for the failed round; a new DKG round is
    /// required because removing a player changes the public polynomial.
    pub fn retry_members(&self, chain_id: u64) -> Result<Option<Vec<(String, String)>>, String> {
        let ready: Vec<_> = self.members.iter().filter(|(key, _)| {
            self.proofs.get(&key.to_lowercase()).is_some_and(|proof|
                check_ready(chain_id, self.round, &self.output, &self.members, key, proof).is_ok())
        }).cloned().collect();
        if ready.len() == self.members.len() { return Ok(None); }
        if ready.len() < 4 { return Err("fewer than four ready seats; wait for a fresh chain proposal".into()); }
        if self.members.len() - ready.len() > (self.members.len() - 1) / 3 {
            return Err("too many unready seats for one bounded roster retry".into());
        }
        Ok(Some(ready))
    }
}

fn ready_message(chain_id: u64, round: u64, output: &str, members: &[(String, String)]) -> Vec<u8> {
    let output_digest = blake3::hash(&hex::decode(output).expect("checked output hex"));
    let roster_digest = blake3::hash(&serde_json::to_vec(members).expect("roster serializes"));
    serde_json::to_vec(&(chain_id, round, output_digest.as_bytes(), roster_digest.as_bytes(), "share-ready"))
        .expect("readiness message serializes")
}

/// Prove the staged share can produce a partial under this exact polynomial.
pub fn sign_ready(chain_id: u64, round: u64, output: &str, members: &[(String, String)], share: &Share) -> String {
    crate::key_binding::check_process();
    hex::encode(ops::threshold::sign_message::<MinSig>(share, READY_NAMESPACE, &ready_message(chain_id, round, output, members)).encode())
}

/// The partial must verify under the output and have the index assigned to
/// `member` in its ordered player set; a different player's share cannot be
/// passed off as this seat's proof.
pub fn check_ready(chain_id: u64, round: u64, output: &str, members: &[(String, String)], member: &str, proof: &str) -> Result<(), String> {
    let output = KeyFile { round, output: output.to_string(), identity: String::new(), share: String::new() }
        .decode_output(members.len() as u32)?;
    let pk = PublicKey::decode(hex::decode(member).map_err(|e| e.to_string())?.as_slice())
        .map_err(|e| format!("ready member: {e:?}"))?;
    let bytes = hex::decode(proof).map_err(|e| e.to_string())?;
    let partial = PartialSignature::<MinSig>::decode(bytes.as_slice()).map_err(|e| format!("ready partial: {e:?}"))?;
    if output.players().key(partial.index) != Some(&pk) {
        return Err("ready partial belongs to another seat".into());
    }
    ops::threshold::verify_message::<MinSig>(output.public(), READY_NAMESPACE,
        &ready_message(chain_id, round, &hex::encode(output.encode()), members), &partial)
        .map_err(|_| "ready partial does not match the new sharing".to_string())
}

fn seed_message(chain_id: u64, draw: u64) -> Vec<u8> {
    [chain_id.to_be_bytes(), draw.to_be_bytes()].concat()
}

/// The committee's signature on draw `draw`, checked under the identity.
pub fn verify_seed(chain_id: u64, identity: &Identity, s: &aether_light::block::Seed) -> Result<(), String> {
    let bytes = hex::decode(&s.signature).map_err(|e| e.to_string())?;
    let sig = <MinSig as Variant>::Signature::decode(bytes.as_slice()).map_err(|e| format!("seed: {e:?}"))?;
    ops::verify_message::<MinSig>(identity, SEED_NAMESPACE, &seed_message(chain_id, s.draw), &sig)
        .map_err(|_| "the committee did not sign this seed".to_string())
}

/// One running member's partial signature on draw `draw`'s seed (hex codec
/// bytes), as the committee actor produces them.
pub fn sign_seed_partial(chain_id: u64, draw: u64, share: &Share) -> String {
    crate::key_binding::check_process();
    hex::encode(ops::threshold::sign_message::<MinSig>(share, SEED_NAMESPACE, &seed_message(chain_id, draw)).encode())
}

/// Check one seed partial against the running sharing.
pub fn check_seed_partial(chain_id: u64, sharing: &Sharing<MinSig>, draw: u64, partial: &str) -> Result<PartialSignature<MinSig>, String> {
    let bytes = hex::decode(partial).map_err(|e| e.to_string())?;
    let p = PartialSignature::<MinSig>::decode(bytes.as_slice()).map_err(|e| format!("partial: {e:?}"))?;
    ops::threshold::verify_message::<MinSig>(sharing, SEED_NAMESPACE, &seed_message(chain_id, draw), &p).map_err(|_| format!("invalid seed partial from signer {}", p.index))?;
    Ok(p)
}

/// Combine checked partials into the committee's seed signature for `draw`.
pub fn combine_seed(sharing: &Sharing<MinSig>, draw: u64, partials: &[PartialSignature<MinSig>]) -> Result<aether_light::block::Seed, String> {
    let sig = ops::threshold::recover::<MinSig, _>(sharing, partials, &Sequential).map_err(|e| format!("need {} partials: {e:?}", sharing.required()))?;
    Ok(aether_light::block::Seed { draw, signature: hex::encode(sig.encode()) })
}

/// The committee's seed for draw `draw` from `shares` of its identity — what
/// `Service::tick` collects over gossip, done in one step (tests).
pub fn sign_seed(chain_id: u64, sharing: &Sharing<MinSig>, draw: u64, shares: &[&Share]) -> aether_light::block::Seed {
    let partials: Vec<_> = shares
        .iter()
        .map(|s| ops::threshold::sign_message::<MinSig>(s, SEED_NAMESPACE, &seed_message(chain_id, draw)))
        .collect();
    let sig = ops::threshold::recover::<MinSig, _>(sharing, &partials, &Sequential)
        .expect("a quorum of shares signs the seed");
    aether_light::block::Seed { draw, signature: hex::encode(sig.encode()) }
}

/// A handoff the chain accepted: it switches at `switch`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pending {
    /// Height of the block that carried it.
    pub at: u64,
    /// First height of the new voting set.
    pub switch: u64,
    pub handoff: Handoff,
}

/// What the committee signs: everything but the signature, bound to the chain.
fn message(chain_id: u64, h: &Handoff) -> Vec<u8> {
    if chain_id == 7_780 {
        serde_json::to_vec(&(chain_id, h.round, &h.output, &h.members)).expect("handoff serializes")
    } else {
        serde_json::to_vec(&(chain_id, h.round, &h.output, &h.members, &h.ready)).expect("handoff serializes")
    }
}

/// One running member's partial signature (hex codec bytes).
pub fn sign_partial(chain_id: u64, h: &Handoff, share: &Share) -> String {
    crate::key_binding::check_process();
    hex::encode(ops::threshold::sign_message::<MinSig>(share, NAMESPACE, &message(chain_id, h)).encode())
}

/// Check one partial against the running sharing.
pub fn check_partial(chain_id: u64, sharing: &Sharing<MinSig>, h: &Handoff, partial: &str) -> Result<PartialSignature<MinSig>, String> {
    let bytes = hex::decode(partial).map_err(|e| e.to_string())?;
    let p = PartialSignature::<MinSig>::decode(bytes.as_slice()).map_err(|e| format!("partial: {e:?}"))?;
    ops::threshold::verify_message::<MinSig>(sharing, NAMESPACE, &message(chain_id, h), &p).map_err(|_| format!("invalid partial from signer {}", p.index))?;
    Ok(p)
}

/// Combine checked partials into the committee signature on `h`.
pub fn combine(sharing: &Sharing<MinSig>, h: &Handoff, partials: &[PartialSignature<MinSig>]) -> Result<Handoff, String> {
    let sig = ops::threshold::recover::<MinSig, _>(sharing, partials, &Sequential).map_err(|e| format!("need {} partials: {e:?}", sharing.required()))?;
    Ok(Handoff { signature: hex::encode(sig.encode()), ..h.clone() })
}

/// A handoff is valid when the running committee signed it and its output is a
/// sharing of the same identity among exactly its members.
pub fn verify(chain_id: u64, identity: &Identity, h: &Handoff) -> Result<(), String> {
    let bytes = hex::decode(&h.signature).map_err(|e| e.to_string())?;
    let sig = <MinSig as Variant>::Signature::decode(bytes.as_slice()).map_err(|e| format!("signature: {e:?}"))?;
    ops::verify_message::<MinSig>(identity, NAMESPACE, &message(chain_id, h), &sig).map_err(|_| "the committee did not sign this handoff".to_string())?;
    verify_output(chain_id, identity, h)
}

/// The output shares the same identity among exactly the members.
pub fn verify_output(chain_id: u64, identity: &Identity, h: &Handoff) -> Result<(), String> {
    let n = u32::try_from(h.members.len()).map_err(|_| "too many members".to_string())?;
    if n < 4 {
        return Err("a voting set needs at least four members".into());
    }
    let output = KeyFile { round: h.round, output: h.output.clone(), identity: String::new(), share: String::new() }.decode_output(n)?;
    if chain_id != 7_780 && output.revealed().iter().any(|player| output.players().position(player).is_some()) {
        return Err("the new sharing reveals a seated player's threshold share".into());
    }
    if output.public().public() != identity {
        return Err("the new sharing is for another identity".into());
    }
    let mut players: Vec<String> = output.players().iter().map(|p| hex::encode(p.encode())).collect();
    let mut members: Vec<String> = h.members.iter().map(|(k, _)| k.to_lowercase()).collect();
    players.sort();
    members.sort();
    if players != members {
        return Err("the new sharing's players are not the members".into());
    }
    if chain_id != 7_780 {
        if h.ready.len() != h.members.len() {
            return Err("every new seat must provide a share-ready partial".into());
        }
        for ((key, _), proof) in h.members.iter().zip(&h.ready) {
            check_ready(chain_id, h.round, &h.output, &h.members, key, proof)?;
        }
    }
    Ok(())
}

/// A running member's partial signature, gossiped to the other validators.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PartialMsg {
    /// The handoff without its signature.
    pub handoff: Handoff,
    pub partial: String,
}

/// What running members gossip: handoff partials and draw-seed partials.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum CommitteeMsg {
    Handoff(PartialMsg),
    Seed { draw: u64, partial: String },
}

/// Each signer's latest partial: (signed message, partial).
type Partials = std::collections::BTreeMap<u32, (Vec<u8>, PartialSignature<MinSig>)>;

/// A failed seat may be removed only on new-genesis chains. Every retained
/// member must be from the chain's proposal with the same node identity.
pub(crate) fn roster_allowed(chain_id: u64, proposed: &[(String, String)], members: &[(String, String)]) -> bool {
    if chain_id == 7_780 {
        return proposed == members;
    }
    members.len() >= 4
        && members.len() <= proposed.len()
        && proposed.len() - members.len() <= proposed.len().saturating_sub(1) / 3
        && members.iter().all(|(key, node)| {
        proposed.iter().any(|(pkey, pnode)| pkey.eq_ignore_ascii_case(key) && pnode == node)
        })
}

/// Collects the running committee's partial signatures and, at the threshold,
/// puts the signed handoff up for the next proposer (`Chain::handoff_ready`).
pub struct Service {
    chain: crate::chain::Chain,
    chain_id: u64,
    identity: Identity,
    share: Share,
    sharing: Sharing<MinSig>,
    data: std::path::PathBuf,
    out: tokio::sync::mpsc::UnboundedSender<CommitteeMsg>,
    partials: std::sync::Mutex<Partials>,
    /// Draw-seed partials for the current draw, one per signer.
    seeds: std::sync::Mutex<(u64, std::collections::BTreeMap<u32, PartialSignature<MinSig>>)>,
    signed_seed: std::sync::Mutex<Option<(u64, std::time::Instant)>>,
}

impl Service {
    pub fn new(
        chain: crate::chain::Chain,
        chain_id: u64,
        share: Share,
        sharing: Sharing<MinSig>,
        data: std::path::PathBuf,
        out: tokio::sync::mpsc::UnboundedSender<CommitteeMsg>,
    ) -> Self {
        let identity = *sharing.public();
        Service {
            chain,
            chain_id,
            identity,
            share,
            sharing,
            data,
            out,
            partials: Default::default(),
            seeds: Default::default(),
            signed_seed: Default::default(),
        }
    }

    /// Take a gossiped message.
    pub fn receive(&self, m: &CommitteeMsg) -> Result<(), String> {
        match m {
            CommitteeMsg::Handoff(p) => self.accept(p),
            CommitteeMsg::Seed { draw, partial } => self.accept_seed(*draw, partial),
        }
    }

    /// While a draw's pool is frozen and its seed not yet on chain: sign it
    /// (again every 10 s, for peers that missed it). Called periodically.
    pub fn tick(&self) {
        let draw = {
            let g = self.chain.lock();
            match (&g.pool, &g.finalized.seed) {
                (Some((d, _)), seed) if seed.as_ref().is_none_or(|s| s.1.draw < *d) => *d,
                _ => return,
            }
        };
        let mut last = self.signed_seed.lock().expect("seed lock");
        if last.is_some_and(|(d, t)| d == draw && t.elapsed() < std::time::Duration::from_secs(10)) {
            return;
        }
        *last = Some((draw, std::time::Instant::now()));
        drop(last);
        let partial = sign_seed_partial(self.chain_id, draw, &self.share);
        if let Err(e) = self.accept_seed(draw, &partial) {
            tracing::debug!(%e, "own seed partial");
        }
        let _ = self.out.send(CommitteeMsg::Seed { draw, partial });
    }

    fn accept_seed(&self, draw: u64, partial: &str) -> Result<(), String> {
        let current = self.chain.lock().pool.as_ref().map(|(d, _)| *d);
        if current != Some(draw) {
            return Err("seed partial for another draw".into());
        }
        let bytes = hex::decode(partial).map_err(|e| e.to_string())?;
        let p = PartialSignature::<MinSig>::decode(bytes.as_slice()).map_err(|e| format!("partial: {e:?}"))?;
        let msg = seed_message(self.chain_id, draw);
        ops::threshold::verify_message::<MinSig>(&self.sharing, SEED_NAMESPACE, &msg, &p).map_err(|_| "invalid seed partial".to_string())?;
        let ready = {
            let mut g = self.seeds.lock().expect("seeds lock");
            if g.0 != draw {
                *g = (draw, Default::default());
            }
            g.1.insert(p.index.get(), p);
            (g.1.len() as u32 >= self.sharing.required()).then(|| g.1.values().cloned().collect::<Vec<_>>())
        };
        if let Some(ps) = ready {
            let sig = ops::threshold::recover::<MinSig, _>(&self.sharing, &ps, &Sequential).map_err(|e| format!("{e:?}"))?;
            let seed = aether_light::block::Seed { draw, signature: hex::encode(sig.encode()) };
            verify_seed(self.chain_id, &self.identity, &seed)?;
            let mut g = self.chain.lock();
            if g.seed_ready.as_ref() != Some(&seed) {
                tracing::info!(draw, "draw seed signed by the committee; next proposer includes it");
                g.seed_ready = Some(seed);
            }
        }
        Ok(())
    }

    /// Sign the handoff this validator's own background reshare produced, if it
    /// is to the voting set the chain proposed. Never signs anything else: an
    /// output nobody holds shares of would stop the chain at the switch.
    pub fn sign_staged(&self) -> Result<Handoff, String> {
        let h = if self.chain_id == 7_780 {
            let key: KeyFile = serde_json::from_slice(&std::fs::read(self.data.join(crate::rotation::STAGED_THRESHOLD))
                .map_err(|e| format!("no staged reshare: {e}"))?).map_err(|e| e.to_string())?;
            let net = crate::roster::NetworkFile::load(&self.data.join(crate::rotation::STAGED_NETWORK))?;
            Handoff { round: key.round, output: key.output,
                members: net.validators.iter().map(|m| (m.key.to_lowercase(), m.node.clone())).collect(),
                ready: vec![], signature: String::new() }
        } else {
            let record: Readiness = serde_json::from_slice(&std::fs::read(self.data.join(READY_FILE))
                .map_err(|e| format!("no completed readiness window: {e}"))?).map_err(|e| e.to_string())?;
            let ready = record.members.iter().map(|(key, _)| record.proofs.get(&key.to_lowercase()).cloned()
                .ok_or_else(|| format!("no share-ready proof for {key}"))).collect::<Result<Vec<_>, _>>()?;
            Handoff { round: record.round, output: record.output, members: record.members,
                ready, signature: String::new() }
        };
        let proposal = self.chain.lock().proposal.clone().ok_or("no voting set is proposed now")?;
        if !roster_allowed(self.chain_id, &proposal.1, &h.members) {
            return Err("the staged reshare is for another voting set".into());
        }
        verify_output(self.chain_id, &self.identity, &h)?;
        let partial = sign_partial(self.chain_id, &h, &self.share);
        self.accept(&PartialMsg { handoff: h.clone(), partial: partial.clone() })?;
        let _ = self.out.send(CommitteeMsg::Handoff(PartialMsg { handoff: h.clone(), partial }));
        Ok(h)
    }

    /// Take a partial (ours or a peer's); at the threshold, the handoff is ready.
    /// Only partials for the voting set this node computed, with a valid output,
    /// are kept, one per signer: memory stays bounded by the committee size.
    pub fn accept(&self, m: &PartialMsg) -> Result<(), String> {
        let h = Handoff { signature: String::new(), ..m.handoff.clone() };
        let proposal = self.chain.lock().proposal.clone().ok_or("no voting set is proposed now")?;
        if !roster_allowed(self.chain_id, &proposal.1, &h.members) {
            return Err("partial for another voting set".into());
        }
        verify_output(self.chain_id, &self.identity, &h)?;
        let p = check_partial(self.chain_id, &self.sharing, &h, &m.partial)?;
        let key = message(self.chain_id, &h);
        let ready = {
            let mut g = self.partials.lock().expect("partials lock");
            g.insert(p.index.get(), (key.clone(), p));
            let agreeing: Vec<PartialSignature<MinSig>> = g.values().filter(|(k, _)| *k == key).map(|(_, p)| p.clone()).collect();
            (agreeing.len() as u32 >= self.sharing.required()).then_some(agreeing)
        };
        if let Some(ps) = ready {
            let signed = combine(&self.sharing, &h, &ps)?;
            verify(self.chain_id, &self.identity, &signed)?;
            let mut g = self.chain.lock();
            if g.handoff_ready.as_ref() != Some(&signed) {
                tracing::info!(round = signed.round, members = signed.members.len(), "handoff signed by the committee; next proposer includes it");
                g.handoff_ready = Some(signed);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handoff_subset_never_adds_an_unproposed_seat_or_changes_legacy_bytes() {
        let proposed: Vec<_> = (0..5).map(|i| (format!("key{i}"), format!("node{i}"))).collect();
        let reduced = proposed[..4].to_vec();
        assert!(roster_allowed(7_781, &proposed, &reduced));
        assert!(!roster_allowed(7_780, &proposed, &reduced));
        assert!(!roster_allowed(7_781, &proposed, &proposed[..3]));
        let large: Vec<_> = (0..8).map(|i| (format!("key{i}"), format!("node{i}"))).collect();
        assert!(!roster_allowed(7_781, &large, &large[..5]), "a retry cannot discard more than one fault bound");
        let mut changed = reduced.clone();
        changed[0].1 = "another-node".into();
        assert!(!roster_allowed(7_781, &proposed, &changed));
        changed[0] = ("new-key".into(), "node0".into());
        assert!(!roster_allowed(7_781, &proposed, &changed));
        let legacy = Handoff { round: 1, output: "00".into(), members: reduced, ready: vec![], signature: String::new() };
        assert_eq!(message(7_780, &legacy), serde_json::to_vec(&(7_780u64, 1u64, "00", &legacy.members)).unwrap());
        assert!(!serde_json::to_string(&legacy).unwrap().contains("ready"));
    }
}
