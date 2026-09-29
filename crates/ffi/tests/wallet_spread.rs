//! Reads spread over follower Macs instead of the validators (capacity review
//! 2026-09-29): five fake followers on this Mac, and the validators watching.
//! The reads land on several followers, none takes more than its share, and
//! the validators are not asked at all. Localhost iroh only: no relay, no DHT.

#[allow(dead_code)]
mod wallet_common;

use aether_ffi::{account_history, chain_status, pin_servers};
use wallet_common::{fixture_address, follower};

#[test]
fn reads_spread_over_followers_and_never_reach_the_validators() {
    let followers: Vec<_> = (0..5).map(|_| follower()).collect();
    let validator = follower();
    pin_servers(
        followers.iter().map(|f| f.pinned()).collect(),
        vec![validator.pinned()],
    )
    .unwrap();

    for _ in 0..12 {
        assert_eq!(chain_status().unwrap().chain_id, 7_777);
    }
    let history = account_history(fixture_address(), None, 50).unwrap();
    assert!(history.contains("history_start"));

    assert_eq!(
        validator.total(),
        0,
        "the validators are not asked while followers answer"
    );
    let counts: Vec<usize> = followers.iter().map(|f| f.total()).collect();
    let distinct = counts.iter().filter(|c| **c > 0).count();
    assert!(
        distinct >= 3,
        "requests spread over several followers: {counts:?}"
    );
    assert!(
        counts.iter().all(|c| *c <= 7),
        "no follower takes more than its share: {counts:?}"
    );
    assert_eq!(
        counts.iter().sum::<usize>(),
        13,
        "every read was answered by a follower: {counts:?}"
    );
}
