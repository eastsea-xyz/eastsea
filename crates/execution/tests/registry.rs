//! CommitteeRegistry: registrar-attested candidates, beacons, streaks, and the
//! storage layout nodes read to select voting nodes.

use aether_crypto::{address_of, P256Signer, Signer};
use aether_execution::registry::{self, attestation_message, candidates, encode_beacon, encode_register, EPOCH_BLOCKS, REGISTRY};
use aether_execution::{execute_block, sign_call, BlockContext, EvmCall, WorldState};
use aether_types::{Address, GasVector, U256};

const CHAIN: u64 = 7_777;

fn seed(b: u8) -> [u8; 32] {
    let mut s = [0u8; 32];
    s[0] = 0x3e;
    s[31] = b;
    s
}

fn at(block: u64) -> BlockContext {
    BlockContext {
        chain_id: CHAIN,
        number: block,
        timestamp: block,
        beneficiary: Address::repeat_byte(0xbe),
        limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
        fees: None,
    }
}

struct Net {
    state: WorldState,
    registrar: P256Signer,
    nonces: std::collections::HashMap<Address, u64>,
}

impl Net {
    fn new() -> Self {
        let registrar = P256Signer::from_seed(&seed(1)).unwrap();
        let mut state = WorldState::default();
        registry::predeploy(
            &mut state,
            aether_crypto::p256_xy(&registrar.public_key().bytes).unwrap(),
            registry::Params { epoch_blocks: EPOCH_BLOCKS, ..Default::default() },
        )
        .unwrap();
        Net { state, registrar, nonces: Default::default() }
    }

    fn fund(&mut self, s: &P256Signer) -> Address {
        let a = address_of(&s.public_key()).unwrap();
        self.state.set_balance(a, U256::from(10u128.pow(20))).unwrap();
        a
    }

    fn call(&mut self, from: &P256Signer, input: aether_types::Bytes, block: u64) -> bool {
        let a = address_of(&from.public_key()).unwrap();
        let n = self.nonces.entry(a).or_default();
        let tx = sign_call(from, CHAIN, *n, 1, &EvmCall { to: Some(REGISTRY), value: U256::ZERO, input, gas_limit: 400_000, delegate: None }).unwrap();
        *n += 1;
        let out = execute_block(&self.state, &at(block), &[tx]).unwrap();
        self.state = out.state;
        out.receipts[0].success
    }

    /// The registrar's attestation (after DeviceCheck accepted the device).
    fn attest(&self, operator: Address, key: [u8; 32], node: [u8; 32], beaconer: Address) -> ([u8; 32], [u8; 32]) {
        let sig = self.registrar.sign(&attestation_message(CHAIN, operator, key, node, beaconer)).unwrap();
        (sig[..32].try_into().unwrap(), sig[32..64].try_into().unwrap())
    }
}

#[test]
fn attested_macs_register_once_and_beacon_their_streak() {
    let mut net = Net::new();
    let owner = P256Signer::from_seed(&seed(2)).unwrap(); // the owner's wallet (Touch ID)
    let node = P256Signer::from_seed(&seed(3)).unwrap(); // the node's own account (unattended)
    let op = net.fund(&owner);
    let beaconer = net.fund(&node);
    let (key, node_id) = ([0x11; 32], [0x22; 32]);

    let (r, s) = net.attest(op, key, node_id, beaconer);
    let forged = net.attest(op, key, node_id, Address::repeat_byte(0x66));
    assert!(!net.call(&owner, encode_register(key, node_id, beaconer, forged.0, forged.1), 10), "attestation for other data");
    assert!(net.call(&owner, encode_register(key, node_id, beaconer, r, s), 10));
    assert!(!net.call(&owner, encode_register(key, node_id, beaconer, r, s), 11), "a validator key registers once");

    // Beacons: only the node's account; one per epoch builds the streak.
    assert!(!net.call(&owner, encode_beacon(key), EPOCH_BLOCKS + 1), "the operator is not the beaconer");
    assert!(net.call(&node, encode_beacon(key), EPOCH_BLOCKS + 1));
    assert!(net.call(&node, encode_beacon(key), 2 * EPOCH_BLOCKS + 5));
    assert!(net.call(&node, encode_beacon(key), 2 * EPOCH_BLOCKS + 6), "a second beacon in the same epoch is a no-op");
    let c = &candidates(&net.state)[0];
    assert_eq!((c.operator, c.beaconer, c.validator_key, c.node_id), (op, beaconer, key, node_id));
    assert_eq!((c.registered_epoch, c.last_epoch, c.streak, c.missed), (0, 2, 3, 0));

    // Two epochs asleep (within the grace period): the streak goes on, the misses count.
    assert!(net.call(&node, encode_beacon(key), 5 * EPOCH_BLOCKS));
    let c = &candidates(&net.state)[0];
    assert_eq!((c.last_epoch, c.streak, c.missed), (5, 4, 2));

    // Gone longer than the grace period: the streak and misses restart.
    assert!(net.call(&node, encode_beacon(key), 40 * EPOCH_BLOCKS));
    let c = &candidates(&net.state)[0];
    assert_eq!((c.last_epoch, c.streak, c.missed), (40, 1, 0));
}

#[test]
fn only_the_registrar_can_attest() {
    let mut net = Net::new();
    let owner = P256Signer::from_seed(&seed(4)).unwrap();
    let op = net.fund(&owner);
    let impostor = P256Signer::from_seed(&seed(5)).unwrap();
    let sig = impostor.sign(&attestation_message(CHAIN, op, [1; 32], [2; 32], op)).unwrap();
    let (r, s): ([u8; 32], [u8; 32]) = (sig[..32].try_into().unwrap(), sig[32..64].try_into().unwrap());
    assert!(!net.call(&owner, encode_register([1; 32], [2; 32], op, r, s), 5));
    assert!(candidates(&net.state).is_empty());
}

#[test]
fn protocol_2_bounds_registrations_per_epoch_and_keeps_candidates() {
    let mut net = Net::new();
    let owner = P256Signer::from_seed(&seed(2)).unwrap();
    let node = P256Signer::from_seed(&seed(3)).unwrap();
    let op = net.fund(&owner);
    let beaconer = net.fund(&node);
    let reg = |net: &mut Net, k: u8, block: u64| {
        let (key, id) = ([k; 32], [k.wrapping_add(100); 32]);
        let (r, s) = net.attest(op, key, id, beaconer);
        net.call(&owner, encode_register(key, id, beaconer, r, s), block)
    };
    assert!(reg(&mut net, 1, 10), "registered under v1");

    // Protocol 2 swaps in v2 (as its activation block does); a limit of 2 per epoch for the test.
    aether_execution::forks::activate(2, &mut net.state).unwrap();
    net.state.set_storage(REGISTRY, U256::from(7u64), U256::from(2u64));
    assert_eq!(candidates(&net.state).len(), 1, "existing candidates stay");
    assert!(net.call(&node, encode_beacon([1; 32]), EPOCH_BLOCKS + 1), "and keep beaconing");
    assert!(reg(&mut net, 2, EPOCH_BLOCKS + 2));
    assert!(reg(&mut net, 3, EPOCH_BLOCKS + 3));
    assert!(!reg(&mut net, 4, EPOCH_BLOCKS + 4), "a third in one epoch is refused");
    assert!(reg(&mut net, 4, 2 * EPOCH_BLOCKS), "the next epoch has room again");
    assert_eq!(candidates(&net.state).len(), 4);

    // A committee-signed upgrade can replace the registrar (zeros stop registrations).
    registry::set_registrar(&mut net.state, ([0; 32], [0; 32]));
    assert!(!reg(&mut net, 5, 3 * EPOCH_BLOCKS), "no registrar, no new candidates");
}

#[test]
fn a_rotated_registrar_signs_and_a_revoked_one_goes_silent() {
    let mut net = Net::new();
    let owner = P256Signer::from_seed(&seed(2)).unwrap();
    let node = P256Signer::from_seed(&seed(3)).unwrap();
    let op = net.fund(&owner);
    let beaconer = net.fund(&node);
    let reg = |net: &mut Net, k: u8, block: u64| {
        let (key, id) = ([k; 32], [k.wrapping_add(100); 32]);
        let (r, s) = net.attest(op, key, id, beaconer);
        net.call(&owner, encode_register(key, id, beaconer, r, s), block)
    };

    assert!(!registry::registrar_revoked(&net.state));
    assert_eq!(
        registry::registrar(&net.state),
        aether_crypto::p256_xy(&net.registrar.public_key().bytes).unwrap()
    );
    assert!(reg(&mut net, 1, 5));

    // The committee rotates the key in a signed upgrade. Attestations signed
    // with the old key die at once — they are what the new registry no longer
    // holds — and this node has to switch to the new key (docs/ops/registrar.md).
    let next = P256Signer::from_seed(&seed(9)).unwrap();
    registry::set_registrar(&mut net.state, aether_crypto::p256_xy(&next.public_key().bytes).unwrap());
    assert!(!registry::registrar_revoked(&net.state), "a rotated registrar is not a stopped one");
    assert!(!reg(&mut net, 2, 6), "the retired key no longer attests");
    net.registrar = next;
    assert!(reg(&mut net, 3, 7), "the key the committee installed does");

    // A second upgrade zeroes both halves: revocation. Nothing attests any more,
    // and the candidates already registered stay (the committee still votes on
    // them; only new registrations stop).
    let candidates_before = candidates(&net.state).len();
    registry::set_registrar(&mut net.state, ([0; 32], [0; 32]));
    assert!(registry::registrar_revoked(&net.state));
    assert!(!reg(&mut net, 4, 8), "a revoked registrar attests nothing");
    assert_eq!(net.state.storage(&REGISTRY, U256::ZERO), U256::ZERO);
    assert_eq!(net.state.storage(&REGISTRY, U256::from(1u64)), U256::ZERO);
    assert_eq!(candidates(&net.state).len(), candidates_before, "registered candidates stay");
}
