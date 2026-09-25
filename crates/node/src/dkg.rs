//! Distributed key generation for the consensus committee (design D8).
//!
//! Joint-Feldman DKG from Commonware (`feldman_desmedt`): every validator is
//! both a dealer and a player. No party ever learns the group secret; each
//! validator ends with its own share and everyone with the same public
//! polynomial (whose constant term is the committee identity wallets trust).
//!
//! This module is the transport-free state machine: feed it messages, send what
//! it returns. Messages ride Commonware's authenticated, encrypted p2p, so the
//! private dealings are only readable by their player.
//!
//! Genesis ceremony rules (simple and safe, not live): every dealer's log is
//! required; logs are relayed so that a log seen by one honest node reaches
//! all; a dealer caught signing two different logs is excluded; the run ends
//! with every validator announcing its computed identity, and the result is
//! only accepted if all announcements agree.

use crate::block::PublicKey;
use aether_light::Identity;
use commonware_codec::{Decode, DecodeExt, Encode};
use commonware_cryptography::bls12381::dkg::feldman_desmedt::{
    Dealer, DealerPrivMsg, DealerPubMsg, Info, Logs, Output, Player, PlayerAck, Reveal, SignedDealerLog,
};
use commonware_cryptography::bls12381::primitives::group::Share;
use commonware_cryptography::bls12381::primitives::sharing::{Mode, ModeVersion};
use commonware_cryptography::bls12381::primitives::variant::MinSig;
use commonware_cryptography::{ed25519, Signer as _};
use commonware_utils::{
    ordered::{Quorum as _, Set},
    N3f1,
};
use rand::CryptoRng;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU32;

pub const DKG_NAMESPACE: &[u8] = b"_AETHER_DEVNET_V1_DKG";

pub type DkgOutput = Output<MinSig, PublicKey>;

/// Wire messages (serde JSON; binary fields are codec bytes).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Msg {
    /// Dealer -> one player: commitment and that player's private evaluation.
    Deal { commitment: Vec<u8>, dealing: Vec<u8> },
    /// Player -> dealer.
    Ack(Vec<u8>),
    /// A dealer's signed log (broadcast, and relayed by everyone).
    Log { dealer: Vec<u8>, log: Vec<u8> },
    /// The identity this validator computed.
    Done(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum To {
    One(PublicKey),
    All,
}

#[derive(Debug)]
pub enum DkgError {
    Setup(String),
    Finalize(String),
    /// Validators computed different identities: the ceremony must be rerun.
    Disagreement,
}

impl std::fmt::Display for DkgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}
impl std::error::Error for DkgError {}

/// One DKG or reshare round: who deals, who receives shares.
#[derive(Clone)]
pub struct Round {
    pub round: u64,
    /// For a reshare: the committee's current output (its public polynomial).
    pub previous: Option<DkgOutput>,
    pub dealers: Set<PublicKey>,
    pub players: Set<PublicKey>,
}

impl Round {
    /// Fresh key: every participant deals and receives.
    pub fn dkg(participants: Set<PublicKey>, round: u64) -> Self {
        Round { round, previous: None, dealers: participants.clone(), players: participants }
    }

    /// Move the existing key to `players`: the current share holders deal. The
    /// committee identity (what wallets pin) is unchanged.
    pub fn reshare(previous: DkgOutput, players: Set<PublicKey>, round: u64) -> Self {
        Round { round, dealers: previous.players().clone(), previous: Some(previous), players }
    }

    fn identity(&self) -> Option<Identity> {
        self.previous.as_ref().map(|o| *o.public().public())
    }
}

pub struct Ceremony {
    info: Info<MinSig, PublicKey>,
    me: PublicKey,
    n: NonZeroU32,
    dealers: Set<PublicKey>,
    players: Set<PublicKey>,
    /// A reshare must reproduce this identity.
    expected: Option<Identity>,
    dealer: Option<Dealer<MinSig, ed25519::PrivateKey>>,
    player: Option<Player<MinSig, ed25519::PrivateKey>>,
    /// Our deal messages, re-sent until acknowledged.
    deals: BTreeMap<PublicKey, Msg>,
    acked: BTreeSet<PublicKey>,
    /// Acks we sent, per dealer, re-sent when a deal arrives again (acks can be lost).
    acks_sent: BTreeMap<PublicKey, Msg>,
    /// Signed log bytes per dealer (first version seen).
    logs: BTreeMap<PublicKey, Vec<u8>>,
    equivocators: BTreeSet<PublicKey>,
    identity: Option<Identity>,
    announced: BTreeMap<PublicKey, Vec<u8>>,
}

impl Ceremony {
    /// Start `round` in whatever roles `key` has in it (dealer, player or both).
    /// `share` is our current share when we deal in a reshare.
    pub fn start(rng: impl CryptoRng, key: ed25519::PrivateKey, round: Round, share: Option<Share>) -> Result<(Self, Vec<(To, Msg)>), DkgError> {
        let setup = |e: &dyn std::fmt::Debug| DkgError::Setup(format!("{e:?}"));
        let expected = round.identity();
        let info =
            Info::new::<N3f1>(DKG_NAMESPACE, round.round, round.previous, Mode::NonZeroCounter, Reveal::V1, round.dealers.clone(), round.players.clone())
                .map_err(|e| setup(&e))?;
        let n = NonZeroU32::new(round.players.len().max(round.dealers.len()) as u32).ok_or_else(|| DkgError::Setup("no participants".into()))?;
        let me = key.public_key();
        let is_dealer = round.dealers.position(&me).is_some();
        let is_player = round.players.position(&me).is_some();
        if !is_dealer && !is_player {
            return Err(DkgError::Setup("this key has no role in the round".into()));
        }
        if is_dealer && expected.is_some() && share.is_none() {
            return Err(DkgError::Setup("a resharing dealer needs its current share".into()));
        }
        let player = if is_player { Some(Player::new(info.clone(), key.clone()).map_err(|e| setup(&e))?) } else { None };
        let dealt = if is_dealer { Some(Dealer::start::<N3f1>(rng, info.clone(), key, share).map_err(|e| setup(&e))?) } else { None };
        let mut c = Ceremony {
            info,
            me: me.clone(),
            n,
            dealers: round.dealers,
            players: round.players,
            expected,
            dealer: None,
            player,
            deals: BTreeMap::new(),
            acked: BTreeSet::new(),
            acks_sent: BTreeMap::new(),
            logs: BTreeMap::new(),
            equivocators: BTreeSet::new(),
            identity: None,
            announced: BTreeMap::new(),
        };
        let mut out = Vec::new();
        if let Some((dealer, commitment, dealings)) = dealt {
            c.dealer = Some(dealer);
            let commitment = commitment.encode().to_vec();
            for (player, dealing) in dealings {
                let msg = Msg::Deal { commitment: commitment.clone(), dealing: dealing.encode().to_vec() };
                if player == me {
                    out.extend(c.on_message(&me, msg));
                } else {
                    c.deals.insert(player.clone(), msg.clone());
                    out.push((To::One(player), msg));
                }
            }
        }
        Ok((c, out))
    }

    pub fn is_player(&self) -> bool {
        self.players.position(&self.me).is_some()
    }

    /// Deals not yet acknowledged (re-send periodically: peers may not be connected yet).
    pub fn pending_deals(&self) -> Vec<(To, Msg)> {
        self.deals.iter().filter(|(p, _)| !self.acked.contains(*p)).map(|(p, m)| (To::One(p.clone()), m.clone())).collect()
    }

    /// A dealer has every player's ack (or is not a dealer).
    pub fn all_acked(&self) -> bool {
        self.dealer.is_none() || self.acked.len() == self.players.len()
    }

    /// Stop dealing (all acks in, or timed out): sign and broadcast our log.
    pub fn close_dealing(&mut self) -> Vec<(To, Msg)> {
        let Some(dealer) = self.dealer.take() else { return vec![] };
        let signed = dealer.finalize::<N3f1>();
        let msg = Msg::Log { dealer: self.me.encode().to_vec(), log: signed.encode().to_vec() };
        let mut out = self.on_message(&self.me.clone(), msg.clone());
        out.push((To::All, msg));
        out
    }

    /// Everything we know, for periodic re-broadcast (logs and our announcement).
    pub fn rebroadcast(&self) -> Vec<(To, Msg)> {
        let mut out: Vec<(To, Msg)> = self.logs.iter().map(|(d, log)| (To::All, Msg::Log { dealer: d.encode().to_vec(), log: log.clone() })).collect();
        if let Some(id) = &self.identity {
            out.push((To::All, Msg::Done(id.encode().to_vec())));
        }
        out
    }

    pub fn have_all_logs(&self) -> bool {
        self.logs.len() == self.dealers.len()
    }

    /// Enough dealer logs to finish (a quorum of dealers), e.g. when one old
    /// validator is offline during a reshare.
    pub fn have_quorum_logs(&self) -> bool {
        self.logs.keys().filter(|d| !self.equivocators.contains(*d)).count() >= self.dealers.quorum::<N3f1>() as usize
    }

    pub fn log_count(&self) -> usize {
        self.logs.len()
    }

    /// Handle one message from authenticated peer `from`.
    pub fn on_message(&mut self, from: &PublicKey, msg: Msg) -> Vec<(To, Msg)> {
        match msg {
            Msg::Deal { commitment, dealing } => {
                let Some(player) = self.player.as_mut() else { return vec![] };
                let (Ok(commitment), Ok(dealing)) =
                    (DealerPubMsg::<MinSig>::decode_cfg(commitment.as_slice(), &self.n), DealerPrivMsg::decode(dealing.as_slice()))
                else {
                    return vec![];
                };
                match player.dealer_message::<N3f1>(from.clone(), commitment, dealing) {
                    Ok(Some(ack)) if *from == self.me => {
                        self.self_ack(ack);
                        vec![]
                    }
                    Ok(Some(ack)) => {
                        let msg = Msg::Ack(ack.encode().to_vec());
                        self.acks_sent.insert(from.clone(), msg.clone());
                        vec![(To::One(from.clone()), msg)]
                    }
                    // Duplicate deal: the dealer did not get our ack, send it again.
                    Ok(None) => self.acks_sent.get(from).map(|m| vec![(To::One(from.clone()), m.clone())]).unwrap_or_default(),
                    Err(_) => vec![],
                }
            }
            Msg::Ack(bytes) => {
                let Ok(ack) = PlayerAck::<PublicKey>::decode(bytes.as_slice()) else { return vec![] };
                if let Some(d) = self.dealer.as_mut() {
                    if d.receive_player_ack(from.clone(), ack).is_ok() {
                        self.acked.insert(from.clone());
                    }
                }
                vec![]
            }
            Msg::Log { dealer, log } => self.on_log(dealer, log),
            Msg::Done(id) => {
                if self.players.position(from).is_some() {
                    self.announced.insert(from.clone(), id);
                }
                vec![]
            }
        }
    }

    fn self_ack(&mut self, ack: PlayerAck<PublicKey>) {
        if let Some(d) = self.dealer.as_mut() {
            if d.receive_player_ack(self.me.clone(), ack).is_ok() {
                self.acked.insert(self.me.clone());
            }
        }
    }

    fn on_log(&mut self, dealer: Vec<u8>, log: Vec<u8>) -> Vec<(To, Msg)> {
        let Ok(dealer_pk) = PublicKey::decode(dealer.as_slice()) else { return vec![] };
        let Ok(signed) = SignedDealerLog::<MinSig, ed25519::PrivateKey>::decode_cfg(log.as_slice(), &self.n) else { return vec![] };
        // Authenticate: signed by the dealer it claims, for this round.
        match signed.check(&self.info) {
            Some((pk, _)) if pk == dealer_pk => {}
            _ => return vec![],
        }
        match self.logs.get(&dealer_pk) {
            None => {
                self.logs.insert(dealer_pk, log.clone());
                vec![(To::All, Msg::Log { dealer, log })] // relay once
            }
            Some(existing) if *existing != log => {
                // Two different signed logs: provable equivocation. Relay the
                // second version so every honest node excludes this dealer too.
                if self.equivocators.insert(dealer_pk) {
                    vec![(To::All, Msg::Log { dealer, log })]
                } else {
                    vec![]
                }
            }
            Some(_) => vec![],
        }
    }

    /// Compute our share from all dealer logs (minus equivocators).
    pub fn finish(&mut self, rng: &mut impl CryptoRng) -> Result<(DkgOutput, Share), DkgError> {
        let player = self.player.take().ok_or_else(|| DkgError::Finalize("already finished".into()))?;
        let mut logs = Logs::<MinSig, PublicKey, N3f1>::new(self.info.clone());
        for (dealer, bytes) in &self.logs {
            if self.equivocators.contains(dealer) {
                continue;
            }
            let signed = SignedDealerLog::<MinSig, ed25519::PrivateKey>::decode_cfg(bytes.as_slice(), &self.n).expect("stored logs decode");
            if let Some((pk, log)) = signed.check(&self.info) {
                logs.record(pk, log);
            }
        }
        let (output, share) =
            player.finalize::<N3f1, ed25519::Batch>(rng, logs, &commonware_parallel::Sequential).map_err(|e| DkgError::Finalize(format!("{e:?}")))?;
        let identity = *output.public().public();
        if self.expected.is_some_and(|e| e != identity) {
            return Err(DkgError::Finalize("reshare changed the committee identity".into()));
        }
        self.identity = Some(identity);
        self.announced.insert(self.me.clone(), identity.encode().to_vec());
        Ok((output, share))
    }

    /// `Some(true)` once every player announced the same identity as ours (a
    /// dealer-only node compares with the committee's existing identity),
    /// `Some(false)` on any mismatch, `None` while waiting.
    pub fn agreement(&self) -> Option<bool> {
        let mine = self.identity.or(if self.is_player() { None } else { self.expected })?.encode().to_vec();
        if self.announced.values().any(|id| *id != mine) {
            return Some(false);
        }
        (self.announced.len() == self.players.len()).then_some(true)
    }
}

/// What a validator keeps after the ceremony. `share` is secret.
#[derive(Serialize, Deserialize)]
pub struct KeyFile {
    pub round: u64,
    /// Codec bytes (hex) of the DKG output: participants and public polynomial.
    pub output: String,
    /// Committee identity (hex), what wallets pin.
    pub identity: String,
    /// This validator's secret share (hex). Never share this file.
    pub share: String,
}

impl KeyFile {
    pub fn new(round: u64, output: &DkgOutput, share: &Share) -> Self {
        KeyFile { round, output: hex::encode(output.encode()), identity: hex::encode(output.public().public().encode()), share: hex::encode(share.encode()) }
    }

    /// Only the public output (for validators joining in a reshare).
    pub fn decode_output(&self, n: u32) -> Result<DkgOutput, String> {
        let n = NonZeroU32::new(n).ok_or("n = 0")?;
        let out = hex::decode(&self.output).map_err(|e| e.to_string())?;
        DkgOutput::decode_cfg(out.as_slice(), &(n, ModeVersion::v0())).map_err(|e| format!("output: {e:?}"))
    }

    pub fn decode(&self, n: u32) -> Result<(DkgOutput, Share), String> {
        let n = NonZeroU32::new(n).ok_or("n = 0")?;
        let out = hex::decode(&self.output).map_err(|e| e.to_string())?;
        let output = DkgOutput::decode_cfg(out.as_slice(), &(n, ModeVersion::v0())).map_err(|e| format!("output: {e:?}"))?;
        let share = Share::decode(hex::decode(&self.share).map_err(|e| e.to_string())?.as_slice()).map_err(|e| format!("share: {e:?}"))?;
        Ok((output, share))
    }
}

/// How long dealers wait for acks before signing their logs (then unacked
/// players get revealed shares), and the overall ceremony deadline.
pub struct Timeouts {
    pub dealing: std::time::Duration,
    pub total: std::time::Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Timeouts { dealing: std::time::Duration::from_secs(30), total: std::time::Duration::from_secs(300) }
    }
}

fn send_all<S: commonware_p2p::Sender<PublicKey = PublicKey>>(sender: &mut S, out: Vec<(To, Msg)>) {
    for (to, msg) in out {
        let bytes = serde_json::to_vec(&msg).expect("dkg message serializes");
        let recipients = match to {
            To::One(p) => commonware_p2p::Recipients::One(p),
            To::All => commonware_p2p::Recipients::All,
        };
        let _ = sender.send(recipients, bytes, false);
    }
}

/// Run the ceremony over a p2p channel. Returns this validator's output and share
/// once every participant announced the same identity.
pub async fn run<S, R>(
    key: ed25519::PrivateKey,
    round: Round,
    share: Option<Share>,
    mut sender: S,
    mut receiver: R,
    timeouts: Timeouts,
) -> Result<Option<(DkgOutput, Share)>, DkgError>
where
    S: commonware_p2p::Sender<PublicKey = PublicKey>,
    R: commonware_p2p::Receiver<PublicKey = PublicKey>,
{
    use std::time::{Duration, Instant};
    let mut rng = commonware_utils::sys_rng();
    let (mut c, out) = Ceremony::start(commonware_utils::sys_rng(), key, round, share)?;
    send_all(&mut sender, out);
    let start = Instant::now();
    let mut result = None;
    let mut logs_stable_since = Instant::now();
    let mut log_count = 0;
    let mut agreed_at: Option<Instant> = None;
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    loop {
        tokio::select! {
            r = receiver.recv() => {
                let Ok((from, msg)) = r else { return Err(DkgError::Finalize("p2p closed".into())) };
                if let Ok(m) = serde_json::from_slice::<Msg>(msg.as_ref()) {
                    let out = c.on_message(&from, m);
                    send_all(&mut sender, out);
                }
            }
            _ = tick.tick() => {
                let elapsed = start.elapsed();
                let mut out = c.pending_deals();
                if c.all_acked() || elapsed > timeouts.dealing {
                    out.extend(c.close_dealing());
                }
                if c.log_count() != log_count {
                    log_count = c.log_count();
                    logs_stable_since = Instant::now();
                }
                // All logs in (or, after the dealing window, a quorum of them) and
                // no new version for a moment (equivocation window).
                let enough = c.have_all_logs() || (elapsed > timeouts.dealing + Duration::from_secs(10) && c.have_quorum_logs());
                if result.is_none() && c.is_player() && enough && logs_stable_since.elapsed() > Duration::from_secs(2) {
                    let (o, s) = c.finish(&mut rng)?;
                    tracing::info!(identity = %hex::encode(o.public().public().encode()), "dkg: computed share; waiting for agreement");
                    result = Some((o, s));
                }
                out.extend(c.rebroadcast());
                send_all(&mut sender, out);
                match c.agreement() {
                    Some(false) => return Err(DkgError::Disagreement),
                    // Keep announcing briefly so slower peers also see agreement.
                    Some(true) => match agreed_at {
                        None => agreed_at = Some(Instant::now()),
                        Some(t) if t.elapsed() > Duration::from_secs(5) => return Ok(result),
                        _ => {}
                    },
                    None => {}
                }
                if elapsed > timeouts.total {
                    return Err(DkgError::Finalize(format!(
                        "timed out: {} of {} logs, {} acks",
                        c.log_count(),
                        c.dealers.len(),
                        c.acked.len()
                    )));
                }
            }
        }
    }
}
