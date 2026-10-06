//! A P-256 account delegated (EIP-7702) to the new-genesis EastSeaAccount
//! predeploy works with the token and signature standards dapps assume:
//! ERC-721 `safeTransferFrom` reaches it through its receiver hook, and
//! OpenZeppelin's `SignatureChecker` (Permit2-style ERC-1271) accepts its
//! owner key's signature — and nothing else.
use super::harness::*;
use aether_crypto::Signer;
use aether_execution::EvmCall;
use alloy_primitives::keccak256;
use alloy_sol_types::{sol, SolCall, SolValue};

sol! {
    interface ReceiverNft {
        function mint(address to) returns (uint256 id);
        function safeTransferFrom(address from, address to, uint256 tokenId);
        function ownerOf(uint256 tokenId) returns (address owner);
    }
    interface Account1271 {
        function signatureMessage(bytes32 hash) returns (bytes message);
        function isValidSignature(bytes32 hash, bytes signature) returns (bytes4 magic);
    }
    interface Checker {
        function isValidSignatureNow(address signer, bytes32 hash, bytes signature) returns (bool ok);
    }
}

/// Install the actual v2 predeploy delegation on `actor`'s P-256 account.
fn delegate(h: &mut Harness, actor: u8) -> Address {
    let account = h.addr(actor);
    h.transact(
        actor,
        EvmCall {
            to: Some(account),
            value: U256::ZERO,
            input: aether_execution::encode_execute(&[]),
            gas_limit: TX_GAS,
            delegate: Some(aether_execution::AETHER_ACCOUNT),
        },
        "EastSeaAccount/delegate-p256-account",
        true,
    );
    let code = h.state.code(&account);
    assert_eq!(&code[..3], &[0xef, 0x01, 0x00]);
    assert_eq!(&code[3..], aether_execution::AETHER_ACCOUNT.as_slice());
    account
}

/// The EIP-712 message an owner key signs for `hash` on `account`, built
/// independently of the contract.
fn eip712_message(chain: u64, account: Address, hash: B256) -> Vec<u8> {
    let domain = keccak256(
        (
            keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
            keccak256("EastSeaAccount"),
            keccak256("2"),
            U256::from(chain),
            account,
        )
            .abi_encode(),
    );
    let contents = keccak256((keccak256("Contents(bytes32 contents)"), hash).abi_encode());
    [&[0x19u8, 0x01][..], domain.as_slice(), contents.as_slice()].concat()
}

/// r ‖ s ‖ x ‖ y: `actor`'s key signing `message` the Secure Enclave way
/// (ECDSA over SHA-256 of the message, low-s).
fn sign(h: &Harness, actor: u8, message: &[u8]) -> Vec<u8> {
    let signer = h.signer(actor);
    let (x, y) = aether_crypto::p256_xy(&signer.public_key().bytes).unwrap();
    [signer.sign(message).unwrap(), x.to_vec(), y.to_vec()].concat()
}

fn checker_says(h: &Harness, checker: Address, signer: Address, hash: B256, signature: &[u8]) -> bool {
    let input = Checker::isValidSignatureNowCall { signer, hash, signature: signature.to_vec().into() }.abi_encode();
    Checker::isValidSignatureNowCall::abi_decode_returns(&h.view(2, checker, input)).unwrap()
}

#[test]
fn delegated_p256_account_receives_an_nft_by_safe_transfer_from() {
    let mut h = Harness::new();
    let account = delegate(&mut h, 0);
    let nft = h.deploy("support/MarketNFT", vec![]);
    let holder = h.addr(1);
    h.ok(1, nft, ReceiverNft::mintCall { to: holder }.abi_encode(), U256::ZERO, "MarketNFT/mint");
    h.ok(
        1,
        nft,
        ReceiverNft::safeTransferFromCall { from: holder, to: account, tokenId: U256::from(1u8) }.abi_encode(),
        U256::ZERO,
        "MarketNFT/safeTransferFrom-to-delegated-p256-account",
    );
    let owner = ReceiverNft::ownerOfCall::abi_decode_returns(&h.view(
        0,
        nft,
        ReceiverNft::ownerOfCall { tokenId: U256::from(1u8) }.abi_encode(),
    ))
    .unwrap();
    assert_eq!(owner, account, "the delegated account holds the NFT");
}

#[test]
fn signature_checker_accepts_the_delegated_accounts_owner_key_signature_only() {
    let mut h = Harness::new();
    let account = delegate(&mut h, 0);
    let other = delegate(&mut h, 1);
    let checker = h.deploy("support/SignatureCheckerProbe", vec![]);
    let hash = keccak256("Permit2 PermitTransferFrom digest");
    let chain = h.chain_id();

    // The contract's message is the EIP-712 encoding built here, and the
    // address it recognises the key by is `aether_crypto::address_of`.
    let message = eip712_message(chain, account, hash);
    let on_chain = Account1271::signatureMessageCall::abi_decode_returns(&h.view(
        2,
        account,
        Account1271::signatureMessageCall { hash }.abi_encode(),
    ))
    .unwrap();
    assert_eq!(on_chain.as_ref(), message.as_slice());

    let good = sign(&h, 0, &message);
    assert!(checker_says(&h, checker, account, hash, &good), "the account's own key, through ERC-1271");
    let magic = Account1271::isValidSignatureCall::abi_decode_returns(&h.view(
        2,
        account,
        Account1271::isValidSignatureCall { hash, signature: good.clone().into() }.abi_encode(),
    ))
    .unwrap();
    assert_eq!(magic.0, [0x16, 0x26, 0xba, 0x7e]);

    // Another hash, another key, another account, another chain: all refused.
    assert!(!checker_says(&h, checker, account, keccak256("another permit"), &good));
    assert!(!checker_says(&h, checker, account, hash, &sign(&h, 1, &message)), "a key that does not own the account");
    assert!(!checker_says(&h, checker, other, hash, &good), "replayed on another account");
    assert!(
        !checker_says(&h, checker, other, hash, &sign(&h, 0, &eip712_message(chain, other, hash))),
        "the key signing for an account it does not own"
    );
    assert!(
        !checker_says(&h, checker, account, hash, &sign(&h, 0, &eip712_message(chain + 1, account, hash))),
        "signed for another chain"
    );
    assert!(!checker_says(&h, checker, account, hash, &sign(&h, 0, hash.as_slice())), "the bare hash");
}
