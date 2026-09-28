//! A follower that serves state that does not verify is demoted (here: the
//! devnet fixture under another chain id — the wallet's chain check refuses
//! it, exactly as it would a lying validator), and the validators answer once
//! no follower is left standing.

#[allow(dead_code)]
mod wallet_common;

use aether_ffi::{
    chain_status, pin_servers, use_devnet_keys, verified_account, verified_height, wallet_servers,
};
use wallet_common::{dead, fixture_address, follower, liar};

#[test]
fn a_lying_follower_is_rejected_and_demoted_then_validators_answer() {
    let liar = liar();
    let broken = dead();
    let validator = follower();
    pin_servers(
        vec![liar.pinned(), broken.pinned()],
        vec![validator.pinned()],
    )
    .unwrap();
    use_devnet_keys();

    // The read fails the wallet's own verification; nothing of the liar's
    // state is believed, and no verified height is recorded from it.
    let err = verified_account(fixture_address(), 4)
        .map(|_| ())
        .unwrap_err()
        .to_string();
    assert!(err.contains("chain 999"), "{err}");
    assert_eq!(
        verified_height(),
        0,
        "a failed verification leaves no floor behind"
    );

    // The liar is demoted for a long while; the Mac that went away is only
    // rotated past briefly. Neither serves another read.
    let servers = wallet_servers();
    let by_node = |n: &str| servers.iter().find(|s| s.node == n).expect("in the pool");
    let l = by_node(&liar.node);
    assert_eq!(
        l.parked.as_deref(),
        Some("lying"),
        "the liar is demoted: {l:?}"
    );
    assert!(
        l.parked_for_ms.unwrap_or_default() > 8 * 60_000,
        "lying is a long demotion: {:?}",
        l.parked_for_ms
    );
    assert_eq!(by_node(&broken.node).parked.as_deref(), Some("error"));
    let (liar_total, broken_total) = (liar.total(), broken.total());
    assert_eq!(
        liar_total, 3,
        "the account, the certified block and the status all came from the liar"
    );

    // No follower left: the validators answer.
    assert_eq!(chain_status().unwrap().chain_id, 7_777);
    assert_eq!(
        validator.total(),
        1,
        "the fallback asked the validators once"
    );
    assert_eq!(
        (liar.total(), broken.total()),
        (liar_total, broken_total),
        "a demoted follower serves nothing"
    );
}
