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
    assert_eq!((c.registered_epoch, c.last_epoch, c.streak), (0, 2, 3));

    // Gone longer than the grace period: the streak restarts.
    assert!(net.call(&node, encode_beacon(key), 30 * EPOCH_BLOCKS));
    let c = &candidates(&net.state)[0];
    assert_eq!((c.last_epoch, c.streak), (30, 1));
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
