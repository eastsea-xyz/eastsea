#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() <= 1 << 20 {
        let _ = aether_node::era::read(data, None);
    }
});
