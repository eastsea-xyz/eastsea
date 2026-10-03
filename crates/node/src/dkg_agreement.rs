//! One-shot Byzantine agreement on a validated DKG transcript digest.
//!
//! This is a small, Tendermint-style two-phase protocol. The DKG layer owns
//! signed-log validation and makes only usable transcript digests available.
//! A rotating proposer supplies a quorum of signed view-change reports. Votes
//! and the lock certificate are transferable, so one equivocating or silent
//! participant cannot hold an honest quorum in a split first view.

use crate::block::PublicKey;
use commonware_codec::{DecodeExt, Encode};
use commonware_cryptography::{ed25519, Signer as _, Verifier as _};
use commonware_utils::{
    ordered::{Quorum as _, Set},
    N3f1,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::Read as _;
use std::io::Write as _;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;

const NAMESPACE: &[u8] = b"_AETHER_NEW_GENESIS_DKG_AGREEMENT_V1";
/// The caller ticks every 500 ms. Views lengthen after failures so bounded
/// network delays eventually fit within an honest proposer's view.
pub const BASE_VIEW_TICKS: u64 = 8;
pub const MAX_VIEW_BACKOFF: u32 = 4;
pub const REBROADCAST_TICKS: u64 = 4;
const MAX_FUTURE_VIEWS: u64 = 32;
const MAX_JOURNAL_RECORD: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Phase {
    Precommit,
    Commit,
}

impl Phase {
    fn tag(self) -> u8 {
        match self {
            Self::Precommit => 1,
            Self::Commit => 2,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignedVote {
    pub view: u64,
    pub phase: Phase,
    pub digest: [u8; 32],
    pub signer: Vec<u8>,
    pub signature: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Certificate {
    pub view: u64,
    pub phase: Phase,
    pub digest: [u8; 32],
    pub votes: Vec<SignedVote>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NewView {
    pub view: u64,
    /// Highest known precommit certificate, including the lock certificate.
    pub best: Option<Certificate>,
    pub signer: Vec<u8>,
    pub signature: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Proposal {
    pub view: u64,
    pub digest: [u8; 32],
    pub new_views: Vec<NewView>,
    pub signer: Vec<u8>,
    pub signature: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum AgreementMsg {
    NewView(NewView),
    Proposal(Proposal),
    Vote(SignedVote),
    PrecommitCertificate(Certificate),
    Decision(Certificate),
}

#[derive(Serialize, Deserialize)]
struct SafetySnapshot {
    version: u8,
    round: u64,
    roster_hash: [u8; 32],
    signer: Vec<u8>,
    view: u64,
    own_votes: Vec<SignedVote>,
    lock: Option<Certificate>,
    best: Option<Certificate>,
    decided: Option<Certificate>,
}

pub struct Agreement {
    key: ed25519::PrivateKey,
    me: PublicKey,
    players: Set<PublicKey>,
    round: u64,
    roster_hash: [u8; 32],
    quorum: usize,
    view: u64,
    view_ticks: u64,
    ticks: u64,
    preferred: Option<[u8; 32]>,
    available: BTreeSet<[u8; 32]>,
    new_views: BTreeMap<u64, BTreeMap<PublicKey, NewView>>,
    proposals: BTreeMap<u64, Proposal>,
    votes: BTreeMap<(u64, u8, [u8; 32]), BTreeMap<PublicKey, SignedVote>>,
    own_votes: BTreeSet<(u64, u8)>,
    lock: Option<Certificate>,
    best: Option<Certificate>,
    decided: Option<Certificate>,
    sent_proposals: BTreeSet<u64>,
    sent_certificates: BTreeSet<(u64, [u8; 32])>,
    journal: Option<File>,
    journal_hash: Option<[u8; 32]>,
}

impl Agreement {
    pub fn new(key: ed25519::PrivateKey, players: Set<PublicKey>, round: u64) -> Self {
        let me = key.public_key();
        let mut hasher = blake3::Hasher::new_derive_key("aether DKG agreement roster v1");
        for player in players.iter() {
            hasher.update(&player.encode());
        }
        let roster_hash = *hasher.finalize().as_bytes();
        let quorum = players.quorum::<N3f1>() as usize;
        Self {
            key,
            me,
            players,
            round,
            roster_hash,
            quorum,
            view: 0,
            view_ticks: 0,
            ticks: 0,
            preferred: None,
            available: BTreeSet::new(),
            new_views: BTreeMap::new(),
            proposals: BTreeMap::new(),
            votes: BTreeMap::new(),
            own_votes: BTreeSet::new(),
            lock: None,
            best: None,
            decided: None,
            sent_proposals: BTreeSet::new(),
            sent_certificates: BTreeSet::new(),
            journal: None,
            journal_hash: None,
        }
    }

    /// Bind votes to the full Commonware DKG configuration as well as the
    /// player roster. A reshare with different old dealers or old output must
    /// not accept a certificate from an earlier attempt using this round id.
    pub fn new_with_context(
        key: ed25519::PrivateKey,
        players: Set<PublicKey>,
        round: u64,
        context: [u8; 32],
    ) -> Self {
        let mut agreement = Self::new(key, players, round);
        let mut hash = blake3::Hasher::new_derive_key("aether DKG agreement full context v1");
        hash.update(&agreement.roster_hash);
        hash.update(&context);
        agreement.roster_hash = *hash.finalize().as_bytes();
        agreement
    }

    /// Bind this one-shot agreement to a durable, exclusively held journal.
    /// A restarted process reuses its old votes and lock rather than signing a
    /// second value in the same view. Call before any agreement message leaves.
    pub fn attach_journal(&mut self, path: PathBuf) -> Result<(), String> {
        let parent = path.parent().ok_or("agreement journal has no parent")?;
        std::fs::create_dir_all(parent).map_err(|e| format!("agreement journal directory: {e}"))?;
        let existed = path.exists();
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)
            .map_err(|e| format!("open agreement journal: {e}"))?;
        if file
            .metadata()
            .map_err(|e| format!("stat agreement journal: {e}"))?
            .permissions()
            .mode()
            & 0o077
            != 0
        {
            return Err("agreement journal permissions must be 0600".into());
        }
        // A second process with the same key and round must not vote alongside
        // this one, even if both see the same bytes on disk.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("agreement journal is already in use".into());
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|e| format!("read agreement journal: {e}"))?;
        let mut offset = 0;
        let mut latest: Option<(SafetySnapshot, [u8; 32])> = None;
        while bytes.len().saturating_sub(offset) >= 4 {
            let len =
                u32::from_be_bytes(bytes[offset..offset + 4].try_into().expect("length slice"))
                    as usize;
            if len > MAX_JOURNAL_RECORD || bytes.len().saturating_sub(offset + 4) < len + 32 {
                break;
            }
            let payload = &bytes[offset + 4..offset + 4 + len];
            let hash = *blake3::hash(payload).as_bytes();
            if bytes[offset + 4 + len..offset + 4 + len + 32] != hash {
                break;
            }
            let snapshot: SafetySnapshot = serde_json::from_slice(payload)
                .map_err(|e| format!("decode agreement journal: {e}"))?;
            latest = Some((snapshot, hash));
            offset += 4 + len + 32;
        }
        if !bytes.is_empty() && latest.is_none() {
            return Err("agreement journal has no valid safety record".into());
        }
        if offset < bytes.len() {
            file.set_len(offset as u64)
                .map_err(|e| format!("truncate torn agreement record: {e}"))?;
            file.sync_all()
                .map_err(|e| format!("sync repaired agreement journal: {e}"))?;
        }
        if let Some((snapshot, hash)) = latest {
            self.restore(snapshot)?;
            self.journal_hash = Some(hash);
        }
        self.journal = Some(file);
        self.persist()?;
        if !existed {
            File::open(parent)
                .and_then(|dir| dir.sync_all())
                .map_err(|e| format!("sync agreement journal directory: {e}"))?;
        }
        Ok(())
    }

    fn restore(&mut self, snapshot: SafetySnapshot) -> Result<(), String> {
        if snapshot.version != 1
            || snapshot.round != self.round
            || snapshot.roster_hash != self.roster_hash
            || snapshot.signer != self.me.encode().to_vec()
        {
            return Err("agreement journal belongs to another key, roster, or round".into());
        }
        for qc in [&snapshot.lock, &snapshot.best].into_iter().flatten() {
            if !self.verify_certificate(qc, Phase::Precommit) || qc.view > snapshot.view {
                return Err("agreement journal has an invalid precommit certificate".into());
            }
        }
        if snapshot.decided.as_ref().is_some_and(|qc| {
            !self.verify_certificate(qc, Phase::Commit) || qc.view > snapshot.view
        }) {
            return Err("agreement journal has an invalid decision certificate".into());
        }
        if let (Some(lock), Some(decision)) = (&snapshot.lock, &snapshot.decided) {
            if lock.digest != decision.digest {
                return Err("agreement journal lock conflicts with decision".into());
            }
        }
        self.view = snapshot.view;
        for vote in snapshot.own_votes {
            if vote.signer != self.me.encode().to_vec()
                || vote.view > self.view
                || self.verify_vote(&vote).is_none()
                || !self.own_votes.insert((vote.view, vote.phase.tag()))
                || !self.store_vote(vote)
            {
                return Err("agreement journal contains an invalid or duplicate own vote".into());
            }
        }
        self.lock = snapshot.lock;
        self.best = snapshot.best;
        self.decided = snapshot.decided;
        Ok(())
    }

    fn snapshot(&self) -> SafetySnapshot {
        let own_votes = self
            .votes
            .values()
            .filter_map(|votes| votes.get(&self.me).cloned())
            .collect();
        SafetySnapshot {
            version: 1,
            round: self.round,
            roster_hash: self.roster_hash,
            signer: self.me.encode().to_vec(),
            view: self.view,
            own_votes,
            lock: self.lock.clone(),
            best: self.best.clone(),
            decided: self.decided.clone(),
        }
    }

    fn persist(&mut self) -> Result<(), String> {
        if self.journal.is_none() {
            return Ok(());
        }
        let payload = serde_json::to_vec(&self.snapshot())
            .map_err(|e| format!("encode agreement journal: {e}"))?;
        if payload.len() > MAX_JOURNAL_RECORD {
            return Err("agreement journal record too large".into());
        }
        let hash = *blake3::hash(&payload).as_bytes();
        if self.journal_hash == Some(hash) {
            return Ok(());
        }
        let file = self.journal.as_mut().expect("journal was checked above");
        file.write_all(&(payload.len() as u32).to_be_bytes())
            .and_then(|_| file.write_all(&payload))
            .and_then(|_| file.write_all(&hash))
            .and_then(|_| file.sync_all())
            .map_err(|e| format!("sync agreement journal before broadcast: {e}"))?;
        self.journal_hash = Some(hash);
        Ok(())
    }

    fn persist_or_stop(&mut self) {
        self.persist()
            .expect("DKG agreement journal must be durable before broadcasting a vote");
    }

    pub fn view(&self) -> u64 {
        self.view
    }
    /// Signed-log bundles worth retaining and regossiping for view change and
    /// late recovery. The caller owns the bundles and validates their bytes.
    pub fn needed_digests(&self) -> BTreeSet<[u8; 32]> {
        let mut needed = BTreeSet::new();
        if let Some(digest) = self.preferred {
            needed.insert(digest);
        }
        if let Some(qc) = &self.lock {
            needed.insert(qc.digest);
        }
        if let Some(qc) = &self.best {
            needed.insert(qc.digest);
        }
        if let Some(qc) = &self.decided {
            needed.insert(qc.digest);
        }
        if let Some(proposal) = self.proposals.get(&self.view) {
            needed.insert(proposal.digest);
        }
        needed
    }
    pub fn decided(&self) -> Option<[u8; 32]> {
        self.decided
            .as_ref()
            .filter(|c| self.available.contains(&c.digest))
            .map(|c| c.digest)
    }

    fn signer(&self, bytes: &[u8]) -> Option<PublicKey> {
        let pk = PublicKey::decode(bytes).ok()?;
        self.players.position(&pk).map(|_| pk)
    }

    fn envelope(&self, tag: u8, view: u64, digest: &[u8; 32], extra: &[u8]) -> Vec<u8> {
        let mut msg = Vec::with_capacity(8 + 32 + 1 + 8 + 32 + extra.len());
        msg.extend_from_slice(&self.round.to_be_bytes());
        msg.extend_from_slice(&self.roster_hash);
        msg.push(tag);
        msg.extend_from_slice(&view.to_be_bytes());
        msg.extend_from_slice(digest);
        msg.extend_from_slice(extra);
        msg
    }

    fn verify_signature(
        &self,
        signer: &[u8],
        signature: &[u8],
        message: &[u8],
    ) -> Option<PublicKey> {
        let pk = self.signer(signer)?;
        let signature = ed25519::Signature::decode(signature).ok()?;
        pk.verify(NAMESPACE, message, &signature).then_some(pk)
    }

    fn sign_vote(&mut self, phase: Phase, digest: [u8; 32]) -> Option<SignedVote> {
        let slot = (self.view, phase.tag());
        if !self.own_votes.insert(slot) {
            return None;
        }
        let message = self.envelope(phase.tag(), self.view, &digest, &[]);
        let vote = SignedVote {
            view: self.view,
            phase,
            digest,
            signer: self.me.encode().to_vec(),
            signature: self.key.sign(NAMESPACE, &message).encode().to_vec(),
        };
        self.store_vote(vote.clone());
        Some(vote)
    }

    fn verify_vote(&self, vote: &SignedVote) -> Option<PublicKey> {
        let message = self.envelope(vote.phase.tag(), vote.view, &vote.digest, &[]);
        self.verify_signature(&vote.signer, &vote.signature, &message)
    }

    fn store_vote(&mut self, vote: SignedVote) -> bool {
        let Some(pk) = self.verify_vote(&vote) else {
            return false;
        };
        if vote.view > self.view.saturating_add(MAX_FUTURE_VIEWS) {
            return false;
        }
        // A sender's first authenticated vote for each phase/view is counted.
        // Equivocation is not permitted to inflate either certificate.
        if self.votes.iter().any(|((v, p, _), by_sender)| {
            *v == vote.view && *p == vote.phase.tag() && by_sender.contains_key(&pk)
        }) {
            return false;
        }
        self.votes
            .entry((vote.view, vote.phase.tag(), vote.digest))
            .or_default()
            .insert(pk, vote);
        true
    }

    fn vote_certificate(&self, view: u64, phase: Phase, digest: [u8; 32]) -> Option<Certificate> {
        let votes = self.votes.get(&(view, phase.tag(), digest))?;
        if votes.len() < self.quorum {
            return None;
        }
        Some(Certificate {
            view,
            phase,
            digest,
            votes: votes.values().take(self.quorum).cloned().collect(),
        })
    }

    fn verify_certificate(&self, qc: &Certificate, phase: Phase) -> bool {
        if qc.phase != phase || qc.votes.len() != self.quorum {
            return false;
        }
        let mut seen = BTreeSet::new();
        qc.votes.iter().all(|vote| {
            vote.view == qc.view
                && vote.phase == phase
                && vote.digest == qc.digest
                && self.verify_vote(vote).is_some_and(|pk| seen.insert(pk))
        })
    }

    fn certificate_hash(qc: &Certificate) -> [u8; 32] {
        let mut hash = blake3::Hasher::new_derive_key("aether DKG agreement certificate v1");
        hash.update(&qc.view.to_be_bytes());
        hash.update(&[qc.phase.tag()]);
        hash.update(&qc.digest);
        let mut votes = qc.votes.clone();
        votes.sort_by(|a, b| a.signer.cmp(&b.signer));
        for vote in votes {
            hash.update(&vote.signer);
            hash.update(&vote.signature);
        }
        *hash.finalize().as_bytes()
    }

    fn new_view_hash(best: &Option<Certificate>) -> [u8; 32] {
        best.as_ref().map(Self::certificate_hash).unwrap_or([0; 32])
    }

    fn sign_new_view(&mut self) -> NewView {
        let best = self.best.clone();
        let hash = Self::new_view_hash(&best);
        let message = self.envelope(3, self.view, &hash, &[]);
        let status = NewView {
            view: self.view,
            best,
            signer: self.me.encode().to_vec(),
            signature: self.key.sign(NAMESPACE, &message).encode().to_vec(),
        };
        self.new_views
            .entry(self.view)
            .or_default()
            .insert(self.me.clone(), status.clone());
        status
    }

    fn verify_new_view(&self, status: &NewView) -> Option<PublicKey> {
        if status.view > self.view.saturating_add(MAX_FUTURE_VIEWS) {
            return None;
        }
        if status.best.as_ref().is_some_and(|qc| {
            qc.view >= status.view || !self.verify_certificate(qc, Phase::Precommit)
        }) {
            return None;
        }
        let hash = Self::new_view_hash(&status.best);
        let message = self.envelope(3, status.view, &hash, &[]);
        self.verify_signature(&status.signer, &status.signature, &message)
    }

    fn proposal_hash(statuses: &[NewView]) -> [u8; 32] {
        let mut hash = blake3::Hasher::new_derive_key("aether DKG agreement view certificate v1");
        let mut statuses = statuses.to_vec();
        statuses.sort_by(|a, b| a.signer.cmp(&b.signer));
        for status in statuses {
            hash.update(&status.signer);
            hash.update(&status.signature);
            hash.update(&Self::new_view_hash(&status.best));
        }
        *hash.finalize().as_bytes()
    }

    fn proposer(&self, view: u64) -> &PublicKey {
        self.players
            .iter()
            .nth((view % self.players.len() as u64) as usize)
            .expect("nonempty DKG roster")
    }

    fn verify_proposal(&self, proposal: &Proposal) -> bool {
        if proposal.view > self.view.saturating_add(MAX_FUTURE_VIEWS)
            || proposal.new_views.len() != self.quorum
        {
            return false;
        }
        if self.signer(&proposal.signer).as_ref() != Some(self.proposer(proposal.view)) {
            return false;
        }
        let mut seen = BTreeSet::new();
        let mut highest: Option<&Certificate> = None;
        for status in &proposal.new_views {
            let Some(pk) = self.verify_new_view(status) else {
                return false;
            };
            if status.view != proposal.view || !seen.insert(pk) {
                return false;
            }
            if let Some(qc) = status.best.as_ref() {
                if highest.is_none_or(|old| qc.view > old.view) {
                    highest = Some(qc);
                } else if highest.is_some_and(|old| qc.view == old.view && qc.digest != old.digest)
                {
                    return false;
                }
            }
        }
        if highest.is_some_and(|qc| qc.digest != proposal.digest) {
            return false;
        }
        let hash = Self::proposal_hash(&proposal.new_views);
        let message = self.envelope(4, proposal.view, &proposal.digest, &hash);
        self.verify_signature(&proposal.signer, &proposal.signature, &message)
            .is_some()
    }

    fn store_best(&mut self, qc: Certificate) {
        if qc.view > self.view {
            self.view = qc.view;
            self.view_ticks = 0;
        }
        if self.best.as_ref().is_none_or(|old| qc.view > old.view) {
            self.best = Some(qc);
        }
    }

    fn store_new_view(&mut self, status: NewView) -> bool {
        let Some(pk) = self.verify_new_view(&status) else {
            return false;
        };
        if self
            .new_views
            .get(&status.view)
            .is_some_and(|reports| reports.contains_key(&pk))
        {
            return false;
        }
        if let Some(qc) = &status.best {
            self.store_best(qc.clone());
        }
        self.new_views
            .entry(status.view)
            .or_default()
            .insert(pk, status);
        true
    }

    fn store_proposal(&mut self, proposal: Proposal) -> bool {
        if !self.verify_proposal(&proposal) || self.proposals.contains_key(&proposal.view) {
            return false;
        }
        self.proposals.insert(proposal.view, proposal);
        true
    }

    fn progress(&mut self) -> Vec<AgreementMsg> {
        let mut out = Vec::new();
        if self.decided.is_some() {
            return out;
        }
        if self.players.position(&self.me).is_none() {
            return out;
        }
        if !self.sent_proposals.contains(&self.view) && self.proposer(self.view) == &self.me {
            if let Some(reports) = self.new_views.get(&self.view) {
                if reports.len() >= self.quorum {
                    let statuses: Vec<_> = reports.values().take(self.quorum).cloned().collect();
                    let highest = statuses
                        .iter()
                        .filter_map(|s| s.best.as_ref())
                        .max_by_key(|qc| qc.view);
                    let digest = highest.map(|qc| qc.digest).or(self.preferred);
                    if let Some(digest) = digest.filter(|d| self.available.contains(d)) {
                        let hash = Self::proposal_hash(&statuses);
                        let message = self.envelope(4, self.view, &digest, &hash);
                        let proposal = Proposal {
                            view: self.view,
                            digest,
                            new_views: statuses,
                            signer: self.me.encode().to_vec(),
                            signature: self.key.sign(NAMESPACE, &message).encode().to_vec(),
                        };
                        self.sent_proposals.insert(self.view);
                        self.proposals.insert(self.view, proposal.clone());
                        out.push(AgreementMsg::Proposal(proposal));
                    }
                }
            }
        }
        if let Some(proposal) = self.proposals.get(&self.view) {
            if self.available.contains(&proposal.digest)
                && !self
                    .own_votes
                    .contains(&(self.view, Phase::Precommit.tag()))
            {
                let proof_view = proposal
                    .new_views
                    .iter()
                    .filter_map(|s| s.best.as_ref().map(|qc| qc.view))
                    .max();
                let allowed = self.lock.as_ref().is_none_or(|lock| {
                    lock.digest == proposal.digest || proof_view.is_some_and(|v| v > lock.view)
                });
                if allowed {
                    if let Some(vote) = self.sign_vote(Phase::Precommit, proposal.digest) {
                        out.push(AgreementMsg::Vote(vote));
                    }
                }
            }
        }
        let precommit_qcs: Vec<_> = self
            .votes
            .keys()
            .filter(|(v, phase, _)| *v == self.view && *phase == Phase::Precommit.tag())
            .filter_map(|(_, _, digest)| {
                self.vote_certificate(self.view, Phase::Precommit, *digest)
            })
            .collect();
        for qc in precommit_qcs {
            self.store_best(qc.clone());
            if self.sent_certificates.insert((qc.view, qc.digest)) {
                out.push(AgreementMsg::PrecommitCertificate(qc.clone()));
            }
            if self.available.contains(&qc.digest)
                && !self.own_votes.contains(&(self.view, Phase::Commit.tag()))
            {
                self.lock = Some(qc.clone());
                if let Some(vote) = self.sign_vote(Phase::Commit, qc.digest) {
                    out.push(AgreementMsg::Vote(vote));
                }
            }
        }
        let commit_qcs: Vec<_> = self
            .votes
            .keys()
            .filter(|(_, phase, _)| *phase == Phase::Commit.tag())
            .filter_map(|(view, _, digest)| self.vote_certificate(*view, Phase::Commit, *digest))
            .collect();
        if let Some(qc) = commit_qcs.into_iter().next() {
            self.decided = Some(qc.clone());
            out.push(AgreementMsg::Decision(qc));
        }
        out
    }

    pub fn propose_available(&mut self, digest: [u8; 32]) -> Vec<AgreementMsg> {
        self.preferred = Some(digest);
        self.available.insert(digest);
        let out = self.progress();
        self.persist_or_stop();
        out
    }

    pub fn tick(&mut self, available: &BTreeSet<[u8; 32]>) -> Vec<AgreementMsg> {
        self.available.extend(available.iter().copied());
        self.ticks = self.ticks.saturating_add(1);
        if self.players.position(&self.me).is_none() {
            return Vec::new();
        }
        self.view_ticks = self.view_ticks.saturating_add(1);
        let mut out = Vec::new();
        if !self
            .new_views
            .get(&self.view)
            .is_some_and(|m| m.contains_key(&self.me))
        {
            out.push(AgreementMsg::NewView(self.sign_new_view()));
        }
        let duration =
            BASE_VIEW_TICKS.saturating_mul(1u64 << (self.view as u32).min(MAX_VIEW_BACKOFF));
        if self.decided.is_none() && self.view_ticks >= duration {
            self.view = self.view.saturating_add(1);
            self.view_ticks = 0;
            out.push(AgreementMsg::NewView(self.sign_new_view()));
        } else if self.ticks.is_multiple_of(REBROADCAST_TICKS) {
            if let Some(status) = self.new_views.get(&self.view).and_then(|m| m.get(&self.me)) {
                out.push(AgreementMsg::NewView(status.clone()));
            }
        }
        out.extend(self.progress());
        if self.ticks.is_multiple_of(REBROADCAST_TICKS) {
            if let Some(cert) = &self.decided {
                out.push(AgreementMsg::Decision(cert.clone()));
            } else {
                if let Some(proposal) = self
                    .proposals
                    .get(&self.view)
                    .filter(|p| p.signer == self.me.encode().to_vec())
                {
                    out.push(AgreementMsg::Proposal(proposal.clone()));
                }
                for ((view, _, _), votes) in &self.votes {
                    if *view == self.view {
                        if let Some(own) = votes.get(&self.me) {
                            out.push(AgreementMsg::Vote(own.clone()));
                        }
                    }
                }
                if let Some(qc) = self.best.as_ref().filter(|qc| qc.view == self.view) {
                    out.push(AgreementMsg::PrecommitCertificate(qc.clone()));
                }
            }
        }
        self.persist_or_stop();
        out
    }

    pub fn on_message(&mut self, from: &PublicKey, msg: AgreementMsg) -> Vec<AgreementMsg> {
        if self.players.position(from).is_none() {
            return Vec::new();
        }
        let accepted = match &msg {
            AgreementMsg::NewView(status) => self.store_new_view(status.clone()),
            AgreementMsg::Proposal(proposal) => self.store_proposal(proposal.clone()),
            AgreementMsg::Vote(vote) => self.store_vote(vote.clone()),
            AgreementMsg::PrecommitCertificate(qc)
                if self.verify_certificate(qc, Phase::Precommit) =>
            {
                let newer = self.best.as_ref().is_none_or(|old| qc.view > old.view);
                if newer {
                    self.store_best(qc.clone());
                }
                // Record each signed vote so another node can reconstruct the QC.
                for vote in &qc.votes {
                    self.store_vote(vote.clone());
                }
                newer
            }
            AgreementMsg::Decision(qc) if self.verify_certificate(qc, Phase::Commit) => {
                for vote in &qc.votes {
                    self.store_vote(vote.clone());
                }
                if self.decided.is_none() {
                    self.view = self.view.max(qc.view);
                    self.decided = Some(qc.clone());
                    true
                } else {
                    false
                }
            }
            _ => false,
        };
        let mut out = if accepted { vec![msg] } else { Vec::new() };
        out.extend(self.progress());
        self.persist_or_stop();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_utils::TryCollect;

    fn deliver(
        agreements: &mut [Agreement],
        members: &[PublicKey],
        sender: usize,
        msg: AgreementMsg,
    ) {
        let mut queue: Vec<(usize, usize, AgreementMsg)> = (0..members.len())
            .filter(|&to| to != sender)
            .map(|to| (sender, to, msg.clone()))
            .collect();
        let mut steps = 0;
        while let Some((from, to, msg)) = queue.pop() {
            steps += 1;
            assert!(steps < 20_000, "agreement gossip must settle");
            for answer in agreements[to].on_message(&members[from], msg) {
                for peer in 0..members.len() {
                    if peer != to {
                        queue.push((to, peer, answer.clone()));
                    }
                }
            }
        }
    }

    #[test]
    fn equivocated_first_view_recovers_without_faulty_vote() {
        let mut keys: Vec<_> = (1..=4).map(aether_light::devnet_validator_key).collect();
        keys.sort_by_key(|k| k.public_key());
        let members: Vec<_> = keys.iter().map(|k| k.public_key()).collect();
        let players: Set<_> = members.iter().cloned().try_collect().unwrap();
        let mut agreements: Vec<_> = keys
            .iter()
            .map(|k| Agreement::new(k.clone(), players.clone(), 19))
            .collect();
        let a = [11; 32];
        let b = [22; 32];
        // Player 0 is the Byzantine first proposer. It receives the three
        // honest view-change reports but offers two different valid digests.
        let mut statuses = Vec::new();
        for (i, agreement) in agreements.iter_mut().enumerate() {
            if i != 0 {
                agreement.propose_available(a);
                agreement.propose_available(b);
            }
            let first = agreement.tick(&[a, b].into_iter().collect());
            let status = first
                .into_iter()
                .find_map(|m| match m {
                    AgreementMsg::NewView(s) => Some(s),
                    _ => None,
                })
                .unwrap();
            statuses.push(status);
        }
        let cert = statuses[..3].to_vec();
        for (to, digest) in [(1, a), (2, b)] {
            let hash = Agreement::proposal_hash(&cert);
            let message = agreements[0].envelope(4, 0, &digest, &hash);
            let proposal = Proposal {
                view: 0,
                digest,
                new_views: cert.clone(),
                signer: members[0].encode().to_vec(),
                signature: keys[0].sign(NAMESPACE, &message).encode().to_vec(),
            };
            let replies = agreements[to].on_message(&members[0], AgreementMsg::Proposal(proposal));
            for reply in replies {
                if let AgreementMsg::Vote(_) = reply {
                    deliver(&mut agreements, &members, to, reply);
                }
            }
        }
        assert!(agreements[1..].iter().all(|a| a.decided().is_none()));
        // The faulty player stops. Three honest players enter a later view;
        // the rotating honest proposer carries any lock and decides one value.
        for _ in 0..80 {
            for i in 1..4 {
                let outbound = agreements[i].tick(&[a, b].into_iter().collect());
                for msg in outbound {
                    deliver(&mut agreements, &members, i, msg);
                }
            }
            if agreements[1..]
                .iter()
                .all(|agreement| agreement.decided().is_some())
            {
                break;
            }
        }
        let decided = agreements[1]
            .decided()
            .expect("honest quorum must recover after first-view equivocation");
        assert!(agreements[1..].iter().all(|a| a.decided() == Some(decided)));
    }

    #[test]
    fn restart_cannot_sign_again_in_the_same_view_and_keeps_lock() {
        let mut keys: Vec<_> = (1..=4).map(aether_light::devnet_validator_key).collect();
        keys.sort_by_key(|k| k.public_key());
        let players: Set<_> = keys.iter().map(|k| k.public_key()).try_collect().unwrap();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let temp = root.join("tmp");
        std::fs::create_dir_all(&temp).unwrap();
        let path = temp.join(format!(
            "dkg-agreement-restart-{}.journal",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let digest = [7; 32];
        let mut voters: Vec<_> = keys
            .iter()
            .map(|k| Agreement::new(k.clone(), players.clone(), 91))
            .collect();
        voters[0].attach_journal(path.clone()).unwrap();
        let votes: Vec<_> = voters[..3]
            .iter_mut()
            .map(|v| v.sign_vote(Phase::Precommit, digest).unwrap())
            .collect();
        let qc = Certificate {
            view: 0,
            phase: Phase::Precommit,
            digest,
            votes,
        };
        voters[0].lock = Some(qc.clone());
        voters[0].best = Some(qc);
        voters[0].persist_or_stop();
        drop(voters);
        let mut recovered = Agreement::new(keys[0].clone(), players, 91);
        recovered.attach_journal(path.clone()).unwrap();
        assert_eq!(recovered.lock.as_ref().map(|qc| qc.digest), Some(digest));
        assert!(
            recovered.sign_vote(Phase::Precommit, [8; 32]).is_none(),
            "one honest key cannot vote for a conflicting transcript after restart"
        );
        drop(recovered);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn seven_players_decide_with_two_silent_proposers() {
        let mut keys: Vec<_> = (1..=7).map(aether_light::devnet_validator_key).collect();
        keys.sort_by_key(|k| k.public_key());
        let members: Vec<_> = keys.iter().map(|k| k.public_key()).collect();
        let players: Set<_> = members.iter().cloned().try_collect().unwrap();
        let mut agreements: Vec<_> = keys
            .iter()
            .map(|k| Agreement::new(k.clone(), players.clone(), 93))
            .collect();
        let digest = [55; 32];
        for agreement in &mut agreements[2..] {
            agreement.propose_available(digest);
        }
        for _ in 0..100 {
            for i in 2..7 {
                for msg in agreements[i].tick(&[digest].into_iter().collect()) {
                    deliver(&mut agreements, &members, i, msg);
                }
            }
            if agreements[2..]
                .iter()
                .all(|agreement| agreement.decided() == Some(digest))
            {
                return;
            }
        }
        panic!("five honest players must outlast two silent leaders in a seven-player round");
    }
}
