//! Wallet core against the live devnet, reached only through the Mainline DHT.
//! Run with the devnet up: cargo test -p aether-ffi --test dht_wallet -- --ignored --nocapture

#[test]
#[ignore = "needs a running devnet and internet (Mainline DHT)"]
fn wallet_reads_verified_state_via_dht() {
    let st = aether_ffi::chain_status().expect("status via DHT");
    println!("connected: {}", aether_ffi::connection());
    println!("chain {} height {}", st.chain_id, st.height);
    let dev1 = "0xf7F3faFCb3a47571B55d6d4991a9F22B4c3920e1".to_string();
    let acct = aether_ffi::verified_account(dev1, 4).expect("verified account");
    println!("dev1 balance {} wei · certified block {} · root {}", acct.balance_wei, acct.certified_block, acct.state_root);
    assert!(acct.balance_wei.len() > 20);
}
