//! Politeness: a follower over its limits ("server busy", the same DoS limits
//! the transport enforces) is answer enough. The caller never sees the busy
//! error, the load is not moved to the validators, and while the follower
//! backs off the rotation simply goes around it.

#[allow(dead_code)]
mod wallet_common;

use aether_ffi::{chain_status, pin_servers};
use wallet_common::{busy, follower};

#[test]
fn a_busy_follower_backs_off_without_bothering_the_validators() {
    let busy = busy();
    let healthy = follower();
    let validator = follower();
    pin_servers(
        vec![busy.pinned(), healthy.pinned()],
        vec![validator.pinned()],
    )
    .unwrap();

    // Two followers, one rotation: the busy one is asked once — the read that
    // reaches it simply goes around it, so the caller never sees the busy
    // error — and it is passed over while it backs off.
    for _ in 0..4 {
        assert_eq!(chain_status().unwrap().chain_id, 7_777);
    }

    assert_eq!(
        busy.total(),
        1,
        "asked once, then passed over: {}",
        busy.total()
    );
    assert_eq!(
        healthy.total(),
        4,
        "every read still gets an answer: {}",
        healthy.total()
    );
    assert_eq!(
        validator.total(),
        0,
        "a busy follower is not a reason to ask the validators"
    );
}
