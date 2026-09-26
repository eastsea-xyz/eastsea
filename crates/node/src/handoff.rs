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
use aether_light::block::Handoff;
use aether_light::Identity;
use commonware_codec::{DecodeExt, Encode};
use commonware_cryptography::bls12381::primitives::group::Share;
use commonware_cryptography::bls12381::primitives::ops;
use commonware_cryptography::bls12381::primitives::sharing::Sharing;
use commonware_cryptography::bls12381::primitives::variant::{MinSig, PartialSignature, Variant};
use commonware_parallel::Sequential;
use serde::{Deserialize, Serialize};

/// Blocks between the handoff block and the first block of the new set: time
/// for every old member to learn the switch height and for new members to
/// catch up to it (about a minute of 1 s blocks).
pub const DELAY: u64 = 64;
const NAMESPACE: &[u8] = b"aether-handoff-v1";

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
    serde_json::to_vec(&(chain_id, h.round, &h.output, &h.members)).expect("handoff serializes")
}

/// One running member's partial signature (hex codec bytes).
pub fn sign_partial(chain_id: u64, h: &Handoff, share: &Share) -> String {
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
    verify_output(identity, h)
}

/// The output shares the same identity among exactly the members.
pub fn verify_output(identity: &Identity, h: &Handoff) -> Result<(), String> {
    let n = u32::try_from(h.members.len()).map_err(|_| "too many members".to_string())?;
    if n < 4 {
        return Err("a voting set needs at least four members".into());
    }
    let output = KeyFile { round: h.round, output: h.output.clone(), identity: String::new(), share: String::new() }.decode_output(n)?;
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
    Ok(())
}

/// A running member's partial signature, gossiped to the other validators.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PartialMsg {
    /// The handoff without its signature.
    pub handoff: Handoff,
    pub partial: String,
}

/// Each signer's latest partial: (signed message, partial).
type Partials = std::collections::BTreeMap<u32, (Vec<u8>, PartialSignature<MinSig>)>;

/// Collects the running committee's partial signatures and, at the threshold,
/// puts the signed handoff up for the next proposer (`Chain::handoff_ready`).
pub struct Service {
    chain: crate::chain::Chain,
    chain_id: u64,
    identity: Identity,
    share: Share,
    sharing: Sharing<MinSig>,
    data: std::path::PathBuf,
    out: tokio::sync::mpsc::UnboundedSender<PartialMsg>,
    partials: std::sync::Mutex<Partials>,
}

impl Service {
    pub fn new(
        chain: crate::chain::Chain,
        chain_id: u64,
        share: Share,
        sharing: Sharing<MinSig>,
        data: std::path::PathBuf,
        out: tokio::sync::mpsc::UnboundedSender<PartialMsg>,
    ) -> Self {
        let identity = *sharing.public();
        Service { chain, chain_id, identity, share, sharing, data, out, partials: Default::default() }
    }

    /// Sign the handoff this validator's own background reshare produced, if it
    /// is to the voting set the chain proposed. Never signs anything else: an
    /// output nobody holds shares of would stop the chain at the switch.
    pub fn sign_staged(&self) -> Result<Handoff, String> {
        let key: KeyFile =
            serde_json::from_slice(&std::fs::read(self.data.join(crate::rotation::STAGED_THRESHOLD)).map_err(|e| format!("no staged reshare: {e}"))?)
                .map_err(|e| e.to_string())?;
        let net = crate::roster::NetworkFile::load(&self.data.join(crate::rotation::STAGED_NETWORK))?;
        let h = Handoff {
            round: key.round,
            output: key.output,
            members: net.validators.iter().map(|m| (m.key.to_lowercase(), m.node.clone())).collect(),
            signature: String::new(),
        };
        let proposal = self.chain.lock().proposal.clone().ok_or("no voting set is proposed now")?;
        if proposal.1 != h.members {
            return Err("the staged reshare is for another voting set".into());
        }
        verify_output(&self.identity, &h)?;
        let partial = sign_partial(self.chain_id, &h, &self.share);
        self.accept(&PartialMsg { handoff: h.clone(), partial: partial.clone() })?;
        let _ = self.out.send(PartialMsg { handoff: h.clone(), partial });
        Ok(h)
    }

    /// Take a partial (ours or a peer's); at the threshold, the handoff is ready.
    /// Only partials for the voting set this node computed, with a valid output,
    /// are kept, one per signer: memory stays bounded by the committee size.
    pub fn accept(&self, m: &PartialMsg) -> Result<(), String> {
        let h = Handoff { signature: String::new(), ..m.handoff.clone() };
        let proposal = self.chain.lock().proposal.clone().ok_or("no voting set is proposed now")?;
        if proposal.1 != h.members {
            return Err("partial for another voting set".into());
        }
        verify_output(&self.identity, &h)?;
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
