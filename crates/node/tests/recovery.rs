//! `AETHER_RECOVER_CONSENSUS` (docs/ops/consensus-recovery.md): the rules
//! `engine::recover` enforces before a node starts consensus again from a
//! stored finalization. The first recovery — the vote journal partition does
//! not exist yet — may only name the last finalization the node itself stored
//! (the runbook's step 1: starting lower would fork away finalizations other
//! validators kept). Once that journal exists, restarts keep working with the
//! same value. A wrong view, a height nothing is stored at, and garbage still
//! refuse to start; an earlier committee epoch's setting is ignored (it stays
//! set after a recovery, so it must never brick a later epoch).

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

/// The certificate archive a node reopens on (the pruning layout).
async fn open(
    context: &deterministic::Context,
    buffers: Buffers,
) -> FinalizedCerts<deterministic::Context> {
    let cache = CacheRef::from_pooler(context, page_size(4096), NZUsize!(64));
    FinalizedCerts::Prunable(
        prunable::Archive::init(
            context.child("finalizations"),
            prunable_config("v1", "finalizations", cache, (), buffers),
        )
        .await
        .unwrap(),
    )
}

#[test]
fn first_recovery_must_name_the_last_stored_finalization() {
    let buffers = Buffers {
        write: NZUsize!(1 << 16),
        replay: NZUsize!(1 << 16),
    };
    // Finalizations for heights 0..=20, each at view 100 + height: the last
    // stored finalization is height 20 at view 120.
    let (_, checkpoint) =
        deterministic::Runner::default().start_and_recover(|context| async move {
            let mut certs = open(&context, buffers).await;
            for h in 0..=20u64 {
                certs =
                    Certificates::put(certs, Height::new(h), digest(h), finalization(100 + h, h))
                        .await
                        .unwrap();
            }
            certs = Certificates::sync(certs).await.unwrap();
            assert_eq!(Certificates::last_index(&certs), Some(Height::new(20)));

            // The runbook's happy path: <view>@<height> naming the last stored finalization.
            let f = engine::recover(&context, &certs, "v1", EPOCH, "120@20")
                .await
                .unwrap()
                .expect("recovers");
            assert_eq!(f.view().get(), 120);
            // The bare <view> form means the last stored finalization: the same.
            assert!(engine::recover(&context, &certs, "v1", EPOCH, "120")
                .await
                .unwrap()
                .is_some());

            // An older height is a fork away from what this node stored: the first
            // recovery (no `v1-consensus-r114` journal yet) refuses to start.
            let err = engine::recover(&context, &certs, "v1", EPOCH, "114@14")
                .await
                .unwrap_err();
            assert!(
                err.contains("must name the last stored finalization, height 20"),
                "{err}"
            );
            // A wrong view and a height nothing is stored at still refuse.
            let err = engine::recover(&context, &certs, "v1", EPOCH, "999@14")
                .await
                .unwrap_err();
            assert!(err.contains("is at view 114"), "{err}");
            let err = engine::recover(&context, &certs, "v1", EPOCH, "121@99")
                .await
                .unwrap_err();
            assert!(
                err.contains("no finalization is stored at height 99"),
                "{err}"
            );
            let err = engine::recover(&context, &certs, "v1", EPOCH, "what")
                .await
                .unwrap_err();
            assert!(err.contains("expected <view>@<height>"), "{err}");

            // The setting outlives a committee epoch change (runbook step 5-6): an
            // earlier epoch's value is ignored, never a refusal — even though no
            // journal exists for it and its height is below the last stored one.
            assert!(
                engine::recover(&context, &certs, "v1", Epoch::new(1), "114@14")
                    .await
                    .unwrap()
                    .is_none()
            );

            // Consensus starts from the first recovery and creates its vote journal.
            context.open("v1-consensus-r114", b"0").await.unwrap();
            // A restart after that recovery keeps the same value: allowed as today.
            let f = engine::recover(&context, &certs, "v1", EPOCH, "114@14")
                .await
                .unwrap()
                .expect("restart after recovery");
            assert_eq!(f.view().get(), 114);
        });
    // The journal partition and the archives survive the restart.
    deterministic::Runner::from(checkpoint).start(|context| async move {
        let certs = open(&context, buffers).await;
        assert_eq!(Certificates::last_index(&certs), Some(Height::new(20)));
        let f = engine::recover(&context, &certs, "v1", EPOCH, "114@14")
            .await
            .unwrap()
            .expect("same value on restart");
        assert_eq!((f.view().get(), f.epoch().get()), (114, 0));
        // A view whose journal never existed still needs the last height.
        let err = engine::recover(&context, &certs, "v1", EPOCH, "113@13")
            .await
            .unwrap_err();
        assert!(
            err.contains("must name the last stored finalization, height 20"),
            "{err}"
        );
        // The journal for another view appears after a recovery there too.
        context.open("v1-consensus-r120", b"0").await.unwrap();
        assert!(engine::recover(&context, &certs, "v1", EPOCH, "120@20")
            .await
            .unwrap()
            .is_some());
    });
}
