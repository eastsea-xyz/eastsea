//! When this validator votes (docs/ops/consensus-recovery.md, roadmap F-P0).
//!
//! Two local rules, neither of which changes what a valid block or vote is:
//!
//! 1. Every check happens before the notarize vote. The engine wraps the
//!    application in marshal's `Inline` adapter, so `Application::verify`
//!    (execution and the FOCIL inclusion-list rule) decides the notarize vote.
//!    Certification then only waits for the block to be on disk, which every
//!    honest validator decides the same way. With `Deferred` (before
//!    2026-09-28) the notarize vote only checked that the block had arrived and
//!    `verify` decided certification; the inclusion-list rule depends on what
//!    each validator has seen, so a notarized block could be certified by half
//!    the committee and refused by the other half. Then the view can be neither
//!    finalized nor nullified and the chain stops (testnet 7780, height 69651).
//!
//! 2. No notarize vote for a block that is not durably stored here.
//!    [`DurableVote`] holds a `true` verdict until marshal has synced the block,
//!    so every notarization is backed by at least f+1 honest validators that
//!    hold the block on disk and can serve it after a crash or restart.

use crate::block::{Block, PublicKey};
use aether_light::Scheme;
use commonware_actor::Feedback;
use commonware_consensus::{
    marshal::{
        core::{DigestFallback, Mailbox},
        standard::Standard,
    },
    simplex::{types::Context, Plan},
    types::Round,
    Automaton, CertifiableAutomaton, Relay,
};
use commonware_cryptography::sha256::Digest;
use commonware_runtime::Spawner;
use commonware_utils::channel::oneshot;
use std::sync::{Arc, Mutex};

/// Wraps a marshal adapter so a `true` notarize verdict waits for the block to
/// be durable in marshal's verified cache. Proposals, certification and relay
/// pass through unchanged.
pub struct DurableVote<E, A> {
    /// Runtime contexts are not `Clone`; spawn from children of a shared one.
    context: Arc<Mutex<E>>,
    inner: A,
    marshal: Mailbox<Scheme, Standard<Block>>,
}

impl<E, A: Clone> Clone for DurableVote<E, A> {
    fn clone(&self) -> Self {
        Self { context: self.context.clone(), inner: self.inner.clone(), marshal: self.marshal.clone() }
    }
}

impl<E, A> DurableVote<E, A> {
    pub fn new(context: E, inner: A, marshal: Mailbox<Scheme, Standard<Block>>) -> Self {
        Self { context: Arc::new(Mutex::new(context)), inner, marshal }
    }
}

impl<E, A> Automaton for DurableVote<E, A>
where
    E: Spawner,
    A: Automaton<Context = Context<Digest, PublicKey>, Digest = Digest>,
{
    type Context = Context<Digest, PublicKey>;
    type Digest = Digest;

    async fn propose(&mut self, context: Self::Context) -> oneshot::Receiver<Self::Digest> {
        self.inner.propose(context).await
    }

    async fn verify(&mut self, context: Self::Context, digest: Self::Digest) -> oneshot::Receiver<bool> {
        let verdict = self.inner.verify(context.clone(), digest).await;
        let marshal = self.marshal.clone();
        let round = context.round;
        // A boundary re-proposal names a block marshal already holds durably.
        let reproposal = digest == context.parent.1;
        let (tx, rx) = oneshot::channel();
        let spawner = self.context.lock().expect("voter context lock").child("durable_vote").with_attribute("round", round);
        spawner.spawn(move |_| async move {
            // A closed verdict means verification can never conclude: close ours too.
            let Ok(valid) = verdict.await else { return };
            if !valid || reproposal {
                let _ = tx.send(valid);
                return;
            }
            if durable(&marshal, round, digest).await {
                let _ = tx.send(true);
            }
        });
        rx
    }
}

/// Persists the (already verified, so locally held) block for `round` and
/// waits for the sync. A duplicate of the adapter's own write is a no-op whose
/// sync covers the original. False only when marshal is shutting down.
async fn durable(marshal: &Mailbox<Scheme, Standard<Block>>, round: Round, digest: Digest) -> bool {
    let Ok(block) = marshal.subscribe_by_digest(digest, DigestFallback::Wait).await else {
        return false;
    };
    marshal.verified(round, block).await
}

impl<E, A> CertifiableAutomaton for DurableVote<E, A>
where
    E: Spawner,
    A: CertifiableAutomaton<Context = Context<Digest, PublicKey>, Digest = Digest>,
{
    async fn certify(&mut self, round: Round, digest: Self::Digest) -> oneshot::Receiver<bool> {
        self.inner.certify(round, digest).await
    }
}

impl<E, A> Relay for DurableVote<E, A>
where
    E: Send + 'static,
    A: Relay<Digest = Digest, PublicKey = PublicKey, Plan = Plan<PublicKey>>,
{
    type Digest = Digest;
    type PublicKey = PublicKey;
    type Plan = Plan<PublicKey>;

    fn broadcast(&mut self, digest: Self::Digest, plan: Self::Plan) -> Feedback {
        self.inner.broadcast(digest, plan)
    }
}
