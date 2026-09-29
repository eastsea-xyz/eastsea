#![no_main]

use aether_execution::WorldState;
use aether_light::block::BeaconAnswer;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 64 << 10 {
        return;
    }
    if let Ok(answer) = serde_json::from_slice::<BeaconAnswer>(data) {
        let _ = aether_node::beacons::verify(&WorldState::default(), 7_777, 1, &answer);
    }
});
