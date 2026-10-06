//! P-256 account recipes on the real `EastSeaAccount` runtime: EIP-7702
//! delegation, self batches (`execute(Call[])`), added-owner relays
//! (`ownerExecute`) and ERC-1271 signatures. Off-chain signatures made here
//! are counted as typed-message signatures of an open workflow.
use crate::harness::{Harness, TX_GAS};
use aether_crypto::Signer;
use aether_execution::{account as acc, AccountCall, EvmCall, Receipt};
use aether_types::{Address, Bytes, B256, U256};
use alloy_sol_types::{sol, SolCall};

sol! {
    interface AccountViews {
        struct OwnerKey { bytes32 x; bytes32 y; }
        function owners() returns (OwnerKey[] keys, uint256 nonce);
        function signatureMessage(bytes32 hash) returns (bytes message);
    }
}

impl Harness {
    /// The P-256 public key coordinates of `actor`.
    pub fn p256_xy(&self, actor: u8) -> ([u8; 32], [u8; 32]) {
        aether_crypto::p256_xy(&self.signer(actor).public_key().bytes).unwrap()
    }

    /// `(r, s)` of `actor`'s key over `message` (ECDSA over SHA-256, low-s).
    pub fn p256_sign(&self, actor: u8, message: &[u8]) -> ([u8; 32], [u8; 32]) {
        let sig = self.signer(actor).sign(message).unwrap();
        (sig[..32].try_into().unwrap(), sig[32..64].try_into().unwrap())
    }

    /// Delegate `actor`'s P-256 account to the genesis `EastSeaAccount` (one tx).
    pub fn delegate_account(&mut self, actor: u8) -> Address {
        let account = self.addr(actor);
        self.transact(
            actor,
            EvmCall {
                to: Some(account),
                value: U256::ZERO,
                input: acc::encode_execute(&[]),
                gas_limit: TX_GAS,
                delegate: Some(aether_execution::AETHER_ACCOUNT),
            },
            "EastSeaAccount/delegate-p256-account",
            true,
        );
        let code = self.state.code(&account);
        assert_eq!(&code[..3], &[0xef, 0x01, 0x00], "delegation designator");
        assert_eq!(&code[3..], aether_execution::AETHER_ACCOUNT.as_slice());
        account
    }

    /// One owner transaction signature runs every call atomically.
    pub fn self_batch(&mut self, actor: u8, calls: &[AccountCall], label: &str, expect: bool) -> Receipt {
        let account = self.addr(actor);
        let input = acc::encode_execute(calls).to_vec();
        if expect {
            self.ok(actor, account, input, U256::ZERO, label)
        } else {
            self.revert(actor, account, input, U256::ZERO, label)
        }
    }

    /// Add `owner_actor`'s key as an owner of `account_actor`'s account through
    /// a self batch; returns the key's index.
    pub fn add_owner(&mut self, account_actor: u8, owner_actor: u8, label: &str) -> u64 {
        let account = self.addr(account_actor);
        let (x, y) = self.p256_xy(owner_actor);
        self.self_batch(account_actor, &[(account, U256::ZERO, acc::encode_add_owner(x, y))], label, true);
        let (keys, _) = self.owners(account);
        keys.iter()
            .position(|k| *k == (x, y))
            .expect("owner key stored") as u64
    }

    /// Owner keys and the owner nonce of a delegated account.
    pub fn owners(&self, account: Address) -> (Vec<([u8; 32], [u8; 32])>, u64) {
        let out = self.view(0, account, AccountViews::ownersCall {}.abi_encode());
        let r = AccountViews::ownersCall::abi_decode_returns(&out).unwrap();
        (
            r.keys.iter().map(|k| (k.x.0, k.y.0)).collect(),
            u64::try_from(r.nonce).unwrap(),
        )
    }

    /// `relayer` submits `calls` signed off-chain by added owner `owner_actor`
    /// (key `key_index`) at the account's current owner nonce. The owner's
    /// signature counts as one typed-message signature.
    #[allow(clippy::too_many_arguments)]
    pub fn owner_relay(
        &mut self,
        relayer: u8,
        account: Address,
        owner_actor: u8,
        key_index: u64,
        calls: &[AccountCall],
        label: &str,
        expect: bool,
    ) -> Receipt {
        let input = self.owner_relay_input(account, owner_actor, key_index, calls, label);
        if expect {
            self.ok(relayer, account, input, U256::ZERO, label)
        } else {
            self.revert(relayer, account, input, U256::ZERO, label)
        }
    }

    /// `ownerExecute` calldata signed by `owner_actor` at the current owner
    /// nonce (one typed-message signature). Reusable for replay tests.
    pub fn owner_relay_input(&mut self, account: Address, owner_actor: u8, key_index: u64, calls: &[AccountCall], label: &str) -> Vec<u8> {
        let (_, nonce) = self.owners(account);
        let (r, s) = self.p256_sign(owner_actor, &acc::owner_message(self.chain_id(), account, nonce, calls));
        self.typed_signature(owner_actor, &format!("{label}/owner-signature"));
        acc::encode_owner_execute(calls, key_index, r, s).to_vec()
    }

    /// The ERC-1271 blob `r ‖ s ‖ x ‖ y` by `signer_actor` for `hash` on
    /// `account`, over the account's own EIP-712 `signatureMessage(hash)`.
    /// Counts as one typed-message signature.
    pub fn account_signature(&mut self, signer_actor: u8, account: Address, hash: B256, label: &str) -> Bytes {
        let message = AccountViews::signatureMessageCall::abi_decode_returns(&self.view(
            0,
            account,
            AccountViews::signatureMessageCall { hash }.abi_encode(),
        ))
        .unwrap();
        let (r, s) = self.p256_sign(signer_actor, &message);
        let (x, y) = self.p256_xy(signer_actor);
        self.typed_signature(signer_actor, label);
        Bytes::from([r, s, x, y].concat())
    }
}
