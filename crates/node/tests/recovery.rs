//! `AETHER_RECOVER_CONSENSUS` (docs/ops/consensus-recovery.md): the rules
//! `engine::recover` enforces before a node starts consensus again from a
//! stored finalization. Audit 4 A4-3: the last finalized view is not evidence
//! of the highest view this signer may have voted in, so a FIRST recovery —
//! the vote journal partition does not exist yet — always refuses and names
//! the safe path (follow until the next committee round): a fresh journal
//! under the floor could sign a view this Mac already signed, and a Byzantine
//! member holding the first vote can fork the chain with the two. What stays
//! is the restart of a past recovery: the journal that recovery created
//! still exists, replays its votes, and votes exactly as before. A wrong
//! view, a height nothing is stored at, and garbage still refuse to start;
//! an earlier committee epoch's setting is ignored (it stays set after a
//! recovery, so it must never brick a later epoch).

use aether_node::archive::{prunable_config, Buffers, FinalizedCerts};
use aether_node::block::EPOCH;
use aether_node::engine::{self, Finalization};
use commonware_codec::ReadExt as _;
use commonware_consensus::marshal::store::Certificates;
use commonware_consensus::simplex::scheme::bls12381_threshold::vrf;
use commonware_consensus::simplex::types::Proposal;
use commonware_consensus::types::{Epoch, Height, Round, View};
use commonware_consensus::{Epochable as _, Viewable as _};
use commonware_cryptography::bls12381::primitives::group::G1;
use commonware_cryptography::{Hasher as _, Sha256};
use commonware_runtime::buffer::paged::{page_size, CacheRef};
use commonware_runtime::{deterministic, Runner as _, Storage, Supervisor as _};
use commonware_storage::archive::prunable;
use commonware_utils::NZUsize;

/// The block digest stored with the finalization at `height`.
fn digest(height: u64) -> commonware_cryptography::sha256::Digest {
    Sha256::hash(&[height.to_be_bytes().as_slice()])
}

/// A valid G1 point (the standard generator, compressed): recovery only reads
/// the stored certificate back, nobody verifies these signatures here.
fn g1() -> G1 {
    let bytes: [u8; 48] = hex::decode("97f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb")
        .unwrap()
        .try_into()
        .unwrap();
    G1::read(&mut &bytes[..]).unwrap()
}

/// A finalization certificate for `height` at `view` in `epoch`.
fn finalization_at(epoch: Epoch, view: u64, height: u64) -> Finalization {
    Finalization {
        proposal: Proposal::new(
            Round::new(epoch, View::new(view)),
            View::new(view.saturating_sub(1)),
            digest(height),
        ),
        certificate: vrf::Certificate::from(vrf::Signature {
            vote_signature: g1(),
            seed_signature: g1(),
        }),
    }
}

fn finalization(view: u64, height: u64) -> Finalization {
    finalization_at(EPOCH, view, height)
}

/// The certificate archive a node reopens on (the pruning layout), holding
/// finalizations for heights 0..=20, each at view 100 + height: the last
/// stored finalization is height 20 at view 120.
async fn archive_with(context: &deterministic::Context, buffers: Buffers) -> FinalizedCerts<deterministic::Context> {
    let cache = CacheRef::from_pooler(context, page_size(4096), NZUsize!(64));
    let mut certs = FinalizedCerts::Prunable(
        prunable::Archive::init(
            context.child("finalizations"),
            prunable_config("v1", "finalizations", cache, (), buffers),
        )
        .await
        .unwrap(),
    );
    for h in 0..=20u64 {
        certs = Certificates::put(certs, Height::new(h), digest(h), finalization(100 + h, h))
            .await
            .unwrap();
    }
    certs = Certificates::sync(certs).await.unwrap();
    assert_eq!(Certificates::last_index(&certs), Some(Height::new(20)));
    certs
}

/// A4-3: the override must never start voting from a journal that does not
/// exist. Even the last stored finalization (the old runbook's only allowed
/// first recovery) is not evidence of the highest view this Mac may have
/// signed: its old votes above the floor can be held by a Byzantine member,
/// so voting there again with the same committee share is a double vote.
/// Every first recovery refuses, and the refusal names the safe path.
#[test]
fn a_first_recovery_refuses_and_names_the_safe_path() {
    let buffers = Buffers {
        write: NZUsize!(1 << 16),
        replay: NZUsize!(1 << 16),
    };
    deterministic::Runner::default().start_and_recover(|context| async move {
        let certs = archive_with(&context, buffers).await;

        // The old happy path — <view>@<height> naming the last stored
        // finalization — is exactly the unsafe one: it must refuse.
        let err = engine::recover(&context, &certs, "v1", EPOCH, "120@20")
            .await
            .unwrap_err();
        assert_refusal(&err, "120@20", "v1-consensus-r120");
        // The bare <view> form (the last stored finalization) refuses too.
        let err = engine::recover(&context, &certs, "v1", EPOCH, "120")
            .await
            .unwrap_err();
        assert_refusal(&err, "120", "v1-consensus-r120");
        // An older height is the same first-recovery refusal (no fork-away
        // rule is needed any more: nothing without a journal may start).
        let err = engine::recover(&context, &certs, "v1", EPOCH, "114@14")
            .await
            .unwrap_err();
        assert_refusal(&err, "114@14", "v1-consensus-r114");
    });
}

/// The refusal tells the operator what to do instead: leave the setting
/// unset, so this Mac follows (and serves) until the next committee round
/// starts a fresh share and journal.
fn assert_refusal(err: &str, value: &str, partition: &str) {
    assert!(err.contains(value), "{err}");
    assert!(err.contains(partition), "{err}");
    assert!(err.contains("does not exist"), "{err}");
    assert!(err.contains("next committee round"), "{err}");
}

/// What survives of the override (audit 4 A4-3): restarting a past
/// recovery. The journal that recovery created is on disk, so the node
/// replays its own votes and resumes exactly as before — the same value
/// must keep working across restarts, or every past recovery would brick
/// on upgrade. Anything the journal does not back still refuses.
#[test]
fn a_past_recoverys_journal_restarts_and_votes_as_before() {
    let buffers = Buffers {
        write: NZUsize!(1 << 16),
        replay: NZUsize!(1 << 16),
    };
    let (_, checkpoint) =
        deterministic::Runner::default().start_and_recover(|context| async move {
            let certs = archive_with(&context, buffers).await;

            // A wrong view, a height nothing is stored at, and garbage still refuse.
            let err = engine::recover(&context, &certs, "v1", EPOCH, "999@14")
                .await
                .unwrap_err();
            assert!(err.contains("is at view 114"), "{err}");
            let err = engine::recover(&context, &certs, "v1", EPOCH, "121@99")
                .await
                .unwrap_err();
            assert!(err.contains("no finalization is stored at height 99"), "{err}");
            let err = engine::recover(&context, &certs, "v1", EPOCH, "what")
                .await
                .unwrap_err();
            assert!(err.contains("expected <view>@<height>"), "{err}");

            // The setting outlives a committee epoch change (runbook step 5-6):
            // an earlier epoch's value is ignored, never a refusal — even with
            // no journal for it.
            assert!(
                engine::recover(&context, &certs, "v1", Epoch::new(1), "114@14")
                    .await
                    .unwrap()
                    .is_none()
            );

            // A past recovery left its journal behind; restarting with the same
            // value replays that journal and votes as before.
            context.open("v1-consensus-r114", b"0").await.unwrap();
            let f = engine::recover(&context, &certs, "v1", EPOCH, "114@14")
                .await
                .unwrap()
                .expect("restart after a past recovery");
            assert_eq!(f.view().get(), 114);
            // A view whose journal never existed still refuses.
            let err = engine::recover(&context, &certs, "v1", EPOCH, "113@13")
                .await
                .unwrap_err();
            assert_refusal(&err, "113@13", "v1-consensus-r113");
        });
    // The journal partition and the archives survive the restart.
    deterministic::Runner::from(checkpoint).start(|context| async move {
        let certs = archive_with(&context, buffers).await;
        // The same value on a restart of a past recovery: still allowed.
        let f = engine::recover(&context, &certs, "v1", EPOCH, "114@14")
            .await
            .unwrap()
            .expect("same value on restart");
        assert_eq!((f.view().get(), f.epoch().get()), (114, 0));
        // The journal for another view appears after a recovery there too.
        context.open("v1-consensus-r120", b"0").await.unwrap();
        assert!(engine::recover(&context, &certs, "v1", EPOCH, "120@20")
            .await
            .unwrap()
            .is_some());
        // And a journal-less view still refuses after that.
        let err = engine::recover(&context, &certs, "v1", EPOCH, "119@19")
            .await
            .unwrap_err();
        assert_refusal(&err, "119@19", "v1-consensus-r119");
    });
}
