//! A follower that only lags (its answers verify, but against old state) is
//! passed over briefly, not demoted: a follower cannot lie — every answer is
//! checked the same as a validator's — it can only be slow or stale.

#[allow(dead_code)]
mod wallet_common;

use aether_ffi::{
    chain_status, pin_servers, use_devnet_keys, verified_account, verified_height, wallet_servers,
};
use wallet_common::{fixture_address, follower};

#[test]
fn a_stale_follower_is_passed_over_not_demoted() {
    let lagging = follower();
    let validator = follower();
    pin_servers(vec![lagging.pinned()], vec![validator.pinned()]).unwrap();
    use_devnet_keys();

    // The captured devnet certificate really does verify — and the state is
    // long since captured, so it is refused as stale.
    let err = verified_account(fixture_address(), 4)
        .map(|_| ())
        .unwrap_err()
        .to_string();
    assert!(err.contains("stale"), "{err}");
    assert_eq!(
        verified_height(),
        6,
        "the certified height is kept even when too old to show"
    );

    let servers = wallet_servers();
    assert_eq!(servers.len(), 1);
    let s = &servers[0];
    assert_eq!(s.parked.as_deref(), Some("stale"));
    assert!(
        s.parked_for_ms.unwrap_or_default() <= 60_000,
        "stale is a brief pass-over: {:?}",
        s.parked_for_ms
    );
    assert!(!s.active, "the rotation drops it while it is passed over");

    // While it is passed over, the validator answers.
    assert_eq!(chain_status().unwrap().chain_id, 7_777);
    assert_eq!(validator.total(), 1);
}
