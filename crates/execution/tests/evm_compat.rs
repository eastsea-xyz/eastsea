//! Standard contract calls against the pinned mainnet runtimes and actual
//! P-256/EIP-7702 accounts. The legacy implementation and empty predeploys
//! are negative controls for the original B1-B3 probes.

use aether_crypto::{P256Signer, Signer};
use aether_execution::{
    aether_account_code, aether_account_code_v2, call, encode_execute, execute_block, predeploys,
    sign_call_with, BlockContext, CallResult, EvmCall, FeePolicy, Receipt, WorldState,
    AETHER_ACCOUNT,
};
use aether_types::{Address, Bytes, FeeVector, GasVector, B256, U256};
use alloy_primitives::{address, keccak256};
use alloy_sol_types::{sol, SolCall, SolValue};

const CHAIN: u64 = 7_781;
const GAS: u64 = 2_000_000;
const TOKEN: Address = address!("0000000000000000000000000000000000010001");
const NFT: Address = address!("0000000000000000000000000000000000010002");

sol! {
    interface Account {
        function isValidSignature(bytes32 hash, bytes signature) external view returns (bytes4);
        function supportsInterface(bytes4 interfaceId) external view returns (bool);
        function onERC1155Received(address operator, address from, uint256 id, uint256 value, bytes data) external returns (bytes4);
        function onERC1155BatchReceived(address operator, address from, uint256[] ids, uint256[] values, bytes data) external returns (bytes4);
    }
    interface Token {
        function mint(address to, uint256 amount) external;
        function balanceOf(address owner) external view returns (uint256);
        function allowance(address owner, address spender) external view returns (uint256);
        function approve(address spender, uint256 amount) external returns (bool);
        function nonces(address owner) external view returns (uint256);
        function permit(address owner, address spender, uint256 value, uint256 deadline, bytes signature) external;
    }
    interface Nft {
        function safeMint(address to, uint256 id) external;
        function ownerOf(uint256 id) external view returns (address);
        function safeTransferFrom(address from, address to, uint256 id) external;
    }
    interface Multicall {
        struct Call { address target; bytes callData; }
        function aggregate(Call[] calls) external payable returns (uint256 blockNumber, bytes[] returnData);
    }
    interface Permit2 {
        struct TokenPermissions { address token; uint256 amount; }
        struct PermitTransferFrom { TokenPermissions permitted; uint256 nonce; uint256 deadline; }
        struct SignatureTransferDetails { address to; uint256 requestedAmount; }
        function DOMAIN_SEPARATOR() external view returns (bytes32);
        function nonceBitmap(address owner, uint256 word) external view returns (uint256);
        function permitTransferFrom(PermitTransferFrom permit, SignatureTransferDetails transferDetails, address owner, bytes signature) external;
    }
}

fn runtime(hex: &str) -> Bytes {
    alloy_primitives::hex::decode(hex.trim()).unwrap().into()
}

struct Harness {
    state: WorldState,
    signers: [P256Signer; 3],
    ctx: BlockContext,
}

impl Harness {
    fn new() -> Self {
        let signers = [1u8, 2, 3].map(|n| {
            let mut seed = [0u8; 32];
            seed[0] = 0xec;
            seed[31] = n;
            P256Signer::from_seed(&seed).unwrap()
        });
        let mut state = WorldState::default();
        for signer in &signers {
            state
                .set_balance(
                    aether_crypto::address_of(&signer.public_key()).unwrap(),
                    U256::from(10u128.pow(21)),
                )
                .unwrap();
        }
        state
            .set_code(AETHER_ACCOUNT, aether_account_code_v2())
            .unwrap();
        for (address, code, hash) in predeploys::all() {
            state.set_code(address, code).unwrap();
            assert_eq!(state.code_hash(&address), hash);
        }
        // Compiled from contracts/test/fixtures/EvmCompat.sol with solc
        // 0.8.19, Paris, optimizer 200; regenerate as described in fixtures/README.md.
        state
            .set_code(
                TOKEN,
                runtime(include_str!("fixtures/evm_compat_token.bin.hex")),
            )
            .unwrap();
        state
            .set_code(
                NFT,
                runtime(include_str!("fixtures/evm_compat_nft.bin.hex")),
            )
            .unwrap();
        Self {
            state,
            signers,
            ctx: BlockContext {
                chain_id: CHAIN,
                number: 0,
                timestamp: 1_800_000_000,
                beneficiary: Address::repeat_byte(0xbe),
                limits: GasVector {
                    exec: 30_000_000,
                    state: aether_execution::fees::MAX_STATE_UNITS_PER_BLOCK,
                    prove: 200_000_000,
                },
                fees: Some(FeePolicy {
                    base: FeeVector {
                        exec: 0,
                        state: aether_execution::fees::STATE_UNIT_PRICE,
                        prove: 0,
                    },
                    proposer: Address::repeat_byte(0xbe),
                }),
            },
        }
    }

    fn addr(&self, actor: usize) -> Address {
        aether_crypto::address_of(&self.signers[actor].public_key()).unwrap()
    }

    fn execute(&mut self, actor: usize, evm_call: EvmCall, success: bool) -> Receipt {
        self.ctx.number += 1;
        let mut tx = sign_call_with(
            &self.signers[actor],
            self.ctx.chain_id,
            self.state.nonce(&self.addr(actor)),
            FeeVector {
                exec: 1,
                state: aether_execution::fees::STATE_UNIT_PRICE,
                prove: 1,
            },
            1,
            &evm_call,
        )
        .unwrap();
        tx.header.gas.state = aether_execution::recommended_state_budget(
            &evm_call,
            Some(self.state.balance(&self.addr(actor))),
            aether_execution::fees::STATE_UNIT_PRICE,
        );
        let mut signature = self.signers[actor].sign(&tx.signing_bytes()).unwrap();
        signature.extend_from_slice(&self.signers[actor].public_key().bytes);
        tx.signature = signature.into();
        let outcome = execute_block(&self.state, &self.ctx, &[tx]).unwrap();
        let receipt = outcome.receipts.into_iter().next().unwrap();
        assert_eq!(receipt.success, success, "receipt: {receipt:?}");
        self.state = outcome.state;
        receipt
    }

    fn transact(&mut self, actor: usize, to: Address, input: Vec<u8>, success: bool) -> Receipt {
        self.execute(
            actor,
            EvmCall {
                to: Some(to),
                value: U256::ZERO,
                input: input.into(),
                gas_limit: GAS,
                delegate: None,
            },
            success,
        )
    }

    fn delegate(&mut self, actor: usize) -> Address {
        let account = self.addr(actor);
        self.execute(
            actor,
            EvmCall {
                to: Some(account),
                value: U256::ZERO,
                input: encode_execute(&[]),
                gas_limit: GAS,
                delegate: Some(AETHER_ACCOUNT),
            },
            true,
        );
        assert_eq!(
            self.state.code(&account).as_ref(),
            [&[0xef, 0x01, 0x00][..], AETHER_ACCOUNT.as_slice()].concat()
        );
        account
    }

    fn view(&self, actor: usize, to: Address, input: Vec<u8>) -> CallResult {
        call(
            &self.state,
            &self.ctx,
            self.addr(actor),
            Some(to),
            input.into(),
            U256::ZERO,
            GAS,
        )
        .unwrap()
    }

    fn valid_signature(&self, owner: Address, hash: B256, signature: Bytes) -> bool {
        let result = self.view(
            2,
            owner,
            Account::isValidSignatureCall { hash, signature }.abi_encode(),
        );
        result.success
            && Account::isValidSignatureCall::abi_decode_returns(&result.output)
                .is_ok_and(|v| v.0 == [0x16, 0x26, 0xba, 0x7e])
    }

    fn balance(&self, owner: Address) -> U256 {
        let result = self.view(2, TOKEN, Token::balanceOfCall { owner }.abi_encode());
        assert!(result.success);
        Token::balanceOfCall::abi_decode_returns(&result.output).unwrap()
    }

    fn nonce_bitmap(&self, owner: Address) -> U256 {
        let result = self.view(
            2,
            predeploys::PERMIT2,
            Permit2::nonceBitmapCall {
                owner,
                word: U256::ZERO,
            }
            .abi_encode(),
        );
        assert!(result.success);
        Permit2::nonceBitmapCall::abi_decode_returns(&result.output).unwrap()
    }

    /// Rebuild the account's typed-data wrapper independently of the runtime.
    /// The Signer hashes this 66-byte message with SHA-256, like the SE API.
    fn signature(&self, actor: usize, owner: Address, hash: B256) -> Bytes {
        let domain = keccak256((
            keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
            keccak256("EastSeaAccount"),
            keccak256("2"),
            U256::from(self.ctx.chain_id),
            owner,
        ).abi_encode());
        let contents = keccak256((keccak256("Contents(bytes32 contents)"), hash).abi_encode());
        let message = [&[0x19, 0x01][..], domain.as_slice(), contents.as_slice()].concat();
        let (x, y) = aether_crypto::p256_xy(&self.signers[actor].public_key().bytes).unwrap();
        [
            self.signers[actor].sign(&message).unwrap(),
            x.to_vec(),
            y.to_vec(),
        ]
        .concat()
        .into()
    }
}

fn permit2_domain(chain: u64) -> B256 {
    keccak256(
        (
            keccak256("EIP712Domain(string name,uint256 chainId,address verifyingContract)"),
            keccak256("Permit2"),
            U256::from(chain),
            predeploys::PERMIT2,
        )
            .abi_encode(),
    )
}

fn permit2_hash(chain: u64, spender: Address, permit: &Permit2::PermitTransferFrom) -> B256 {
    let token = keccak256(
        (
            keccak256("TokenPermissions(address token,uint256 amount)"),
            permit.permitted.token,
            permit.permitted.amount,
        )
            .abi_encode(),
    );
    let contents = keccak256((
        keccak256("PermitTransferFrom(TokenPermissions permitted,address spender,uint256 nonce,uint256 deadline)TokenPermissions(address token,uint256 amount)"),
        token,
        spender,
        permit.nonce,
        permit.deadline,
    ).abi_encode());
    keccak256(
        [
            &[0x19, 0x01][..],
            permit2_domain(chain).as_slice(),
            contents.as_slice(),
        ]
        .concat(),
    )
}

#[test]
fn p256_erc1271_accepts_the_owner_and_rejects_wrong_signatures_with_legacy_control() {
    let mut h = Harness::new();
    let owner = h.delegate(0);
    let hash = keccak256("ordinary contract signature");
    let good = h.signature(0, owner, hash);
    assert!(h.valid_signature(owner, hash, good.clone()));
    assert!(!h.valid_signature(owner, keccak256("wrong hash"), good.clone()));
    assert!(!h.valid_signature(owner, hash, h.signature(1, owner, hash)));
    assert!(!h.valid_signature(owner, hash, Bytes::new()));
    h.state
        .set_code(AETHER_ACCOUNT, aether_account_code())
        .unwrap();
    assert!(
        !h.valid_signature(owner, hash, good),
        "B1: the old implementation cannot verify ERC-1271"
    );
}

#[test]
fn safe_mint_and_transfer_reach_delegated_accounts_with_legacy_control() {
    let mut h = Harness::new();
    let owner = h.delegate(0);
    let recipient = h.delegate(1);
    h.transact(
        2,
        NFT,
        Nft::safeMintCall {
            to: owner,
            id: U256::from(7),
        }
        .abi_encode(),
        true,
    );
    h.transact(
        0,
        NFT,
        Nft::safeTransferFromCall {
            from: owner,
            to: recipient,
            id: U256::from(7),
        }
        .abi_encode(),
        true,
    );
    let result = h.view(2, NFT, Nft::ownerOfCall { id: U256::from(7) }.abi_encode());
    assert_eq!(
        Nft::ownerOfCall::abi_decode_returns(&result.output).unwrap(),
        recipient
    );
    h.state
        .set_code(AETHER_ACCOUNT, aether_account_code())
        .unwrap();
    h.transact(
        2,
        NFT,
        Nft::safeMintCall {
            to: owner,
            id: U256::from(8),
        }
        .abi_encode(),
        false,
    );
    let result = h.view(2, NFT, Nft::ownerOfCall { id: U256::from(8) }.abi_encode());
    assert_eq!(
        Nft::ownerOfCall::abi_decode_returns(&result.output).unwrap(),
        Address::ZERO,
        "reverted mint rolls back ownership"
    );
    h.transact(
        1,
        NFT,
        Nft::safeTransferFromCall {
            from: recipient,
            to: owner,
            id: U256::from(7),
        }
        .abi_encode(),
        false,
    );
}

#[test]
fn delegated_accounts_advertise_and_answer_token_receiver_interfaces() {
    let mut h = Harness::new();
    let owner = h.delegate(0);
    for (id, expected) in [
        (0x01ffc9a7u32, true),
        (0x1626ba7e, true),
        (0x150b7a02, true),
        (0x4e2312e0, true),
        (0xffffffff, false),
        (0, false),
    ] {
        let result = h.view(
            2,
            owner,
            Account::supportsInterfaceCall {
                interfaceId: id.to_be_bytes().into(),
            }
            .abi_encode(),
        );
        assert!(result.success);
        assert_eq!(
            Account::supportsInterfaceCall::abi_decode_returns(&result.output).unwrap(),
            expected
        );
    }
    let result = h.view(
        2,
        owner,
        Account::onERC1155ReceivedCall {
            operator: h.addr(2),
            from: h.addr(1),
            id: U256::from(1),
            value: U256::from(10),
            data: Bytes::new(),
        }
        .abi_encode(),
    );
    assert!(result.success);
    assert_eq!(
        Account::onERC1155ReceivedCall::abi_decode_returns(&result.output)
            .unwrap()
            .0,
        [0xf2, 0x3a, 0x6e, 0x61]
    );
    let result = h.view(
        2,
        owner,
        Account::onERC1155BatchReceivedCall {
            operator: h.addr(2),
            from: h.addr(1),
            ids: vec![U256::from(1), U256::from(2)],
            values: vec![U256::from(10), U256::from(20)],
            data: Bytes::new(),
        }
        .abi_encode(),
    );
    assert!(result.success);
    assert_eq!(
        Account::onERC1155BatchReceivedCall::abi_decode_returns(&result.output)
            .unwrap()
            .0,
        [0xbc, 0x19, 0x7c, 0x81]
    );
}

#[test]
fn multicall3_aggregate_executes_the_canonical_runtime_with_missing_code_control() {
    let mut h = Harness::new();
    let owner = h.delegate(0);
    h.transact(
        2,
        TOKEN,
        Token::mintCall {
            to: owner,
            amount: U256::from(123),
        }
        .abi_encode(),
        true,
    );
    let input = Multicall::aggregateCall {
        calls: vec![
            Multicall::Call {
                target: TOKEN,
                callData: Token::balanceOfCall { owner }.abi_encode().into(),
            },
            Multicall::Call {
                target: owner,
                callData: Account::supportsInterfaceCall {
                    interfaceId: [0x16, 0x26, 0xba, 0x7e].into(),
                }
                .abi_encode()
                .into(),
            },
        ],
    }
    .abi_encode();
    let result = h.view(2, predeploys::MULTICALL3, input.clone());
    assert!(result.success);
    let result = Multicall::aggregateCall::abi_decode_returns(&result.output).unwrap();
    assert_eq!(result.blockNumber, U256::from(h.ctx.number));
    assert_eq!(result.returnData.len(), 2);
    assert_eq!(
        Token::balanceOfCall::abi_decode_returns(&result.returnData[0]).unwrap(),
        U256::from(123)
    );
    assert!(Account::supportsInterfaceCall::abi_decode_returns(&result.returnData[1]).unwrap());
    h.state
        .set_code(predeploys::MULTICALL3, Bytes::new())
        .unwrap();
    assert!(
        Multicall::aggregateCall::abi_decode_returns(
            &h.view(2, predeploys::MULTICALL3, input).output
        )
        .is_err(),
        "B3: no code gives no aggregate result"
    );
}

#[test]
fn canonical_permit2_uses_the_local_chain_domain_and_verifies_p256_transfers() {
    let mut h = Harness::new();
    let owner = h.delegate(0);
    let spender = h.addr(1);
    let recipient = h.addr(2);
    let domain = h.view(
        2,
        predeploys::PERMIT2,
        Permit2::DOMAIN_SEPARATORCall {}.abi_encode(),
    );
    assert_eq!(
        Permit2::DOMAIN_SEPARATORCall::abi_decode_returns(&domain.output).unwrap(),
        permit2_domain(CHAIN)
    );
    assert_ne!(
        permit2_domain(CHAIN),
        permit2_domain(1),
        "mainnet immutables must not fix the EastSea domain to chain 1"
    );
    h.transact(
        2,
        TOKEN,
        Token::mintCall {
            to: owner,
            amount: U256::from(100),
        }
        .abi_encode(),
        true,
    );
    h.transact(
        0,
        TOKEN,
        Token::approveCall {
            spender: predeploys::PERMIT2,
            amount: U256::MAX,
        }
        .abi_encode(),
        true,
    );
    let permit = Permit2::PermitTransferFrom {
        permitted: Permit2::TokenPermissions {
            token: TOKEN,
            amount: U256::from(40),
        },
        nonce: U256::from(9),
        deadline: U256::from(h.ctx.timestamp + 3_600),
    };
    let hash = permit2_hash(CHAIN, spender, &permit);
    let good = h.signature(0, owner, hash);
    let calldata = |signature| {
        Permit2::permitTransferFromCall {
            permit: permit.clone(),
            transferDetails: Permit2::SignatureTransferDetails {
                to: recipient,
                requestedAmount: U256::from(30),
            },
            owner,
            signature,
        }
        .abi_encode()
    };
    // Neither the original B1 implementation nor the original B3 empty
    // address can complete this same transfer. No production artifacts change.
    h.state
        .set_code(AETHER_ACCOUNT, aether_account_code())
        .unwrap();
    h.transact(1, predeploys::PERMIT2, calldata(good.clone()), false);
    h.state
        .set_code(AETHER_ACCOUNT, aether_account_code_v2())
        .unwrap();
    h.state.set_code(predeploys::PERMIT2, Bytes::new()).unwrap();
    h.transact(1, predeploys::PERMIT2, calldata(good.clone()), true); // empty-code call is an EVM success
    assert_eq!(h.balance(owner), U256::from(100));
    assert_eq!(
        h.balance(recipient),
        U256::ZERO,
        "B3: absent Permit2 cannot transfer tokens"
    );
    h.state
        .set_code(predeploys::PERMIT2, predeploys::permit2_code())
        .unwrap();
    h.transact(
        1,
        predeploys::PERMIT2,
        calldata(h.signature(2, owner, hash)),
        false,
    );
    h.transact(
        1,
        predeploys::PERMIT2,
        calldata(h.signature(0, owner, keccak256("wrong permit"))),
        false,
    );
    h.transact(2, predeploys::PERMIT2, calldata(good.clone()), false); // spender is part of the signed hash
    h.transact(
        1,
        predeploys::PERMIT2,
        calldata(h.signature(0, owner, permit2_hash(1, spender, &permit))),
        false,
    );
    assert_eq!(
        h.nonce_bitmap(owner),
        U256::ZERO,
        "failed signatures do not consume the unordered nonce"
    );
    h.transact(1, predeploys::PERMIT2, calldata(good.clone()), true);
    assert_eq!(h.balance(owner), U256::from(70));
    assert_eq!(h.balance(recipient), U256::from(30));
    assert_eq!(h.nonce_bitmap(owner), U256::from(1) << 9);
    h.transact(1, predeploys::PERMIT2, calldata(good), false);
    assert_eq!(h.balance(recipient), U256::from(30), "replay moves nothing");
}

#[test]
fn erc2612_style_caller_with_erc1271_fallback_accepts_the_account_signature() {
    let mut h = Harness::new();
    let owner = h.delegate(0);
    let spender = h.addr(1);
    let value = U256::from(50);
    let deadline = U256::from(h.ctx.timestamp + 3_600);
    let domain = keccak256((
        keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
        keccak256("EvmCompatToken"), keccak256("1"), U256::from(CHAIN), TOKEN,
    ).abi_encode());
    let contents = keccak256((
        keccak256("Permit(address owner,address spender,uint256 value,uint256 nonce,uint256 deadline)"),
        owner, spender, value, U256::ZERO, deadline,
    ).abi_encode());
    let hash = keccak256([&[0x19, 0x01][..], domain.as_slice(), contents.as_slice()].concat());
    let calldata = |signature| {
        Token::permitCall {
            owner,
            spender,
            value,
            deadline,
            signature,
        }
        .abi_encode()
    };
    let good = h.signature(0, owner, hash);
    h.state
        .set_code(AETHER_ACCOUNT, aether_account_code())
        .unwrap();
    h.transact(2, TOKEN, calldata(good.clone()), false);
    h.state
        .set_code(AETHER_ACCOUNT, aether_account_code_v2())
        .unwrap();
    h.transact(2, TOKEN, calldata(h.signature(1, owner, hash)), false);
    h.transact(2, TOKEN, calldata(good.clone()), true);
    let result = h.view(
        2,
        TOKEN,
        Token::allowanceCall { owner, spender }.abi_encode(),
    );
    assert_eq!(
        Token::allowanceCall::abi_decode_returns(&result.output).unwrap(),
        value
    );
    let result = h.view(2, TOKEN, Token::noncesCall { owner }.abi_encode());
    assert_eq!(
        Token::noncesCall::abi_decode_returns(&result.output).unwrap(),
        U256::from(1)
    );
    h.transact(2, TOKEN, calldata(good), false);
}
