//! Every follower busy (red-team 2026-09-29 §3): the one case where the load
//! moves back — the validators answer, so an all-busy pool cannot turn into a
//! denial of service. The caller still never sees the busy error, and the busy
//! follower is not hammered while it backs off. (One test per file: the wallet's
//! configuration is process-global.)

#[allow(dead_code)]
mod wallet_common;

use aether_ffi::{chain_status, pin_servers};
use wallet_common::{busy, follower};

#[test]
fn the_validators_answer_when_every_follower_is_busy() {
    let busy = busy();
    let validator = follower();
    pin_servers(vec![busy.pinned()], vec![validator.pinned()]).unwrap();

    for _ in 0..3 {
        assert_eq!(chain_status().unwrap().chain_id, 7_777);
    }

    assert_eq!(
        validator.total(),
        3,
        "the validators answer when every follower is busy"
    );
    assert!(
        busy.total() <= 3,
        "the busy follower backs off, it is not hammered: {}",
        busy.total()
    );
}
