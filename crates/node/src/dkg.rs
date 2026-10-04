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
//! New-genesis ceremonies agree on an exact, validated bundle of signed dealer
//! logs before finalizing a share. Consensus votes are signed and transferable;
//! a delayed player can replay its private dealings against the decided bundle.
//! The legacy testnet ceremony keeps its original identity announcement and
//! completion path so an in-flight 7780 round remains wire-compatible.

use crate::block::PublicKey;
use crate::dkg_agreement::{Agreement, AgreementMsg, BASE_VIEW_TICKS, MAX_VIEW_BACKOFF};
use aether_light::Identity;
use commonware_codec::{Decode, DecodeExt, Encode};
use commonware_cryptography::bls12381::dkg::feldman_desmedt::{
    observe, Dealer, DealerPrivMsg, DealerPubMsg, Info, Logs, Output, Player, PlayerAck, Reveal, SignedDealerLog,
};
use commonware_cryptography::bls12381::primitives::group::Share;
use commonware_cryptography::bls12381::primitives::sharing::{Mode, ModeVersion};
use commonware_cryptography::bls12381::primitives::variant::MinSig;
use commonware_cryptography::{ed25519, Signer as _};
use commonware_utils::{
    ordered::{Quorum as _, Set},
    N3f1,
};
use rand::{CryptoRng, RngExt as _, SeedableRng};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::num::NonZeroU32;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;

pub const DKG_NAMESPACE: &[u8] = b"_AETHER_DEVNET_V1_DKG";
const CHAIN_DKG_NAMESPACE: &[u8] = b"_AETHER_CHAIN_V1_DKG_";
const TICK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);
const QUORUM_LOG_DELAY: std::time::Duration = std::time::Duration::from_secs(10);
const LOG_SETTLE_TIME: std::time::Duration = std::time::Duration::from_secs(2);
const LEGACY_QUORUM_DONE_DELAY: std::time::Duration = std::time::Duration::from_secs(20);
const LEGACY_DONE_RELAY: std::time::Duration = std::time::Duration::from_secs(5);
/// Keep the certified decision on the live channel briefly before staging.
/// Later arrivals use the journal-backed relay after staging.
const STRICT_DECISION_GRACE: std::time::Duration = std::time::Duration::from_secs(5);
const CERTIFIED_TRANSCRIPT_RELAY: std::time::Duration = std::time::Duration::from_secs(30);
/// Keep a staged, certified round reachable while a late player catches up.
pub const POST_STAGE_RELAY: std::time::Duration = std::time::Duration::from_secs(60);

fn strict_decision_grace_elapsed(agreed_at: Option<std::time::Instant>, now: std::time::Instant) -> bool {
    agreed_at.is_some_and(|decided| now.duration_since(decided) > STRICT_DECISION_GRACE)
}

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
    /// Legacy identity or the digest of the complete public output and dealers.
    Done(Vec<u8>),
    /// New-genesis proposal: the exact signed dealer logs to use for finalization.
    Transcript(Vec<(Vec<u8>, Vec<u8>)>),
    /// Certified new-genesis transcript consensus control message.
    Agreement(AgreementMsg),
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
    /// Validators computed different public outputs: the ceremony must be rerun.
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
    strict_agreement: bool,
    chain_id: Option<u64>,
}

impl Round {
    /// Fresh key: every participant deals and receives.
    pub fn dkg(participants: Set<PublicKey>, round: u64) -> Self {
        Round { round, previous: None, dealers: participants.clone(), players: participants, strict_agreement: true, chain_id: None }
    }

    /// Move the existing key to `players`: the current share holders deal. The
    /// committee identity (what wallets pin) is unchanged.
    pub fn reshare(previous: DkgOutput, players: Set<PublicKey>, round: u64) -> Self {
        Round { round, dealers: previous.players().clone(), previous: Some(previous), players, strict_agreement: true, chain_id: None }
    }

    /// Bind a new-genesis ceremony to its chain. Production call sites must
    /// set this before `start` or `run_with_journal`; test constructors retain
    /// the historical namespace by default.
    pub fn with_chain_id(mut self, chain_id: u64) -> Self {
        if self.strict_agreement { self.chain_id = Some(chain_id); }
        self
    }

    /// Existing 7780 rounds retain their original DKG control-message bytes.
    /// The call site must select this only for chain 7780.
    pub fn legacy_agreement(mut self) -> Self {
        self.strict_agreement = false;
        self.chain_id = None;
        self
    }

    fn namespace(&self) -> Vec<u8> {
        match (self.strict_agreement, self.chain_id) {
            (true, Some(chain_id)) => {
                let mut namespace = Vec::with_capacity(CHAIN_DKG_NAMESPACE.len() + 8);
                namespace.extend_from_slice(CHAIN_DKG_NAMESPACE);
                namespace.extend_from_slice(&chain_id.to_be_bytes());
                namespace
            }
            _ => DKG_NAMESPACE.to_vec(),
        }
    }

    fn context_digest(&self) -> [u8; 32] {
        let mut hash = blake3::Hasher::new_derive_key("aether DKG full round context v1");
        let namespace = self.namespace();
        hash.update(&(namespace.len() as u64).to_be_bytes());
        hash.update(&namespace);
        match self.chain_id {
            Some(chain_id) if self.strict_agreement => { hash.update(&[1]); hash.update(&chain_id.to_be_bytes()); }
            _ => { hash.update(&[0]); }
        }
        hash.update(&self.round.to_be_bytes());
        match &self.previous {
            Some(previous) => {
                let bytes = previous.encode();
                hash.update(&[1]);
                hash.update(&(bytes.len() as u64).to_be_bytes());
                hash.update(&bytes);
            }
            None => { hash.update(&[0]); }
        }
        hash.update(&(self.dealers.len() as u64).to_be_bytes());
        for dealer in self.dealers.iter() { hash.update(&dealer.encode()); }
        hash.update(&(self.players.len() as u64).to_be_bytes());
        for player in self.players.iter() { hash.update(&player.encode()); }
        *hash.finalize().as_bytes()
    }

    fn identity(&self) -> Option<Identity> {
        self.previous.as_ref().map(|o| *o.public().public())
    }
}

/// Append-only private-dealing journal. Records are synced before an Ack or
/// signed dealer log can leave the process, so restart cannot lose a dealing
/// which the public transcript says this player acknowledged.
struct DealJournal {
    file: File,
    seed: [u8; 32],
    accepted: BTreeMap<PublicKey, (Vec<u8>, Vec<u8>)>,
    own_log: Option<Vec<u8>>,
    decided_transcript: Option<Vec<(Vec<u8>, Vec<u8>)>>,
}

#[derive(Serialize, Deserialize)]
enum DealRecord {
    Header { binding: [u8; 32], seed: [u8; 32] },
    Accepted { dealer: Vec<u8>, commitment: Vec<u8>, dealing: Vec<u8> },
    OwnLog { log: Vec<u8> },
    DecidedTranscript { entries: Vec<(Vec<u8>, Vec<u8>)> },
}

#[derive(Serialize, Deserialize)]
struct CheckedDealRecord {
    record: DealRecord,
    checksum: [u8; 32],
}

impl DealJournal {
    fn binding(key: &ed25519::PrivateKey, round: &Round) -> [u8; 32] {
        let mut hash = blake3::Hasher::new_derive_key("aether private DKG journal binding v1");
        hash.update(&round.context_digest());
        hash.update(&key.public_key().encode());
        *hash.finalize().as_bytes()
    }

    fn checksum(record: &DealRecord) -> Result<[u8; 32], String> {
        let bytes = serde_json::to_vec(record).map_err(|e| format!("encode DKG deal journal: {e}"))?;
        Ok(*blake3::Hasher::new_derive_key("aether private DKG journal record v1").update(&bytes).finalize().as_bytes())
    }

    fn append(&mut self, record: DealRecord) -> Result<(), String> {
        let checksum = Self::checksum(&record)?;
        let mut bytes = serde_json::to_vec(&CheckedDealRecord { record, checksum }).map_err(|e| format!("encode DKG deal journal: {e}"))?;
        bytes.push(b'\n');
        self.file.write_all(&bytes).map_err(|e| format!("write DKG deal journal: {e}"))?;
        self.file.sync_all().map_err(|e| format!("sync DKG deal journal: {e}"))
    }

    fn open(path: PathBuf, binding: [u8; 32]) -> Result<Self, String> {
        let (mut file, fresh) = match OpenOptions::new().read(true).append(true).create_new(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(&path) {
            Ok(file) => (file, true),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists =>
                (OpenOptions::new().read(true).append(true).custom_flags(libc::O_NOFOLLOW).open(&path).map_err(|e| format!("open DKG deal journal: {e}"))?, false),
            Err(e) => return Err(format!("create DKG deal journal: {e}")),
        };
        if file.metadata().map_err(|e| format!("stat DKG deal journal: {e}"))?.permissions().mode() & 0o077 != 0 {
            return Err("DKG deal journal must have mode 0600".into());
        }
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(format!("DKG deal journal is already in use: {}", path.display()));
        }
        let mut seed = [0; 32];
        let mut accepted = BTreeMap::new();
        let mut own_log = None;
        let mut decided_transcript = None;
        if fresh {
            commonware_utils::sys_rng().fill(&mut seed);
            let mut journal = Self { file, seed, accepted, own_log, decided_transcript };
            journal.append(DealRecord::Header { binding, seed })?;
            let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| std::path::Path::new("."));
            File::open(parent).and_then(|dir| dir.sync_all()).map_err(|e| format!("sync DKG deal journal directory: {e}"))?;
            return Ok(journal);
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(|e| format!("read DKG deal journal: {e}"))?;
        let valid_len = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
        if valid_len != bytes.len() {
            file.set_len(valid_len as u64).map_err(|e| format!("truncate incomplete DKG deal record: {e}"))?;
            file.sync_all().map_err(|e| format!("sync truncated DKG deal journal: {e}"))?;
        }
        let mut header_seen = false;
        for line in bytes[..valid_len].split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
            let checked: CheckedDealRecord = serde_json::from_slice(line).map_err(|e| format!("decode DKG deal journal: {e}"))?;
            if Self::checksum(&checked.record)? != checked.checksum { return Err("DKG deal journal checksum mismatch".into()); }
            match checked.record {
                DealRecord::Header { binding: found, seed: found_seed } if !header_seen && found == binding => {
                    seed = found_seed;
                    header_seen = true;
                }
                DealRecord::Accepted { dealer, commitment, dealing } if header_seen => {
                    let dealer = PublicKey::decode(dealer.as_slice()).map_err(|e| format!("journal dealer: {e:?}"))?;
                    match accepted.insert(dealer, (commitment.clone(), dealing.clone())) {
                        Some(previous) if previous != (commitment, dealing) => return Err("conflicting persisted private dealing".into()),
                        _ => {}
                    }
                }
                DealRecord::OwnLog { log } if header_seen => {
                    if own_log.as_ref().is_some_and(|old| *old != log) { return Err("conflicting persisted dealer log".into()); }
                    own_log = Some(log);
                }
                DealRecord::DecidedTranscript { entries } if header_seen => {
                    if decided_transcript.as_ref().is_some_and(|old| *old != entries) { return Err("conflicting persisted certified transcript".into()); }
                    decided_transcript = Some(entries);
                }
                _ => return Err("DKG deal journal belongs to a different key, round, or roster".into()),
            }
        }
        if !header_seen { return Err("missing DKG deal journal header".into()); }
        Ok(Self { file, seed, accepted, own_log, decided_transcript })
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
    strict_agreement: bool,
    round: u64,
    dealer: Option<Dealer<MinSig, ed25519::PrivateKey>>,
    player: Option<Player<MinSig, ed25519::PrivateKey>>,
    key: ed25519::PrivateKey,
    /// Our deal messages, re-sent until acknowledged.
    deals: BTreeMap<PublicKey, Msg>,
    acked: BTreeSet<PublicKey>,
    /// Acks we sent, per dealer, re-sent when a deal arrives again (acks can be lost).
    acks_sent: BTreeMap<PublicKey, Msg>,
    /// Valid private dealings retained so a delayed player can adopt a decided transcript.
    accepted_deals: BTreeMap<PublicKey, (Vec<u8>, Vec<u8>)>,
    /// Signed log bytes per dealer (first version seen).
    logs: BTreeMap<PublicKey, Vec<u8>>,
    equivocators: BTreeSet<PublicKey>,
    equivocation_logs: BTreeMap<PublicKey, Vec<u8>>,
    log_revision: u64,
    identity: Option<Identity>,
    output_digest: Option<Vec<u8>>,
    announced: BTreeMap<PublicKey, Vec<u8>>,
    transcripts: BTreeMap<Vec<u8>, BTreeMap<PublicKey, Vec<u8>>>,
    transcript_counts: BTreeMap<PublicKey, usize>,
    decided: Option<Vec<u8>>,
    agreement: Option<Agreement>,
    pending_agreement: Vec<AgreementMsg>,
    deal_journal: Option<DealJournal>,
    journal_error: Option<String>,
}

impl Ceremony {
    fn attach_deal_journal(&mut self, mut journal: DealJournal) -> Result<(), DkgError> {
        for (dealer, (commitment, dealing)) in &self.accepted_deals {
            if let Some(old) = journal.accepted.get(dealer) {
                if old != &(commitment.clone(), dealing.clone()) {
                    return Err(DkgError::Setup("persisted self dealing differs from regenerated dealing".into()));
                }
            } else {
                journal.append(DealRecord::Accepted { dealer: dealer.encode().to_vec(), commitment: commitment.clone(), dealing: dealing.clone() }).map_err(DkgError::Setup)?;
                journal.accepted.insert(dealer.clone(), (commitment.clone(), dealing.clone()));
            }
        }
        if self.is_player() {
            let mut player = Player::new(self.info.clone(), self.key.clone()).map_err(|e| DkgError::Setup(format!("restore DKG player: {e:?}")))?;
            let mut acks_sent = BTreeMap::new();
            for (dealer, (commitment, dealing)) in &journal.accepted {
                let pub_msg = DealerPubMsg::<MinSig>::decode_cfg(commitment.as_slice(), &self.n).map_err(|e| DkgError::Setup(format!("persisted commitment: {e:?}")))?;
                let priv_msg = DealerPrivMsg::decode(dealing.as_slice()).map_err(|e| DkgError::Setup(format!("persisted dealing: {e:?}")))?;
                let ack = player.dealer_message::<N3f1>(dealer.clone(), pub_msg, priv_msg).map_err(|e| DkgError::Setup(format!("persisted dealing invalid: {e:?}")))?;
                if let Some(ack) = ack {
                    if dealer == &self.me { self.self_ack(ack); }
                    else { acks_sent.insert(dealer.clone(), Msg::Ack(ack.encode().to_vec())); }
                }
            }
            self.player = Some(player);
            self.acks_sent = acks_sent;
        }
        self.accepted_deals = journal.accepted.clone();
        if let Some(log) = journal.own_log.clone() {
            let owner = self.me.encode().to_vec();
            let before = self.logs.len();
            self.on_log(owner, log);
            if self.logs.len() == before { return Err(DkgError::Setup("persisted own log invalid".into())); }
            self.dealer = None;
        }
        if let Some(entries) = journal.decided_transcript.clone() {
            self.record_transcript(entries).ok_or_else(|| DkgError::Setup("persisted certified transcript invalid".into()))?;
        }
        self.deal_journal = Some(journal);
        Ok(())
    }

    fn persist_accepted_deal(&mut self, dealer: &PublicKey, commitment: &[u8], dealing: &[u8]) -> bool {
        let Some(journal) = self.deal_journal.as_mut() else { return true; };
        if let Some(old) = journal.accepted.get(dealer) {
            if old.0 == commitment && old.1 == dealing { return true; }
            self.journal_error = Some("dealer changed an acknowledged private dealing".into());
            return false;
        }
        let record = DealRecord::Accepted { dealer: dealer.encode().to_vec(), commitment: commitment.to_vec(), dealing: dealing.to_vec() };
        if let Err(error) = journal.append(record) {
            self.journal_error = Some(error);
            return false;
        }
        journal.accepted.insert(dealer.clone(), (commitment.to_vec(), dealing.to_vec()));
        true
    }

    /// Start `round` in whatever roles `key` has in it (dealer, player or both).
    /// `share` is our current share when we deal in a reshare.
    pub fn start(rng: impl CryptoRng, key: ed25519::PrivateKey, round: Round, share: Option<Share>) -> Result<(Self, Vec<(To, Msg)>), DkgError> {
        let setup = |e: &dyn std::fmt::Debug| DkgError::Setup(format!("{e:?}"));
        let expected = round.identity();
        let namespace = round.namespace();
        let context = round.context_digest();
        let info =
            Info::new::<N3f1>(&namespace, round.round, round.previous, Mode::NonZeroCounter, Reveal::V1, round.dealers.clone(), round.players.clone())
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
        let replay_key = key.clone();
        let dealt = if is_dealer { Some(Dealer::start::<N3f1>(rng, info.clone(), key, share).map_err(|e| setup(&e))?) } else { None };
        let agreement = round.strict_agreement.then(|| Agreement::new_with_context(replay_key.clone(), round.players.clone(), round.round, context));
        let mut c = Ceremony {
            info,
            me: me.clone(),
            n,
            dealers: round.dealers,
            players: round.players,
            expected,
            strict_agreement: round.strict_agreement,
            round: round.round,
            dealer: None,
            player,
            key: replay_key,
            deals: BTreeMap::new(),
            acked: BTreeSet::new(),
            acks_sent: BTreeMap::new(),
            accepted_deals: BTreeMap::new(),
            logs: BTreeMap::new(),
            equivocators: BTreeSet::new(),
            equivocation_logs: BTreeMap::new(),
            log_revision: 0,
            identity: None,
            output_digest: None,
            announced: BTreeMap::new(),
            transcripts: BTreeMap::new(),
            transcript_counts: BTreeMap::new(),
            decided: None,
            agreement,
            pending_agreement: Vec::new(),
            deal_journal: None,
            journal_error: None,
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
        let bytes = signed.encode().to_vec();
        if let Some(journal) = self.deal_journal.as_mut() {
            if let Err(error) = journal.append(DealRecord::OwnLog { log: bytes.clone() }) {
                self.journal_error = Some(error);
                return vec![];
            }
            journal.own_log = Some(bytes.clone());
        }
        let msg = Msg::Log { dealer: self.me.encode().to_vec(), log: bytes };
        let mut out = self.on_message(&self.me.clone(), msg.clone());
        out.push((To::All, msg));
        out
    }

    /// Everything we know, for periodic re-broadcast (logs and our announcement).
    pub fn rebroadcast(&self) -> Vec<(To, Msg)> {
        let mut out: Vec<(To, Msg)> = self.logs.iter().map(|(d, log)| (To::All, Msg::Log { dealer: d.encode().to_vec(), log: log.clone() })).collect();
        if let Some(done) = self.announcement() {
            out.push((To::All, Msg::Done(done)));
        }
        if self.strict_agreement {
            out.extend(self.equivocation_logs.iter().map(|(d, log)| (To::All, Msg::Log { dealer: d.encode().to_vec(), log: log.clone() })));
            // A later view can name a prior certified transcript. Keep the
            // bundles needed by the agreement lock and current proposal
            // available without rebroadcasting every adversarial subset.
            if let Some(agreement) = &self.agreement {
                for digest in agreement.needed_digests() {
                    if let Some(logs) = self.transcripts.get(digest.as_slice()) {
                        out.push((To::All, Msg::Transcript(logs.iter().map(|(d, l)| (d.encode().to_vec(), l.clone())).collect())));
                    }
                }
            }
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

    pub fn log_revision(&self) -> u64 {
        self.log_revision
    }

    /// Handle one message from authenticated peer `from`.
    pub fn on_message(&mut self, from: &PublicKey, msg: Msg) -> Vec<(To, Msg)> {
        match msg {
            Msg::Deal { commitment, dealing } => {
                let Some(player) = self.player.as_mut() else { return vec![] };
                let commitment_bytes = commitment.clone();
                let dealing_bytes = dealing.clone();
                let (Ok(commitment), Ok(dealing)) =
                    (DealerPubMsg::<MinSig>::decode_cfg(commitment.as_slice(), &self.n), DealerPrivMsg::decode(dealing.as_slice()))
                else {
                    return vec![];
                };
                match player.dealer_message::<N3f1>(from.clone(), commitment, dealing) {
                    Ok(Some(ack)) if *from == self.me => {
                        if !self.persist_accepted_deal(from, &commitment_bytes, &dealing_bytes) { return vec![]; }
                        self.accepted_deals.insert(from.clone(), (commitment_bytes, dealing_bytes));
                        self.self_ack(ack);
                        vec![]
                    }
                    Ok(Some(ack)) => {
                        if !self.persist_accepted_deal(from, &commitment_bytes, &dealing_bytes) { return vec![]; }
                        self.accepted_deals.insert(from.clone(), (commitment_bytes, dealing_bytes));
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
                if self.players.position(from).is_some() && (!self.strict_agreement || id.len() == 32) {
                    self.announced.insert(from.clone(), id);
                }
                vec![]
            }
            Msg::Transcript(entries) => {
                if !self.strict_agreement || (self.players.position(from).is_none() && self.dealers.position(from).is_none()) { return vec![]; }
                // A Byzantine sender can assemble many valid quorum subsets.
                // Bound retained bundles per authenticated sender while leaving
                // room for an honest peer's log set to converge over a round.
                let limit = self.dealers.len().saturating_mul(2).max(4);
                if self.transcript_counts.get(from).copied().unwrap_or(0) >= limit { return vec![]; }
                let before = self.transcripts.len();
                self.record_transcript(entries);
                if self.transcripts.len() > before { *self.transcript_counts.entry(from.clone()).or_default() += 1; }
                vec![]
            }
            Msg::Agreement(msg) => {
                if self.players.position(from).is_none()
                    && (self.dealers.position(from).is_none() || !matches!(&msg, AgreementMsg::Decision(_))) {
                    return vec![];
                }
                self.agreement.as_mut().map(|a| a.on_message(from, msg).into_iter().map(|m| (To::All, Msg::Agreement(m))).collect()).unwrap_or_default()
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
                self.log_revision = self.log_revision.saturating_add(1);
                vec![(To::All, Msg::Log { dealer, log })] // relay once
            }
            Some(existing) if *existing != log => {
                // Two different signed logs: provable equivocation. Relay the
                // second version so every honest node excludes this dealer too.
                if self.equivocators.insert(dealer_pk.clone()) {
                    self.equivocation_logs.insert(dealer_pk, log.clone());
                    self.log_revision = self.log_revision.saturating_add(1);
                    vec![(To::All, Msg::Log { dealer, log })]
                } else {
                    vec![]
                }
            }
            Some(_) => vec![],
        }
    }

    fn checked_logs(&self, entries: impl IntoIterator<Item = (PublicKey, Vec<u8>)>) -> Option<(BTreeMap<PublicKey, Vec<u8>>, DkgOutput)> {
        let mut raw = BTreeMap::new();
        let mut logs = Logs::<MinSig, PublicKey, N3f1>::new(self.info.clone());
        for (dealer, bytes) in entries {
            if self.dealers.position(&dealer).is_none() || raw.contains_key(&dealer) { return None; }
            let signed = SignedDealerLog::<MinSig, ed25519::PrivateKey>::decode_cfg(bytes.as_slice(), &self.n).ok()?;
            let (signer, log) = signed.check(&self.info)?;
            if signer != dealer { return None; }
            logs.record(dealer.clone(), log);
            raw.insert(dealer, bytes);
        }
        if raw.len() < self.dealers.quorum::<N3f1>() as usize { return None; }
        let output = observe::<MinSig, PublicKey, N3f1, ed25519::Batch>(&mut commonware_utils::sys_rng(), logs, &commonware_parallel::Sequential).ok()?;
        if self.expected.is_some_and(|id| id != *output.public().public()) { return None; }
        // Commonware can validly select logs that reveal a player's complete
        // threshold share. Such a bundle must never become vote-eligible.
        if self.strict_agreement && output.revealed().iter().any(|player| self.players.position(player).is_some()) { return None; }
        Some((raw, output))
    }

    fn transcript_digest(&self, raw: &BTreeMap<PublicKey, Vec<u8>>, output: &DkgOutput) -> Vec<u8> {
        let mut hash = blake3::Hasher::new_derive_key("aether DKG signed transcript agreement v2");
        hash.update(&self.round.to_be_bytes());
        hash.update(&output.encode());
        for (dealer, signed_log) in raw {
            hash.update(&dealer.encode());
            hash.update(&(signed_log.len() as u64).to_be_bytes());
            hash.update(signed_log);
        }
        hash.finalize().as_bytes().to_vec()
    }

    fn record_transcript(&mut self, entries: Vec<(Vec<u8>, Vec<u8>)>) -> Option<Vec<u8>> {
        if entries.len() > self.dealers.len() { return None; }
        let decoded = entries.into_iter().map(|(d, l)| PublicKey::decode(d.as_slice()).ok().map(|d| (d, l))).collect::<Option<Vec<_>>>()?;
        let (raw, output) = self.checked_logs(decoded)?;
        let digest = self.transcript_digest(&raw, &output);
        self.transcripts.entry(digest.clone()).or_insert(raw);
        Some(digest)
    }

    /// Publish a candidate consisting of the exact signed logs currently held.
    /// Public output validity is checked before this proposal can be voted on.
    pub fn propose_transcript(&mut self) -> Option<(Vec<u8>, Msg)> {
        if !self.strict_agreement || !self.is_player() { return None; }
        let entries: Vec<_> = self.logs.iter().filter(|(d, _)| !self.equivocators.contains(*d)).map(|(d, l)| (d.encode().to_vec(), l.clone())).collect();
        let digest = self.record_transcript(entries.clone())?;
        if let (Some(agreement), Ok(bytes)) = (self.agreement.as_mut(), <[u8; 32]>::try_from(digest.as_slice())) {
            self.pending_agreement.extend(agreement.propose_available(bytes));
        }
        Some((digest, Msg::Transcript(entries)))
    }

    /// Advance the certified agreement protocol after signed bundles are available.
    /// Call at a fixed 500 ms cadence in new-genesis ceremonies.
    pub fn tick_agreement(&mut self) -> Vec<(To, Msg)> {
        let Some(agreement) = self.agreement.as_mut() else { return vec![] };
        let available = self.transcripts.keys().filter_map(|d| <[u8; 32]>::try_from(d.as_slice()).ok()).collect();
        let mut out = std::mem::take(&mut self.pending_agreement);
        out.extend(agreement.tick(&available));
        out.into_iter().map(|m| (To::All, Msg::Agreement(m))).collect()
    }

    pub fn certified_transcript(&self) -> Option<Vec<u8>> {
        let digest = self.agreement.as_ref()?.decided()?.to_vec();
        self.transcripts.contains_key(&digest).then_some(digest)
    }

    /// Persist the exact certified bundle even for an old-only reshare dealer,
    /// which has no player share to finalize. A later relay process must be
    /// able to reconstruct it after this ceremony process returns.
    fn persist_certified_transcript(&mut self, digest: &[u8]) -> Result<(), DkgError> {
        if self.certified_transcript().as_deref() != Some(digest) {
            return Err(DkgError::Finalize("no quorum certificate for transcript".into()));
        }
        let raw = self.transcripts.get(digest).ok_or_else(|| DkgError::Finalize("decided transcript unavailable".into()))?;
        if let Some(journal) = self.deal_journal.as_mut() {
            let entries: Vec<_> = raw.iter().map(|(dealer, log)| (dealer.encode().to_vec(), log.clone())).collect();
            if let Some(old) = &journal.decided_transcript {
                if old != &entries { return Err(DkgError::Finalize("persisted transcript conflicts with new decision".into())); }
            } else {
                journal.append(DealRecord::DecidedTranscript { entries: entries.clone() }).map_err(DkgError::Finalize)?;
                journal.decided_transcript = Some(entries);
            }
        }
        Ok(())
    }

    /// Rebuild from retained private dealings and finalize only the certified log bundle.
    /// This also repairs a player that computed an earlier, incompatible candidate.
    pub fn finish_decided(&mut self, rng: &mut impl CryptoRng, digest: &[u8]) -> Result<(DkgOutput, Share), DkgError> {
        if self.certified_transcript().as_deref() != Some(digest) {
            return Err(DkgError::Finalize("no quorum certificate for transcript".into()));
        }
        let raw = self.transcripts.get(digest).ok_or_else(|| DkgError::Finalize("decided transcript unavailable".into()))?;
        // Revalidate before deriving even a local share from the certified
        // logs. A journal or future agreement change must not turn a revealed
        // seated share into a finalized committee output.
        if self.checked_logs(raw.iter().map(|(dealer, log)| (dealer.clone(), log.clone()))).is_none() {
            return Err(DkgError::Finalize("decided transcript has an unsafe DKG output".into()));
        }
        let mut player = Player::new(self.info.clone(), self.key.clone()).map_err(|e| DkgError::Finalize(format!("rebuild player: {e:?}")))?;
        for (dealer, (commitment, dealing)) in &self.accepted_deals {
            let pub_msg = DealerPubMsg::<MinSig>::decode_cfg(commitment.as_slice(), &self.n).map_err(|e| DkgError::Finalize(format!("stored commitment: {e:?}")))?;
            let priv_msg = DealerPrivMsg::decode(dealing.as_slice()).map_err(|e| DkgError::Finalize(format!("stored dealing: {e:?}")))?;
            player.dealer_message::<N3f1>(dealer.clone(), pub_msg, priv_msg).map_err(|e| DkgError::Finalize(format!("replay dealing: {e:?}")))?;
        }
        let mut logs = Logs::<MinSig, PublicKey, N3f1>::new(self.info.clone());
        for (dealer, bytes) in raw {
            let signed = SignedDealerLog::<MinSig, ed25519::PrivateKey>::decode_cfg(bytes.as_slice(), &self.n).map_err(|e| DkgError::Finalize(format!("stored signed log: {e:?}")))?;
            let (signer, log) = signed.check(&self.info).ok_or_else(|| DkgError::Finalize("stored log lost validity".into()))?;
            if &signer != dealer { return Err(DkgError::Finalize("stored log signer changed".into())); }
            logs.record(signer, log);
        }
        let (output, share) = player.finalize::<N3f1, ed25519::Batch>(rng, logs, &commonware_parallel::Sequential).map_err(|e| DkgError::Finalize(format!("decided output: {e:?}")))?;
        if output.revealed().iter().any(|player| self.players.position(player).is_some()) {
            return Err(DkgError::Finalize("decided output reveals a seated player's threshold share; retry with a new round".into()));
        }
        if self.transcript_digest(raw, &output) != digest { return Err(DkgError::Finalize("decided output digest mismatch".into())); }
        self.persist_certified_transcript(digest)?;
        self.identity = Some(*output.public().public());
        self.output_digest = Some(digest.to_vec());
        self.announced.insert(self.me.clone(), digest.to_vec());
        self.decided = Some(digest.to_vec());
        Ok((output, share))
    }

    /// Legacy 7780 finalization. New-genesis callers must first obtain an
    /// Agreement decision and use `finish_decided` on its exact signed logs.
    pub fn finish(&mut self, rng: &mut impl CryptoRng) -> Result<(DkgOutput, Share), DkgError> {
        if self.strict_agreement {
            return Err(DkgError::Finalize("new-genesis finalization requires a certified transcript".into()));
        }
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
        if self.strict_agreement {
            let mut hash = blake3::Hasher::new_derive_key("aether DKG output agreement v1");
            hash.update(&self.round.to_be_bytes());
            hash.update(&output.encode());
            // Explicitly bind the exact accepted dealers, even if an upstream
            // Output codec changes what it includes in a future version.
            for dealer in output.dealers().iter() {
                hash.update(&dealer.encode());
            }
            self.output_digest = Some(hash.finalize().as_bytes().to_vec());
        }
        self.announced.insert(self.me.clone(), self.announcement().expect("finished player announces"));
        Ok((output, share))
    }

    fn announcement(&self) -> Option<Vec<u8>> {
        if self.strict_agreement { self.output_digest.clone() } else { self.identity.map(|id| id.encode().to_vec()) }
    }

    /// New-genesis agreement is true only after this player finalized the
    /// certified transcript. Legacy 7780 retains its original Done semantics.
    pub fn agreement(&self, late: bool) -> Option<bool> {
        if self.strict_agreement {
            return self.decided.as_ref().and_then(|digest| (self.certified_transcript().as_ref() == Some(digest)).then_some(true));
        }
        let mine = match self.announcement() {
            Some(done) => done,
            None if !self.is_player() => self.expected?.encode().to_vec(),
            None => return None,
        };
        if self.announced.values().any(|done| *done != mine) {
            return Some(false);
        }
        let quorum = self.players.quorum::<N3f1>() as usize;
        (self.announced.len() == self.players.len() || (late && self.announced.len() >= quorum)).then_some(true)
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
    /// True when the output's revealed players include a seated player: the
    /// A4-1 check, one implementation shared by every gate that refuses such
    /// an output (the ceremony itself, startup, the mainnet final-file gate).
    pub fn reveals_seated_share(output: &DkgOutput, seated: &Set<PublicKey>) -> bool {
        output.revealed().iter().any(|player| seated.position(player).is_some())
    }

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

impl Timeouts {
    /// Last tick at which the child can wait for a certified decision.
    fn strict_deadline(&self, players: usize) -> std::time::Duration {
        let longest_view = TICK_INTERVAL.saturating_mul((BASE_VIEW_TICKS << MAX_VIEW_BACKOFF) as u32);
        self.total.max(self.dealing.saturating_add(longest_view.saturating_mul(players.min(u32::MAX as usize) as u32)).saturating_add(CERTIFIED_TRANSCRIPT_RELAY))
    }

    /// Allow a decision at the deadline to receive its grace before returning.
    /// The supervisor adds a separate process return margin to this bound.
    pub fn strict_return_bound(&self, players: usize) -> std::time::Duration {
        // One tick may notice a decision that arrived just before the
        // deadline; another may notice that its grace has elapsed.
        self.strict_deadline(players).saturating_add(STRICT_DECISION_GRACE).saturating_add(TICK_INTERVAL.saturating_mul(2))
    }
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

/// Run a legacy ceremony over a p2p channel. New-genesis ceremonies require
/// [`run_with_journal`] so a restart cannot sign a conflicting agreement vote.
pub async fn run<S, R>(
    key: ed25519::PrivateKey,
    round: Round,
    share: Option<Share>,
    sender: S,
    receiver: R,
    timeouts: Timeouts,
) -> Result<Option<(DkgOutput, Share)>, DkgError>
where
    S: commonware_p2p::Sender<PublicKey = PublicKey>,
    R: commonware_p2p::Receiver<PublicKey = PublicKey>,
{
    if round.strict_agreement {
        return Err(DkgError::Setup("new-genesis DKG requires a durable agreement journal".into()));
    }
    run_inner(key, round, share, sender, receiver, timeouts, None).await
}

/// Run a new-genesis ceremony with a durable per-validator, per-round vote
/// journal. The path must be stable across retries and restarts of this round.
pub async fn run_with_journal<S, R>(
    key: ed25519::PrivateKey,
    round: Round,
    share: Option<Share>,
    sender: S,
    receiver: R,
    timeouts: Timeouts,
    journal: std::path::PathBuf,
) -> Result<Option<(DkgOutput, Share)>, DkgError>
where
    S: commonware_p2p::Sender<PublicKey = PublicKey>,
    R: commonware_p2p::Receiver<PublicKey = PublicKey>,
{
    run_inner(key, round, share, sender, receiver, timeouts, Some(journal)).await
}

/// Resume only the certified transcript relay after the ceremony child has
/// returned and its caller has staged any usable share. A departing dealer
/// also serves the decision. This deliberately reopens the same round journals;
/// a new attempt after a disclosed share must use a new round and journals.
pub async fn run_relay_with_journal<S, R>(
    key: ed25519::PrivateKey,
    round: Round,
    share: Option<Share>,
    mut sender: S,
    mut receiver: R,
    journal: PathBuf,
    duration: std::time::Duration,
) -> Result<(), DkgError>
where
    S: commonware_p2p::Sender<PublicKey = PublicKey>,
    R: commonware_p2p::Receiver<PublicKey = PublicKey>,
{
    if !round.strict_agreement {
        return Err(DkgError::Setup("post-stage relay requires a new-genesis ceremony".into()));
    }
    if round.dealers.position(&key.public_key()).is_none() && round.players.position(&key.public_key()).is_none() {
        return Err(DkgError::Setup("post-stage relay requires a ceremony participant".into()));
    }
    let deal_path = journal.with_extension("deals");
    if !journal.exists() || !deal_path.exists() {
        return Err(DkgError::Setup("post-stage relay requires existing ceremony journals".into()));
    }
    let deals = DealJournal::open(deal_path, DealJournal::binding(&key, &round)).map_err(DkgError::Setup)?;
    if deals.decided_transcript.is_none() {
        return Err(DkgError::Setup("post-stage relay has no persisted certified transcript".into()));
    }
    let (mut c, _) = Ceremony::start(rand::rngs::StdRng::from_seed(deals.seed), key, round, share)?;
    c.attach_deal_journal(deals)?;
    c.agreement.as_mut().expect("strict ceremony has agreement").attach_journal(journal).map_err(DkgError::Setup)?;
    // Restore Agreement's available set from the verified bundle before
    // checking the persisted decision or broadcasting any message.
    let out = c.tick_agreement();
    if c.certified_transcript().is_none() {
        return Err(DkgError::Setup("post-stage relay has no certified decision for its transcript".into()));
    }
    send_all(&mut sender, out);
    send_all(&mut sender, c.rebroadcast());
    let end = tokio::time::Instant::now() + duration;
    let mut tick = tokio::time::interval(TICK_INTERVAL);
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(end) => return Ok(()),
            r = receiver.recv() => {
                let Ok((from, msg)) = r else { return Err(DkgError::Finalize("p2p closed during post-stage relay".into())) };
                if let Ok(m) = serde_json::from_slice::<Msg>(msg.as_ref()) {
                    let out = c.on_message(&from, m);
                    if let Some(error) = c.journal_error.take() { return Err(DkgError::Finalize(error)); }
                    send_all(&mut sender, out);
                }
            }
            _ = tick.tick() => {
                let mut out = c.tick_agreement();
                out.extend(c.rebroadcast());
                send_all(&mut sender, out);
            }
        }
    }
}

/// New-genesis players return only after finalizing a certified signed-log
/// bundle. An old-only resharing dealer returns `None` after relaying the
/// decision.
async fn run_inner<S, R>(
    key: ed25519::PrivateKey,
    round: Round,
    share: Option<Share>,
    mut sender: S,
    mut receiver: R,
    timeouts: Timeouts,
    journal: Option<std::path::PathBuf>,
) -> Result<Option<(DkgOutput, Share)>, DkgError>
where
    S: commonware_p2p::Sender<PublicKey = PublicKey>,
    R: commonware_p2p::Receiver<PublicKey = PublicKey>,
{
    use std::time::Instant;
    let mut rng = commonware_utils::sys_rng();
    let deal_journal = if round.strict_agreement {
        let base = journal.as_ref().ok_or_else(|| DkgError::Setup("new-genesis DKG requires a durable agreement journal".into()))?;
        Some(DealJournal::open(base.with_extension("deals"), DealJournal::binding(&key, &round)).map_err(DkgError::Setup)?)
    } else { None };
    let (mut c, out) = if let Some(deals) = deal_journal.as_ref() {
        Ceremony::start(rand::rngs::StdRng::from_seed(deals.seed), key, round, share)?
    } else {
        Ceremony::start(commonware_utils::sys_rng(), key, round, share)?
    };
    if c.strict_agreement {
        c.attach_deal_journal(deal_journal.expect("strict ceremony has deal journal"))?;
        let path = journal.ok_or_else(|| DkgError::Setup("new-genesis DKG requires a durable agreement journal".into()))?;
        c.agreement.as_mut().expect("strict ceremony has agreement").attach_journal(path).map_err(DkgError::Setup)?;
    }
    send_all(&mut sender, out);
    let start = Instant::now();
    let mut result = None;
    let mut logs_stable_since = Instant::now();
    let mut log_revision = 0;
    let mut agreed_at: Option<Instant> = None;
    let mut proposal: Option<Vec<u8>> = None;
    // Give a strict ceremony enough time to rotate past offline proposers.
    let strict_deadline = timeouts.strict_deadline(c.players.len());
    let mut tick = tokio::time::interval(TICK_INTERVAL);
    loop {
        tokio::select! {
            r = receiver.recv() => {
                let Ok((from, msg)) = r else { return Err(DkgError::Finalize("p2p closed".into())) };
                if let Ok(m) = serde_json::from_slice::<Msg>(msg.as_ref()) {
                    let out = c.on_message(&from, m);
                    if let Some(error) = c.journal_error.take() { return Err(DkgError::Finalize(error)); }
                    send_all(&mut sender, out);
                }
            }
            _ = tick.tick() => {
                let elapsed = start.elapsed();
                let mut out = c.pending_deals();
                if c.all_acked() || elapsed > timeouts.dealing {
                    out.extend(c.close_dealing());
                }
                if let Some(error) = c.journal_error.take() { return Err(DkgError::Finalize(error)); }
                if c.log_revision() != log_revision {
                    log_revision = c.log_revision();
                    logs_stable_since = Instant::now();
                }
                // All logs in (or, after the dealing window, a quorum of them) and
                // no new version for a moment (equivocation window).
                let enough = c.have_all_logs() || (elapsed > timeouts.dealing + QUORUM_LOG_DELAY && c.have_quorum_logs());
                if c.strict_agreement {
                    // New genesis: agree on signed logs before consuming private
                    // player state. A candidate is never accepted from its hash
                    // alone; the entire bundle is verified by every receiver.
                    if c.is_player() && enough && logs_stable_since.elapsed() > LOG_SETTLE_TIME {
                        if let Some((digest, msg)) = c.propose_transcript() {
                            if proposal.as_ref() != Some(&digest) {
                                proposal = Some(digest);
                                out.push((To::All, msg));
                            }
                        }
                    }
                    out.extend(c.tick_agreement());
                    if let Some(digest) = c.certified_transcript() {
                        c.persist_certified_transcript(&digest)?;
                    }
                    if result.is_none() && c.is_player() {
                        if let Some(digest) = c.certified_transcript() {
                            let (o, s) = c.finish_decided(&mut rng, &digest)?;
                            tracing::info!(identity = %hex::encode(o.public().public().encode()), "dkg: finalized quorum-certified transcript");
                            result = Some((o, s));
                            agreed_at = Some(Instant::now());
                        }
                    } else if !c.is_player() && agreed_at.is_none() && c.certified_transcript().is_some() {
                        agreed_at = Some(Instant::now());
                    }
                } else if result.is_none() && c.is_player() && enough && logs_stable_since.elapsed() > LOG_SETTLE_TIME {
                    let (o, s) = c.finish(&mut rng)?;
                    tracing::info!(identity = %hex::encode(o.public().public().encode()), "dkg: computed share; waiting for agreement");
                    result = Some((o, s));
                }
                out.extend(c.rebroadcast());
                send_all(&mut sender, out);
                if c.strict_agreement {
                    // The exact certified bundle is persisted and the player's
                    // share has been finalized before agreed_at is set. Do not
                    // wait for every Done: a silent player would hold staging
                    // past the supervisor timeout. The caller reopens a bounded
                    // relay after staging for late players.
                    if strict_decision_grace_elapsed(agreed_at, Instant::now()) {
                        return Ok(result);
                    }
                } else {
                    match c.agreement(elapsed > timeouts.dealing + LEGACY_QUORUM_DONE_DELAY) {
                        Some(false) => return Err(DkgError::Disagreement),
                        Some(true) => match agreed_at {
                            None => agreed_at = Some(Instant::now()),
                            Some(t) if t.elapsed() > LEGACY_DONE_RELAY => return Ok(result),
                            _ => {}
                        },
                        None => {}
                    }
                }
                if agreed_at.is_none() && elapsed > if c.strict_agreement { strict_deadline } else { timeouts.total } {
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

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_utils::TryCollect;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn certified_return_needs_only_a_short_decision_grace() {
        let decided = std::time::Instant::now();
        assert!(!strict_decision_grace_elapsed(Some(decided), decided));
        assert!(!strict_decision_grace_elapsed(Some(decided), decided + STRICT_DECISION_GRACE));
        assert!(strict_decision_grace_elapsed(Some(decided), decided + STRICT_DECISION_GRACE + TICK_INTERVAL));
        assert!(!strict_decision_grace_elapsed(None, decided + STRICT_DECISION_GRACE + TICK_INTERVAL));
        assert!(STRICT_DECISION_GRACE < std::time::Duration::from_secs(120));
    }
    #[test]
    fn private_deals_and_own_log_survive_restart() {
        let keys: Vec<_> = (1..=4).map(aether_light::devnet_validator_key).collect();
        let pks: Vec<_> = keys.iter().map(|key| key.public_key()).collect();
        let participants: Set<PublicKey> = pks.iter().cloned().try_collect().unwrap();
        let round = Round::dkg(participants, 91);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
        let tmp = root.join("tmp");
        std::fs::create_dir_all(&tmp).unwrap();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = tmp.join(format!("dkg-deals-test-{}-{nonce}.deals", std::process::id()));
        let binding = DealJournal::binding(&keys[0], &round);

        let journal = DealJournal::open(path.clone(), binding).unwrap();
        let seed = journal.seed;
        let (mut first, _) = Ceremony::start(rand::rngs::StdRng::from_seed(seed), keys[0].clone(), round.clone(), None).unwrap();
        first.attach_deal_journal(journal).unwrap();
        let (_, outbound) = Ceremony::start(rand::rngs::StdRng::seed_from_u64(7), keys[1].clone(), round.clone(), None).unwrap();
        let external_deal = outbound.into_iter().find_map(|(to, msg)| match to {
            To::One(player) if player == pks[0] && matches!(msg, Msg::Deal { .. }) => Some(msg),
            _ => None,
        }).unwrap();
        let ack = first.on_message(&pks[1], external_deal.clone());
        assert!(ack.iter().any(|(_, msg)| matches!(msg, Msg::Ack(_))));
        assert!(first.journal_error.is_none());
        drop(first);

        let journal = DealJournal::open(path.clone(), binding).unwrap();
        assert_eq!(journal.seed, seed);
        assert!(journal.accepted.contains_key(&pks[1]), "Ack must follow a durable private Deal");
        let (mut restarted, _) = Ceremony::start(rand::rngs::StdRng::from_seed(journal.seed), keys[0].clone(), round.clone(), None).unwrap();
        restarted.attach_deal_journal(journal).unwrap();
        let repeated_ack = restarted.on_message(&pks[1], external_deal);
        assert!(repeated_ack.iter().any(|(_, msg)| matches!(msg, Msg::Ack(_))), "restart must replay the same Ack");
        assert!(restarted.accepted_deals.contains_key(&pks[1]));
        for (recipient, deal) in restarted.deals.clone() {
            let recipient_index = pks.iter().position(|pk| *pk == recipient).unwrap();
            let (mut player, _) = Ceremony::start(rand::rngs::StdRng::seed_from_u64(200 + recipient_index as u64), keys[recipient_index].clone(), round.clone(), None).unwrap();
            for (_, ack) in player.on_message(&pks[0], deal) { restarted.on_message(&recipient, ack); }
        }
        restarted.close_dealing();
        assert!(restarted.journal_error.is_none());
        drop(restarted);

        let journal = DealJournal::open(path.clone(), binding).unwrap();
        assert!(journal.own_log.is_some());
        let (mut final_restart, _) = Ceremony::start(rand::rngs::StdRng::from_seed(journal.seed), keys[0].clone(), round.clone(), None).unwrap();
        final_restart.attach_deal_journal(journal).unwrap();
        assert!(final_restart.dealer.is_none(), "a persisted signed log must not be signed again");
        assert!(final_restart.logs.contains_key(&pks[0]));
        for (index, seed) in [(1, 7), (2, 8)] {
            let (mut dealer, outbound) = Ceremony::start(rand::rngs::StdRng::seed_from_u64(seed), keys[index].clone(), round.clone(), None).unwrap();
            for (to, deal) in outbound {
                let To::One(recipient) = to else { continue };
                let recipient_index = pks.iter().position(|pk| *pk == recipient).unwrap();
                let replies = if recipient_index == 0 {
                    final_restart.on_message(&pks[index], deal)
                } else {
                    let (mut player, _) = Ceremony::start(rand::rngs::StdRng::seed_from_u64(100 + recipient_index as u64), keys[recipient_index].clone(), round.clone(), None).unwrap();
                    player.on_message(&pks[index], deal)
                };
                for (_, ack) in replies { dealer.on_message(&recipient, ack); }
            }
            let signed_log = dealer.close_dealing().into_iter().find_map(|(_, msg)| matches!(msg, Msg::Log { .. }).then_some(msg)).unwrap();
            final_restart.on_message(&pks[index], signed_log);
        }
        let (digest, _) = final_restart.propose_transcript().expect("three signed logs form a valid transcript");
        let entries: Vec<_> = final_restart.transcripts[&digest].iter().map(|(dealer, log)| (dealer.encode().to_vec(), log.clone())).collect();
        let journal = final_restart.deal_journal.as_mut().unwrap();
        journal.append(DealRecord::DecidedTranscript { entries: entries.clone() }).unwrap();
        journal.decided_transcript = Some(entries);
        drop(final_restart);

        let journal = DealJournal::open(path.clone(), binding).unwrap();
        let (mut after_decision, _) = Ceremony::start(rand::rngs::StdRng::from_seed(journal.seed), keys[0].clone(), round, None).unwrap();
        after_decision.attach_deal_journal(journal).unwrap();
        assert!(after_decision.transcripts.contains_key(&digest), "certified signed bundle must survive restart after Done");
        drop(after_decision);
        std::fs::remove_file(path).unwrap();
    }
}
