//! Names, tokens, NFT standards, and proof/name-gated native distributions.
use super::harness::*;
use aether_crypto::{Secp256k1Signer, Signer};
use alloy_primitives::keccak256;
use alloy_sol_types::{sol, SolCall, SolValue};

sol! {
    interface Names {
        function commit(bytes32 commitment);
        function register(string name, address owner, bytes32 salt, address relayer);
        function clear(bytes32 commitment);
        function renew(string name);
        function transferPropose(string name, address to);
        function transferAccept(string name);
        function setAddr(string name, address a);
        function setText(string name, string key, string value);
        function setReverse(string name);
        function reverseOf(address account) returns (string name);
        function textOf(bytes32 node, string key) returns (string value);
        function ownerOf(bytes32 node) returns (address owner);
        function expiresOf(bytes32 node) returns (uint64 expires);
    }
    interface Token {
        function transfer(address to, uint256 value) returns (bool);
        function approve(address spender, uint256 value) returns (bool);
        function transferFrom(address from, address to, uint256 value) returns (bool);
        function balanceOf(address account) returns (uint256 balance);
        function allowance(address owner, address spender) returns (uint256 amount);
        function permit(address owner, address spender, uint256 value, uint256 deadline, uint8 v, bytes32 r, bytes32 s);
        function nonces(address owner) returns (uint256 nonce);
        function DOMAIN_SEPARATOR() returns (bytes32 domain);
    }
    interface Nft {
        function mint(address to, uint8 color, uint8 shape, uint8 pattern, uint8 halo);
        function burn(uint256 tokenId);
        function approve(address to, uint256 tokenId);
        function setApprovalForAll(address operator, bool approved);
        function transferFrom(address from, address to, uint256 tokenId);
        function safeTransferFrom(address from, address to, uint256 tokenId);
        function safeTransferFrom(address from, address to, uint256 tokenId, bytes data);
        function ownerOf(uint256 tokenId) returns (address owner);
        function engageBrake(uint8 state);
    }
    interface Editions {
        function createEdition(string name, uint256 cap, uint256 maxPerWallet, uint256 price, uint16 feeBps);
        function mint(uint256 editionId);
        function withdraw(uint256 editionId);
        function setApprovalForAll(address operator, bool approved);
        function safeTransferFrom(address from, address to, uint256 id, uint256 value, bytes data);
        function safeBatchTransferFrom(address from, address to, uint256[] ids, uint256[] values, bytes data);
        function balanceOf(address account, uint256 id) returns (uint256 balance);
        function engageBrake(uint8 state);
    }
    interface Airdrop {
        function claim(uint256 amount, bytes32[] proof);
        function sweep();
        function claimed(address account) returns (bool value);
        function totalClaimed() returns (uint256 amount);
        function deadline() returns (uint256 value);
    }
    interface NameDrop {
        function deadline() returns (uint256 value);
        function claim();
        function sweep();
        function claimed(address account) returns (bool value);
    }
    interface Callback {
        function execute(address target_, bytes data_, uint256 value) returns (bool success, bytes result);
        function executeOrRevert(address target_, bytes data_, uint256 value) returns (bytes result);
        function configure(address target_, bytes data_);
        function setRejectPayment(bool reject);
        function callbackAttempted() returns (bool value);
        function innerSuccess() returns (bool value);
        function innerOutput() returns (bytes value);
    }
}

macro_rules! ok {
    ($h:ident, $actor:expr, $to:expr, $call:expr, $label:expr) => {
        $h.ok($actor, $to, $call.abi_encode(), U256::ZERO, $label)
    };
}
macro_rules! no {
    ($h:ident, $actor:expr, $to:expr, $call:expr, $label:expr) => {
        $h.revert($actor, $to, $call.abi_encode(), U256::ZERO, $label)
    };
}
fn uint(n: u64) -> U256 {
    U256::from(n)
}
fn commitment(name: &str, owner: Address, salt: B256, relayer: Address) -> B256 {
    let mut bytes = name.as_bytes().to_vec();
    bytes.extend_from_slice(owner.as_slice());
    bytes.extend_from_slice(salt.as_slice());
    bytes.extend_from_slice(relayer.as_slice());
    keccak256(bytes)
}
fn register(h: &mut Harness, names: Address, actor: u8, name: &str, salt: B256) {
    let owner = h.addr(actor);
    let hash = commitment(name, owner, salt, Address::ZERO);
    h.ok(
        actor,
        names,
        Names::commitCall { commitment: hash }.abi_encode(),
        U256::from(10_000_000_000_000_000u64),
        "commit funded bond",
    );
    h.advance(60);
    let price = match name.len() {
        3 => 2_000_000_000_000_000_000u64,
        4 => 500_000_000_000_000_000,
        _ => 100_000_000_000_000_000,
    };
    h.ok(
        actor,
        names,
        Names::registerCall {
            name: name.into(),
            owner,
            salt,
            relayer: Address::ZERO,
        }
        .abi_encode(),
        U256::from(price - 10_000_000_000_000_000),
        "register aged commitment exact fee",
    );
}

#[test]
fn core_and_toolbox_names_commit_resolve_transfer_renew_and_release() {
    for artifact in ["core/EastSeaNames", "toolbox/EastSeaNames"] {
        let mut h = Harness::new();
        let names = h.deploy(artifact, vec![]);
        let owner = h.addr(0);
        let other = h.addr(1);
        let salt = B256::repeat_byte(1);
        let bond = U256::from(10_000_000_000_000_000u64);
        let due = U256::from(90_000_000_000_000_000u64);
        let name = "island".to_string();
        let hash = commitment(&name, owner, salt, Address::ZERO);
        for invalid in [
            "",
            "ab",
            "-abc",
            "abc-",
            "xn--abc",
            "UPPER",
            "가나다",
            &"a".repeat(33),
        ] {
            h.revert(
                0,
                names,
                Names::registerCall {
                    name: invalid.into(),
                    owner,
                    salt,
                    relayer: Address::ZERO,
                }
                .abi_encode(),
                due,
                "register invalid name",
            );
        }
        no!(
            h,
            0,
            names,
            Names::registerCall {
                name: name.clone(),
                owner: Address::ZERO,
                salt,
                relayer: Address::ZERO
            },
            "register zero owner"
        );
        no!(
            h,
            0,
            names,
            Names::registerCall {
                name: name.clone(),
                owner,
                salt,
                relayer: Address::ZERO
            },
            "register unknown commitment"
        );
        no!(
            h,
            0,
            names,
            Names::commitCall { commitment: hash },
            "commit wrong zero bond"
        );
        h.revert(
            0,
            names,
            Names::commitCall { commitment: hash }.abi_encode(),
            bond + uint(1),
            "commit excess bond",
        );
        h.ok(
            0,
            names,
            Names::commitCall { commitment: hash }.abi_encode(),
            bond,
            "commit exact bond",
        );
        h.revert(
            1,
            names,
            Names::commitCall { commitment: hash }.abi_encode(),
            bond,
            "commit duplicate cannot reset victim window",
        );
        h.revert(
            0,
            names,
            Names::commitCall { commitment: hash }.abi_encode(),
            bond,
            "commit duplicate self",
        );
        h.revert(
            1,
            names,
            Names::registerCall {
                name: name.clone(),
                owner,
                salt,
                relayer: Address::ZERO,
            }
            .abi_encode(),
            due,
            "register wrong revealer",
        );
        h.revert(
            0,
            names,
            Names::registerCall {
                name: name.clone(),
                owner,
                salt,
                relayer: Address::ZERO,
            }
            .abi_encode(),
            due,
            "register too early",
        );
        no!(
            h,
            1,
            names,
            Names::clearCall { commitment: hash },
            "clear active commitment"
        );
        h.advance(60);
        h.revert(
            0,
            names,
            Names::registerCall {
                name: name.clone(),
                owner,
                salt,
                relayer: Address::ZERO,
            }
            .abi_encode(),
            due - uint(1),
            "register insufficient fee",
        );
        h.ok(
            0,
            names,
            Names::registerCall {
                name: name.clone(),
                owner,
                salt,
                relayer: Address::ZERO,
            }
            .abi_encode(),
            due + uint(1),
            "register overpayment refunded",
        );
        h.revert(
            0,
            names,
            Names::registerCall {
                name: name.clone(),
                owner,
                salt,
                relayer: Address::ZERO,
            }
            .abi_encode(),
            due,
            "register spent commitment",
        );
        let node = keccak256(name.as_bytes());
        assert_eq!(
            Names::ownerOfCall::abi_decode_returns(&h.view(
                0,
                names,
                Names::ownerOfCall { node }.abi_encode()
            ))
            .unwrap(),
            owner
        );
        no!(
            h,
            1,
            names,
            Names::setAddrCall {
                name: name.clone(),
                a: other
            },
            "setAddr wrong owner"
        );
        no!(
            h,
            0,
            names,
            Names::setAddrCall {
                name: "unknown".into(),
                a: owner
            },
            "setAddr unregistered"
        );
        no!(
            h,
            0,
            names,
            Names::setReverseCall { name: name.clone() },
            "setReverse forward mismatch"
        );
        ok!(
            h,
            0,
            names,
            Names::setAddrCall {
                name: name.clone(),
                a: owner
            },
            "setAddr owner"
        );
        ok!(
            h,
            0,
            names,
            Names::setReverseCall { name: name.clone() },
            "setReverse primary"
        );
        assert_eq!(
            Names::reverseOfCall::abi_decode_returns(&h.view(
                0,
                names,
                Names::reverseOfCall { account: owner }.abi_encode()
            ))
            .unwrap(),
            name
        );
        no!(
            h,
            1,
            names,
            Names::setReverseCall { name: name.clone() },
            "setReverse wrong owner"
        );
        no!(
            h,
            1,
            names,
            Names::setTextCall {
                name: name.clone(),
                key: "url".into(),
                value: "x".into()
            },
            "setText wrong owner"
        );
        for key in ["", "UPPER", "space key", &"a".repeat(33)] {
            no!(
                h,
                0,
                names,
                Names::setTextCall {
                    name: name.clone(),
                    key: key.into(),
                    value: "x".into()
                },
                "setText invalid key"
            );
        }
        no!(
            h,
            0,
            names,
            Names::setTextCall {
                name: name.clone(),
                key: "url".into(),
                value: "x".repeat(129)
            },
            "setText oversized value"
        );
        for key in ["url", "email", "github", "twitter"] {
            ok!(
                h,
                0,
                names,
                Names::setTextCall {
                    name: name.clone(),
                    key: key.into(),
                    value: "x".repeat(128)
                },
                "setText maximum value"
            );
        }
        no!(
            h,
            0,
            names,
            Names::setTextCall {
                name: name.clone(),
                key: "fifth".into(),
                value: "x".into()
            },
            "setText fifth record"
        );
        ok!(
            h,
            0,
            names,
            Names::setTextCall {
                name: name.clone(),
                key: "url".into(),
                value: "updated".into()
            },
            "setText update existing at cap"
        );
        ok!(
            h,
            0,
            names,
            Names::setTextCall {
                name: name.clone(),
                key: "url".into(),
                value: "".into()
            },
            "setText delete existing"
        );
        ok!(
            h,
            0,
            names,
            Names::setTextCall {
                name: name.clone(),
                key: "absent".into(),
                value: "".into()
            },
            "setText delete absent"
        );
        ok!(
            h,
            0,
            names,
            Names::setTextCall {
                name: name.clone(),
                key: "fifth".into(),
                value: "new".into()
            },
            "setText freed slot reusable"
        );
        no!(
            h,
            1,
            names,
            Names::transferProposeCall {
                name: name.clone(),
                to: other
            },
            "transferPropose wrong owner"
        );
        no!(
            h,
            1,
            names,
            Names::transferAcceptCall { name: name.clone() },
            "transferAccept no proposal"
        );
        ok!(
            h,
            0,
            names,
            Names::transferProposeCall {
                name: name.clone(),
                to: other
            },
            "transferPropose"
        );
        ok!(
            h,
            0,
            names,
            Names::transferProposeCall {
                name: name.clone(),
                to: Address::ZERO
            },
            "transferPropose cancel"
        );
        no!(
            h,
            1,
            names,
            Names::transferAcceptCall { name: name.clone() },
            "transferAccept cancelled"
        );
        ok!(
            h,
            0,
            names,
            Names::transferProposeCall {
                name: name.clone(),
                to: other
            },
            "transferPropose again"
        );
        no!(
            h,
            2,
            names,
            Names::transferAcceptCall { name: name.clone() },
            "transferAccept wrong recipient"
        );
        ok!(
            h,
            1,
            names,
            Names::transferAcceptCall { name: name.clone() },
            "transferAccept recipient"
        );
        no!(
            h,
            0,
            names,
            Names::setAddrCall {
                name: name.clone(),
                a: owner
            },
            "setAddr previous owner"
        );
        ok!(
            h,
            1,
            names,
            Names::setAddrCall {
                name: name.clone(),
                a: other
            },
            "setAddr new owner retires reverse"
        );
        assert!(Names::reverseOfCall::abi_decode_returns(&h.view(
            0,
            names,
            Names::reverseOfCall { account: owner }.abi_encode()
        ))
        .unwrap()
        .is_empty());
        ok!(
            h,
            1,
            names,
            Names::setReverseCall { name: name.clone() },
            "setReverse new owner"
        );
        no!(
            h,
            0,
            names,
            Names::renewCall {
                name: "unknown".into()
            },
            "renew unregistered"
        );
        no!(
            h,
            0,
            names,
            Names::renewCall { name: name.clone() },
            "renew insufficient fee"
        );
        h.ok(
            2,
            names,
            Names::renewCall { name: name.clone() }.abi_encode(),
            U256::from(100_000_000_000_000_001u64),
            "renew gift plus refund",
        );
        let expiry = Names::expiresOfCall::abi_decode_returns(&h.view(
            0,
            names,
            Names::expiresOfCall { node }.abi_encode(),
        ))
        .unwrap();
        h.at(expiry + 30 * 86400);
        h.revert(
            1,
            names,
            Names::renewCall { name: name.clone() }.abi_encode(),
            U256::from(100_000_000_000_000_000u64),
            "renew released",
        );
        no!(
            h,
            1,
            names,
            Names::setAddrCall {
                name: name.clone(),
                a: other
            },
            "setAddr released"
        );
        no!(
            h,
            1,
            names,
            Names::transferAcceptCall { name: name.clone() },
            "transferAccept released"
        );
        assert!(Names::reverseOfCall::abi_decode_returns(&h.view(
            0,
            names,
            Names::reverseOfCall { account: other }.abi_encode()
        ))
        .unwrap()
        .is_empty());
        register(&mut h, names, 2, &name, B256::repeat_byte(9));
        assert!(Names::textOfCall::abi_decode_returns(
            &h.view(
                0,
                names,
                Names::textOfCall {
                    node,
                    key: "fifth".into()
                }
                .abi_encode()
            )
        )
        .unwrap()
        .is_empty());
        let stale = commitment("stale", owner, salt, other);
        h.ok(
            0,
            names,
            Names::commitCall { commitment: stale }.abi_encode(),
            bond,
            "commit expires",
        );
        h.at(h.timestamp() + 86400);
        h.revert(
            0,
            names,
            Names::registerCall {
                name: "stale".into(),
                owner,
                salt,
                relayer: other,
            }
            .abi_encode(),
            due,
            "register expired commitment",
        );
        ok!(
            h,
            2,
            names,
            Names::clearCall { commitment: stale },
            "clear expired permissionless"
        );
        no!(
            h,
            2,
            names,
            Names::clearCall { commitment: stale },
            "clear unknown"
        );
        let relayed = commitment("relayed", owner, salt, other);
        h.ok(
            0,
            names,
            Names::commitCall {
                commitment: relayed,
            }
            .abi_encode(),
            bond,
            "commit relayer authorized",
        );
        h.advance(60);
        h.revert(
            1,
            names,
            Names::registerCall {
                name: "relayed".into(),
                owner,
                salt,
                relayer: other,
            }
            .abi_encode(),
            due,
            "register relayer gets no bond credit",
        );
        h.ok(
            1,
            names,
            Names::registerCall {
                name: "relayed".into(),
                owner,
                salt,
                relayer: other,
            }
            .abi_encode(),
            U256::from(100_000_000_000_000_000u64),
            "register relayer full fee",
        );
    }
}

#[test]
fn fixed_supply_erc20_and_real_secp256k1_permit() {
    let mut h = Harness::new();
    let owner = h.addr(0);
    let spender = h.addr(1);
    let recipient = h.addr(2);
    h.deploy_revert(
        "toolbox/FixedSupplyToken",
        ("Coin", "C", uint(100), Address::ZERO).abi_encode_params(),
        "token constructor zero recipient",
    );
    h.deploy_revert(
        "toolbox/FixedSupplyToken",
        ("Coin", "C", U256::ZERO, owner).abi_encode_params(),
        "token constructor zero supply",
    );
    let token = h.deploy(
        "toolbox/FixedSupplyToken",
        ("Coin", "C", uint(100), owner).abi_encode_params(),
    );
    no!(
        h,
        0,
        token,
        Token::transferCall {
            to: Address::ZERO,
            value: uint(1)
        },
        "transfer zero recipient"
    );
    no!(
        h,
        1,
        token,
        Token::transferCall {
            to: recipient,
            value: uint(1)
        },
        "transfer insufficient balance"
    );
    ok!(
        h,
        0,
        token,
        Token::transferCall {
            to: recipient,
            value: U256::ZERO
        },
        "transfer zero amount"
    );
    ok!(
        h,
        0,
        token,
        Token::transferCall {
            to: recipient,
            value: uint(10)
        },
        "transfer funded recipient"
    );
    no!(
        h,
        0,
        token,
        Token::approveCall {
            spender: Address::ZERO,
            value: uint(1)
        },
        "approve zero spender"
    );
    ok!(
        h,
        0,
        token,
        Token::approveCall {
            spender,
            value: uint(20)
        },
        "approve exact allowance"
    );
    no!(
        h,
        1,
        token,
        Token::transferFromCall {
            from: owner,
            to: recipient,
            value: uint(21)
        },
        "transferFrom insufficient allowance"
    );
    ok!(
        h,
        1,
        token,
        Token::transferFromCall {
            from: owner,
            to: recipient,
            value: uint(20)
        },
        "transferFrom consumes exact allowance"
    );
    no!(
        h,
        1,
        token,
        Token::transferFromCall {
            from: owner,
            to: recipient,
            value: uint(1)
        },
        "transferFrom spent allowance"
    );
    ok!(
        h,
        0,
        token,
        Token::approveCall {
            spender,
            value: U256::MAX
        },
        "approve maximum infinite"
    );
    no!(
        h,
        1,
        token,
        Token::transferFromCall {
            from: owner,
            to: recipient,
            value: U256::MAX
        },
        "transferFrom maximum insufficient balance"
    );
    ok!(
        h,
        1,
        token,
        Token::transferFromCall {
            from: owner,
            to: recipient,
            value: uint(1)
        },
        "transferFrom infinite allowance preserved"
    );
    assert_eq!(
        Token::allowanceCall::abi_decode_returns(&h.view(
            0,
            token,
            Token::allowanceCall { owner, spender }.abi_encode()
        ))
        .unwrap(),
        U256::MAX
    );
    ok!(
        h,
        0,
        token,
        Token::approveCall {
            spender,
            value: U256::ZERO
        },
        "approve revoke"
    );
    let signer = Secp256k1Signer::from_seed(&[0x43; 32]).unwrap();
    let permit_owner = aether_crypto::address_of(&signer.public_key()).unwrap();
    let domain = Token::DOMAIN_SEPARATORCall::abi_decode_returns(&h.view(
        0,
        token,
        Token::DOMAIN_SEPARATORCall {}.abi_encode(),
    ))
    .unwrap();
    let deadline = uint(h.timestamp() + 1000);
    let struct_hash = keccak256((keccak256(b"Permit(address owner,address spender,uint256 value,uint256 nonce,uint256 deadline)"), permit_owner, spender, uint(7), U256::ZERO, deadline).abi_encode_params());
    let mut message = vec![0x19, 0x01];
    message.extend_from_slice(domain.as_slice());
    message.extend_from_slice(struct_hash.as_slice());
    let signature = signer.sign(&message).unwrap();
    let call = Token::permitCall {
        owner: permit_owner,
        spender,
        value: uint(7),
        deadline,
        v: signature[64] + 27,
        r: B256::from_slice(&signature[..32]),
        s: B256::from_slice(&signature[32..64]),
    };
    ok!(
        h,
        3,
        token,
        call.clone(),
        "permit genuine secp256k1 EIP2612 relayed by P256"
    );
    assert_eq!(
        Token::noncesCall::abi_decode_returns(
            &h.view(
                0,
                token,
                Token::noncesCall {
                    owner: permit_owner
                }
                .abi_encode()
            )
        )
        .unwrap(),
        uint(1)
    );
    assert_eq!(
        Token::allowanceCall::abi_decode_returns(
            &h.view(
                0,
                token,
                Token::allowanceCall {
                    owner: permit_owner,
                    spender
                }
                .abi_encode()
            )
        )
        .unwrap(),
        uint(7)
    );
    no!(h, 3, token, call.clone(), "permit replay invalid nonce");
    let mut bad = call.clone();
    bad.owner = owner;
    no!(h, 3, token, bad, "permit wrong recovered owner");
    let mut bad = call.clone();
    bad.v = 0;
    no!(h, 3, token, bad, "permit malformed recovery id");
    let mut bad = call.clone();
    bad.s = B256::repeat_byte(0xff);
    no!(h, 3, token, bad, "permit high s rejected");
    let mut bad = call;
    bad.deadline = U256::ZERO;
    no!(h, 3, token, bad, "permit expired");
    assert_eq!(
        Token::balanceOfCall::abi_decode_returns(&h.view(
            0,
            token,
            Token::balanceOfCall { account: recipient }.abi_encode()
        ))
        .unwrap(),
        uint(31)
    );
}

#[test]
fn onchain_nft_mint_burn_approvals_transfers_and_brake() {
    let mut h = Harness::new();
    let owner = h.addr(0);
    let alice = h.addr(1);
    let bob = h.addr(2);
    h.deploy_revert(
        "toolbox/OnchainNFT",
        ("Art", "ART", U256::ZERO, uint(500), owner).abi_encode_params(),
        "nft constructor zero supply",
    );
    h.deploy_revert(
        "toolbox/OnchainNFT",
        ("Art", "ART", uint(3), uint(10001), owner).abi_encode_params(),
        "nft constructor royalty above denominator",
    );
    let nft = h.deploy(
        "toolbox/OnchainNFT",
        ("Art", "ART", uint(3), uint(500), owner).abi_encode_params(),
    );
    let mint = Nft::mintCall {
        to: alice,
        color: 0,
        shape: 7,
        pattern: 0,
        halo: 7,
    };
    no!(h, 1, nft, mint.clone(), "mint wrong creator");
    for index in 0..4 {
        let mut bad = mint.clone();
        match index {
            0 => bad.color = 8,
            1 => bad.shape = 8,
            2 => bad.pattern = 8,
            _ => bad.halo = 255,
        };
        no!(h, 0, nft, bad, "mint trait outside bounds");
    }
    let mut bad = mint.clone();
    bad.to = Address::ZERO;
    no!(h, 0, nft, bad, "mint zero recipient");
    let nonreceiver = h.deploy(
        "toolbox/FixedSupplyToken",
        ("NoReceiver", "NO", uint(1), owner).abi_encode_params(),
    );
    let mut bad = mint.clone();
    bad.to = nonreceiver;
    no!(h, 0, nft, bad, "mint contract missing ERC721 receiver");
    ok!(h, 0, nft, mint.clone(), "mint boundary traits");
    no!(
        h,
        2,
        nft,
        Nft::burnCall { tokenId: uint(1) },
        "burn wrong owner"
    );
    no!(
        h,
        2,
        nft,
        Nft::approveCall {
            to: bob,
            tokenId: uint(1)
        },
        "approve wrong owner"
    );
    no!(
        h,
        1,
        nft,
        Nft::approveCall {
            to: bob,
            tokenId: uint(99)
        },
        "approve nonexistent token"
    );
    no!(
        h,
        1,
        nft,
        Nft::setApprovalForAllCall {
            operator: Address::ZERO,
            approved: true
        },
        "setApprovalForAll zero operator"
    );
    ok!(
        h,
        1,
        nft,
        Nft::approveCall {
            to: bob,
            tokenId: uint(1)
        },
        "approve token operator"
    );
    no!(
        h,
        3,
        nft,
        Nft::transferFromCall {
            from: alice,
            to: bob,
            tokenId: uint(1)
        },
        "transferFrom unauthorized"
    );
    no!(
        h,
        2,
        nft,
        Nft::transferFromCall {
            from: owner,
            to: bob,
            tokenId: uint(1)
        },
        "transferFrom incorrect from"
    );
    no!(
        h,
        2,
        nft,
        Nft::transferFromCall {
            from: alice,
            to: Address::ZERO,
            tokenId: uint(1)
        },
        "transferFrom zero recipient"
    );
    ok!(
        h,
        2,
        nft,
        Nft::transferFromCall {
            from: alice,
            to: bob,
            tokenId: uint(1)
        },
        "transferFrom approved"
    );
    no!(
        h,
        1,
        nft,
        Nft::transferFromCall {
            from: bob,
            to: alice,
            tokenId: uint(1)
        },
        "transferFrom previous owner approval cleared"
    );
    ok!(
        h,
        2,
        nft,
        Nft::setApprovalForAllCall {
            operator: alice,
            approved: true
        },
        "setApprovalForAll"
    );
    ok!(
        h,
        1,
        nft,
        Nft::safeTransferFrom_0Call {
            from: bob,
            to: alice,
            tokenId: uint(1)
        },
        "safeTransferFrom three args"
    );
    no!(
        h,
        1,
        nft,
        Nft::safeTransferFrom_1Call {
            from: alice,
            to: nonreceiver,
            tokenId: uint(1),
            data: Bytes::new()
        },
        "safeTransferFrom rejected callback"
    );
    ok!(
        h,
        1,
        nft,
        Nft::safeTransferFrom_1Call {
            from: alice,
            to: bob,
            tokenId: uint(1),
            data: Bytes::from_static(b"data")
        },
        "safeTransferFrom four args"
    );
    ok!(h, 0, nft, mint.clone(), "mint second");
    ok!(h, 0, nft, mint.clone(), "mint reaches supply");
    no!(h, 0, nft, mint.clone(), "mint sold out");
    no!(
        h,
        1,
        nft,
        Nft::engageBrakeCall { state: 1 },
        "engageBrake wrong guardian"
    );
    no!(
        h,
        0,
        nft,
        Nft::engageBrakeCall { state: 0 },
        "engageBrake zero"
    );
    no!(
        h,
        0,
        nft,
        Nft::engageBrakeCall { state: 3 },
        "engageBrake invalid state"
    );
    ok!(
        h,
        0,
        nft,
        Nft::engageBrakeCall { state: 1 },
        "engageBrake entry"
    );
    no!(h, 0, nft, mint, "mint braked");
    no!(
        h,
        0,
        nft,
        Nft::engageBrakeCall { state: 1 },
        "engageBrake cannot weaken or repeat"
    );
    ok!(
        h,
        0,
        nft,
        Nft::engageBrakeCall { state: 2 },
        "engageBrake full"
    );
    ok!(
        h,
        2,
        nft,
        Nft::burnCall { tokenId: uint(1) },
        "burn exit allowed under brake"
    );
    no!(h, 2, nft, Nft::burnCall { tokenId: uint(1) }, "burn double");
    ok!(
        h,
        1,
        nft,
        Nft::transferFromCall {
            from: alice,
            to: bob,
            tokenId: uint(2)
        },
        "transfer exit allowed under brake"
    );
    ok!(
        h,
        2,
        nft,
        Nft::setApprovalForAllCall {
            operator: alice,
            approved: false
        },
        "setApprovalForAll revoke"
    );
    assert_eq!(
        Nft::ownerOfCall::abi_decode_returns(&h.view(
            0,
            nft,
            Nft::ownerOfCall { tokenId: uint(2) }.abi_encode()
        ))
        .unwrap(),
        bob
    );
}

#[test]
fn editions_sale_erc1155_callbacks_and_numeric_bounds() {
    let mut h = Harness::new();
    let owner = h.addr(0);
    let alice = h.addr(1);
    let bob = h.addr(2);
    let editions = h.deploy("toolbox/Editions1155", (owner,).abi_encode_params());
    let good = Editions::createEditionCall {
        name: "edition".into(),
        cap: uint(3),
        maxPerWallet: uint(1),
        price: uint(10),
        feeBps: 1000,
    };
    for name in ["".to_string(), "x".repeat(65)] {
        let mut bad = good.clone();
        bad.name = name;
        no!(h, 0, editions, bad, "createEdition invalid name");
    }
    for cap in [U256::ZERO, U256::from(1u64 << 48), U256::MAX] {
        let mut bad = good.clone();
        bad.cap = cap;
        no!(h, 0, editions, bad, "createEdition invalid cap");
    }
    for max in [U256::ZERO, uint(4), U256::MAX] {
        let mut bad = good.clone();
        bad.maxPerWallet = max;
        no!(h, 0, editions, bad, "createEdition invalid wallet cap");
    }
    let mut bad = good.clone();
    bad.feeBps = 1001;
    no!(h, 0, editions, bad, "createEdition excess royalty");
    let mut bad = good.clone();
    bad.cap = uint(1u64 << 32);
    bad.maxPerWallet = uint(1u64 << 32);
    no!(
        h,
        0,
        editions,
        bad,
        "createEdition wallet cap uint32 truncation regression"
    );
    let mut bad = good.clone();
    bad.price = U256::from(1u8) << 128;
    no!(
        h,
        0,
        editions,
        bad,
        "createEdition price uint128 truncation regression"
    );
    ok!(h, 0, editions, good, "createEdition sale");
    no!(
        h,
        0,
        editions,
        Editions::mintCall {
            editionId: uint(99)
        },
        "mint unknown edition"
    );
    no!(
        h,
        1,
        editions,
        Editions::mintCall { editionId: uint(1) },
        "mint missing price"
    );
    h.revert(
        1,
        editions,
        Editions::mintCall { editionId: uint(1) }.abi_encode(),
        uint(11),
        "mint excess price",
    );
    h.ok(
        1,
        editions,
        Editions::mintCall { editionId: uint(1) }.abi_encode(),
        uint(10),
        "mint exact price",
    );
    h.revert(
        1,
        editions,
        Editions::mintCall { editionId: uint(1) }.abi_encode(),
        uint(10),
        "mint wallet cap reached",
    );
    no!(
        h,
        1,
        editions,
        Editions::withdrawCall { editionId: uint(1) },
        "withdraw wrong creator"
    );
    no!(
        h,
        0,
        editions,
        Editions::withdrawCall {
            editionId: uint(99)
        },
        "withdraw unknown edition"
    );
    no!(
        h,
        1,
        editions,
        Editions::setApprovalForAllCall {
            operator: Address::ZERO,
            approved: true
        },
        "ERC1155 zero operator"
    );
    no!(
        h,
        2,
        editions,
        Editions::safeTransferFromCall {
            from: alice,
            to: bob,
            id: uint(1),
            value: uint(1),
            data: Bytes::new()
        },
        "ERC1155 unauthorized transfer"
    );
    no!(
        h,
        1,
        editions,
        Editions::safeTransferFromCall {
            from: alice,
            to: bob,
            id: uint(1),
            value: uint(2),
            data: Bytes::new()
        },
        "ERC1155 insufficient balance"
    );
    no!(
        h,
        1,
        editions,
        Editions::safeTransferFromCall {
            from: alice,
            to: Address::ZERO,
            id: uint(1),
            value: uint(1),
            data: Bytes::new()
        },
        "ERC1155 zero recipient"
    );
    ok!(
        h,
        1,
        editions,
        Editions::setApprovalForAllCall {
            operator: bob,
            approved: true
        },
        "ERC1155 approval"
    );
    ok!(
        h,
        2,
        editions,
        Editions::safeTransferFromCall {
            from: alice,
            to: bob,
            id: uint(1),
            value: uint(1),
            data: Bytes::new()
        },
        "ERC1155 approved transfer"
    );
    h.ok(
        1,
        editions,
        Editions::mintCall { editionId: uint(1) }.abi_encode(),
        uint(10),
        "mint after sending away wallet limit is balance based",
    );
    h.ok(
        3,
        editions,
        Editions::mintCall { editionId: uint(1) }.abi_encode(),
        uint(10),
        "mint total reaches cap",
    );
    h.revert(
        4,
        editions,
        Editions::mintCall { editionId: uint(1) }.abi_encode(),
        uint(10),
        "mint edition sold out",
    );
    no!(
        h,
        1,
        editions,
        Editions::safeBatchTransferFromCall {
            from: alice,
            to: bob,
            ids: vec![uint(1)],
            values: vec![],
            data: Bytes::new()
        },
        "ERC1155 mismatched arrays"
    );
    no!(
        h,
        4,
        editions,
        Editions::safeBatchTransferFromCall {
            from: alice,
            to: bob,
            ids: vec![uint(1)],
            values: vec![uint(1)],
            data: Bytes::new()
        },
        "ERC1155 batch unauthorized"
    );
    ok!(
        h,
        1,
        editions,
        Editions::safeBatchTransferFromCall {
            from: alice,
            to: bob,
            ids: vec![uint(1)],
            values: vec![uint(1)],
            data: Bytes::new()
        },
        "ERC1155 batch transfer"
    );
    let receiver = h.deploy("support/NativeCallback", vec![]);
    ok!(
        h,
        2,
        editions,
        Editions::safeBatchTransferFromCall {
            from: bob,
            to: receiver,
            ids: vec![uint(1)],
            values: vec![uint(1)],
            data: Bytes::new()
        },
        "ERC1155 accepting batch receiver"
    );
    ok!(
        h,
        2,
        editions,
        Editions::safeTransferFromCall {
            from: bob,
            to: receiver,
            id: uint(1),
            value: uint(1),
            data: Bytes::new()
        },
        "ERC1155 accepting single receiver"
    );
    ok!(
        h,
        0,
        editions,
        Editions::engageBrakeCall { state: 1 },
        "editions brake entry"
    );
    no!(
        h,
        0,
        editions,
        Editions::createEditionCall {
            name: "new".into(),
            cap: uint(1),
            maxPerWallet: uint(1),
            price: U256::ZERO,
            feeBps: 0
        },
        "createEdition braked"
    );
    h.revert(
        5,
        editions,
        Editions::mintCall { editionId: uint(1) }.abi_encode(),
        uint(10),
        "mint braked",
    );
    ok!(
        h,
        0,
        editions,
        Editions::withdrawCall { editionId: uint(1) },
        "withdraw remains open under brake"
    );
    no!(
        h,
        0,
        editions,
        Editions::withdrawCall { editionId: uint(1) },
        "withdraw double nothing"
    );
    ok!(
        h,
        1,
        editions,
        Editions::setApprovalForAllCall {
            operator: bob,
            approved: false
        },
        "ERC1155 revoke"
    );
    assert_eq!(
        Editions::balanceOfCall::abi_decode_returns(
            &h.view(
                0,
                editions,
                Editions::balanceOfCall {
                    account: receiver,
                    id: uint(1)
                }
                .abi_encode()
            )
        )
        .unwrap(),
        uint(2)
    );
}

fn leaf(account: Address, amount: U256) -> B256 {
    let mut data = account.as_slice().to_vec();
    data.extend_from_slice(&amount.to_be_bytes::<32>());
    keccak256(data)
}
#[test]
fn merkle_airdrop_proof_double_claim_timeout_and_zero_amount() {
    let mut h = Harness::new();
    let owner = h.addr(0);
    let alice = h.addr(1);
    let amount = uint(10);
    let root = leaf(alice, amount);
    h.deploy_revert(
        "toolbox/MerkleAirdrop",
        (B256::ZERO, owner, uint(100)).abi_encode_params(),
        "airdrop constructor zero root",
    );
    h.deploy_revert(
        "toolbox/MerkleAirdrop",
        (root, Address::ZERO, uint(100)).abi_encode_params(),
        "airdrop constructor zero distributor",
    );
    h.deploy_revert(
        "toolbox/MerkleAirdrop",
        (root, owner, U256::ZERO).abi_encode_params(),
        "airdrop constructor zero period",
    );
    let drop = h.deploy(
        "toolbox/MerkleAirdrop",
        (root, owner, uint(100)).abi_encode_params(),
    );
    let deadline = Airdrop::deadlineCall::abi_decode_returns(&h.view(
        0,
        drop,
        Airdrop::deadlineCall {}.abi_encode(),
    ))
    .unwrap()
    .to::<u64>();
    no!(h, 0, drop, Airdrop::sweepCall {}, "sweep too early");
    no!(
        h,
        2,
        drop,
        Airdrop::claimCall {
            amount,
            proof: vec![]
        },
        "claim wrong account proof"
    );
    no!(
        h,
        1,
        drop,
        Airdrop::claimCall {
            amount: amount + uint(1),
            proof: vec![]
        },
        "claim wrong amount proof"
    );
    no!(
        h,
        1,
        drop,
        Airdrop::claimCall {
            amount,
            proof: vec![B256::repeat_byte(9)]
        },
        "claim invalid sibling"
    );
    no!(
        h,
        1,
        drop,
        Airdrop::claimCall {
            amount,
            proof: vec![]
        },
        "claim insufficient pool"
    );
    h.ok(
        0,
        drop,
        vec![],
        uint(11),
        "airdrop receive pool sponsorship",
    );
    ok!(
        h,
        1,
        drop,
        Airdrop::claimCall {
            amount,
            proof: vec![]
        },
        "claim valid single leaf"
    );
    no!(
        h,
        1,
        drop,
        Airdrop::claimCall {
            amount,
            proof: vec![]
        },
        "claim double"
    );
    assert!(Airdrop::claimedCall::abi_decode_returns(&h.view(
        0,
        drop,
        Airdrop::claimedCall { account: alice }.abi_encode()
    ))
    .unwrap());
    assert_eq!(
        Airdrop::totalClaimedCall::abi_decode_returns(&h.view(
            0,
            drop,
            Airdrop::totalClaimedCall {}.abi_encode()
        ))
        .unwrap(),
        amount
    );
    h.at(deadline + 1);
    no!(
        h,
        2,
        drop,
        Airdrop::claimCall {
            amount,
            proof: vec![]
        },
        "claim closed"
    );
    ok!(
        h,
        3,
        drop,
        Airdrop::sweepCall {},
        "sweep permissionless fixed distributor"
    );
    ok!(h, 3, drop, Airdrop::sweepCall {}, "sweep zero remainder");
    let zero = h.deploy(
        "toolbox/MerkleAirdrop",
        (leaf(alice, U256::ZERO), owner, uint(100)).abi_encode_params(),
    );
    ok!(
        h,
        1,
        zero,
        Airdrop::claimCall {
            amount: U256::ZERO,
            proof: vec![]
        },
        "claim committed zero amount"
    );
    no!(
        h,
        1,
        zero,
        Airdrop::claimCall {
            amount: U256::ZERO,
            proof: vec![]
        },
        "claim zero still consumes entitlement"
    );
}

#[test]
fn name_gated_drop_live_names_and_distribution_timeouts() {
    let mut h = Harness::new();
    let owner = h.addr(0);
    let alice = h.addr(1);
    let names = h.deploy("toolbox/EastSeaNames", vec![]);
    h.deploy_revert(
        "toolbox/NameGatedDrop",
        (names, owner, U256::ZERO, uint(100)).abi_encode_params(),
        "name drop zero amount",
    );
    h.deploy_revert(
        "toolbox/NameGatedDrop",
        (names, Address::ZERO, uint(1), uint(100)).abi_encode_params(),
        "name drop zero distributor",
    );
    h.deploy_revert(
        "toolbox/NameGatedDrop",
        (names, owner, uint(1), U256::ZERO).abi_encode_params(),
        "name drop zero period",
    );
    register(&mut h, names, 1, "named", B256::repeat_byte(1));
    ok!(
        h,
        1,
        names,
        Names::setAddrCall {
            name: "named".into(),
            a: alice
        },
        "name drop forward address"
    );
    ok!(
        h,
        1,
        names,
        Names::setReverseCall {
            name: "named".into()
        },
        "name drop primary"
    );
    let drop = h.deploy(
        "toolbox/NameGatedDrop",
        (names, owner, uint(10), uint(100)).abi_encode_params(),
    );
    let deadline = NameDrop::deadlineCall::abi_decode_returns(&h.view(
        0,
        drop,
        NameDrop::deadlineCall {}.abi_encode(),
    ))
    .unwrap()
    .to::<u64>();
    no!(h, 0, drop, NameDrop::sweepCall {}, "name drop sweep early");
    no!(h, 2, drop, NameDrop::claimCall {}, "name drop no primary");
    no!(
        h,
        1,
        drop,
        NameDrop::claimCall {},
        "name drop insufficient pool"
    );
    h.ok(0, drop, vec![], uint(21), "name drop receive funding");
    ok!(h, 1, drop, NameDrop::claimCall {}, "name drop eligible");
    no!(h, 1, drop, NameDrop::claimCall {}, "name drop double");
    ok!(
        h,
        1,
        names,
        Names::setAddrCall {
            name: "named".into(),
            a: h.addr(2)
        },
        "name drop stale reverse removed"
    );
    no!(
        h,
        2,
        drop,
        NameDrop::claimCall {},
        "name drop forward alone no reverse"
    );
    h.at(deadline + 1);
    no!(
        h,
        3,
        drop,
        NameDrop::claimCall {},
        "name drop deadline closed"
    );
    ok!(
        h,
        3,
        drop,
        NameDrop::sweepCall {},
        "name drop sweep permissionless"
    );
    ok!(h, 3, drop, NameDrop::sweepCall {}, "name drop zero sweep");
}

fn via(target: Address, call: impl SolCall, value: U256) -> Callback::executeOrRevertCall {
    Callback::executeOrRevertCall {
        target_: target,
        data_: call.abi_encode().into(),
        value,
    }
}
fn assert_callback_guard(h: &Harness, receiver: Address) {
    assert!(Callback::callbackAttemptedCall::abi_decode_returns(&h.view(
        0,
        receiver,
        Callback::callbackAttemptedCall {}.abi_encode()
    ))
    .unwrap());
    assert!(!Callback::innerSuccessCall::abi_decode_returns(&h.view(
        0,
        receiver,
        Callback::innerSuccessCall {}.abi_encode()
    ))
    .unwrap());
}
fn assert_reentry_error(h: &Harness, receiver: Address, error: &str) {
    assert_callback_guard(h, receiver);
    let output = Callback::innerOutputCall::abi_decode_returns(&h.view(
        0,
        receiver,
        Callback::innerOutputCall {}.abi_encode(),
    ))
    .unwrap();
    assert_eq!(
        &output[..4],
        &keccak256(error.as_bytes())[..4],
        "callback must hit intended guard"
    );
}

#[test]
fn names_exact_reveal_boundaries_fee_buckets_taken_and_grace() {
    let mut h = Harness::new();
    let names = h.deploy("core/EastSeaNames", vec![]);
    let owner = h.addr(0);
    for (index, name) in ["abc".to_string(), "abcd".into(), "a".repeat(32)]
        .into_iter()
        .enumerate()
    {
        register(&mut h, names, 0, &name, B256::repeat_byte(index as u8 + 20));
    }
    let name = "boundary".to_string();
    let salt = B256::repeat_byte(30);
    let hash = commitment(&name, owner, salt, Address::ZERO);
    let bond = U256::from(10_000_000_000_000_000u64);
    let due = U256::from(90_000_000_000_000_000u64);
    let committed_at = h.timestamp();
    h.ok(
        0,
        names,
        Names::commitCall { commitment: hash }.abi_encode(),
        bond,
        "names exact minimum commitment age setup",
    );
    h.at(committed_at + 59);
    h.revert(
        0,
        names,
        Names::registerCall {
            name: name.clone(),
            owner,
            salt,
            relayer: Address::ZERO,
        }
        .abi_encode(),
        due,
        "names age 59 refused",
    );
    h.ok(
        0,
        names,
        Names::registerCall {
            name: name.clone(),
            owner,
            salt,
            relayer: Address::ZERO,
        }
        .abi_encode(),
        due,
        "names age 60 accepted",
    );
    let taken_salt = B256::repeat_byte(31);
    h.ok(
        0,
        names,
        Names::commitCall {
            commitment: commitment(&name, owner, taken_salt, Address::ZERO),
        }
        .abi_encode(),
        bond,
        "names taken commitment",
    );
    h.advance(60);
    h.revert(
        0,
        names,
        Names::registerCall {
            name: name.clone(),
            owner,
            salt: taken_salt,
            relayer: Address::ZERO,
        }
        .abi_encode(),
        due,
        "names registration refuses occupied name",
    );
    let key = "a".repeat(32);
    ok!(
        h,
        0,
        names,
        Names::setTextCall {
            name: name.clone(),
            key,
            value: "x".repeat(128)
        },
        "names exact text bounds"
    );
    let node = keccak256(name.as_bytes());
    let expiry = Names::expiresOfCall::abi_decode_returns(&h.view(
        0,
        names,
        Names::expiresOfCall { node }.abi_encode(),
    ))
    .unwrap();
    h.at(expiry + 1);
    h.ok(
        1,
        names,
        Names::renewCall { name: name.clone() }.abi_encode(),
        U256::from(100_000_000_000_000_000u64),
        "names renew during grace",
    );
    let expired_salt = B256::repeat_byte(32);
    let expired = commitment("expired", owner, expired_salt, Address::ZERO);
    let committed_at = h.timestamp();
    h.ok(
        0,
        names,
        Names::commitCall {
            commitment: expired,
        }
        .abi_encode(),
        bond,
        "names exact max commitment setup",
    );
    h.at(committed_at + 86400);
    h.revert(
        0,
        names,
        Names::registerCall {
            name: "expired".into(),
            owner,
            salt: expired_salt,
            relayer: Address::ZERO,
        }
        .abi_encode(),
        due,
        "names exact max age expired",
    );
    h.ok(
        1,
        names,
        Names::commitCall {
            commitment: expired,
        }
        .abi_encode(),
        bond,
        "names expired commitment replace permissionless",
    );
    no!(
        h,
        0,
        names,
        Names::clearCall {
            commitment: expired
        },
        "names replaced commitment fresh window"
    );
}

#[test]
fn merkle_valid_sibling_maximum_amount_and_deadline_boundary() {
    let mut h = Harness::new();
    let alice = h.addr(1);
    let bob = h.addr(2);
    let a = leaf(alice, uint(10));
    let b = leaf(bob, uint(20));
    let (left, right) = if a < b { (a, b) } else { (b, a) };
    let mut bytes = left.as_slice().to_vec();
    bytes.extend_from_slice(right.as_slice());
    let drop = h.deploy(
        "toolbox/MerkleAirdrop",
        (keccak256(bytes), h.addr(0), uint(1000)).abi_encode_params(),
    );
    let deadline = Airdrop::deadlineCall::abi_decode_returns(&h.view(
        0,
        drop,
        Airdrop::deadlineCall {}.abi_encode(),
    ))
    .unwrap()
    .to::<u64>();
    h.ok(0, drop, vec![], uint(30), "airdrop two leaf pool");
    ok!(
        h,
        1,
        drop,
        Airdrop::claimCall {
            amount: uint(10),
            proof: vec![b]
        },
        "airdrop valid nonempty sorted proof"
    );
    h.at(deadline);
    ok!(
        h,
        2,
        drop,
        Airdrop::claimCall {
            amount: uint(20),
            proof: vec![a]
        },
        "airdrop claim inclusive deadline"
    );
    ok!(
        h,
        3,
        drop,
        Airdrop::sweepCall {},
        "airdrop sweep first second after deadline"
    );
    let max = h.deploy(
        "toolbox/MerkleAirdrop",
        (leaf(alice, U256::MAX), h.addr(0), uint(1000)).abi_encode_params(),
    );
    no!(
        h,
        1,
        max,
        Airdrop::claimCall {
            amount: U256::MAX,
            proof: vec![]
        },
        "airdrop maximum allocation refuses insufficient pool without overflow"
    );
}

#[test]
fn distribution_callback_failure_rolls_back_and_reentry_cannot_double_claim() {
    let mut h = Harness::new();
    let receiver = h.deploy("support/NativeCallback", vec![]);
    let drop = h.deploy(
        "toolbox/MerkleAirdrop",
        (leaf(receiver, uint(10)), receiver, uint(1000)).abi_encode_params(),
    );
    let deadline = Airdrop::deadlineCall::abi_decode_returns(&h.view(
        0,
        drop,
        Airdrop::deadlineCall {}.abi_encode(),
    ))
    .unwrap()
    .to::<u64>();
    h.ok(0, drop, vec![], uint(21), "airdrop callback pool funded");
    ok!(
        h,
        0,
        receiver,
        Callback::setRejectPaymentCall { reject: true },
        "receiver rejects native payment"
    );
    no!(
        h,
        0,
        receiver,
        via(
            drop,
            Airdrop::claimCall {
                amount: uint(10),
                proof: vec![]
            },
            U256::ZERO
        ),
        "airdrop failed payout reverts entire world"
    );
    assert!(!Airdrop::claimedCall::abi_decode_returns(&h.view(
        0,
        drop,
        Airdrop::claimedCall { account: receiver }.abi_encode()
    ))
    .unwrap());
    ok!(
        h,
        0,
        receiver,
        Callback::setRejectPaymentCall { reject: false },
        "receiver accepts payment"
    );
    ok!(
        h,
        0,
        receiver,
        Callback::configureCall {
            target_: drop,
            data_: Airdrop::claimCall {
                amount: uint(10),
                proof: vec![]
            }
            .abi_encode()
            .into()
        },
        "airdrop reentry attempt configured"
    );
    ok!(
        h,
        0,
        receiver,
        via(
            drop,
            Airdrop::claimCall {
                amount: uint(10),
                proof: vec![]
            },
            U256::ZERO
        ),
        "airdrop payout nonReentrant"
    );
    assert_reentry_error(&h, receiver, "ReentrancyGuardReentrantCall()");
    assert_eq!(h.state.balance(&receiver), uint(10));
    h.at(deadline + 1);
    ok!(
        h,
        0,
        receiver,
        Callback::setRejectPaymentCall { reject: true },
        "airdrop sweep receiver rejects"
    );
    no!(
        h,
        3,
        drop,
        Airdrop::sweepCall {},
        "airdrop sweep failed payout rollback"
    );
    ok!(
        h,
        0,
        receiver,
        Callback::setRejectPaymentCall { reject: false },
        "airdrop sweep receiver accepts"
    );
    ok!(
        h,
        0,
        receiver,
        Callback::configureCall {
            target_: drop,
            data_: Airdrop::sweepCall {}.abi_encode().into()
        },
        "airdrop sweep reentry configured"
    );
    ok!(
        h,
        3,
        drop,
        Airdrop::sweepCall {},
        "airdrop sweep nonReentrant"
    );
    assert_reentry_error(&h, receiver, "ReentrancyGuardReentrantCall()");
    assert_eq!(h.state.balance(&receiver), uint(21));

    let names = h.deploy("toolbox/EastSeaNames", vec![]);
    let salt = B256::repeat_byte(7);
    let hash = commitment("callback", receiver, salt, Address::ZERO);
    h.ok(
        0,
        names,
        Names::commitCall { commitment: hash }.abi_encode(),
        U256::from(10_000_000_000_000_000u64),
        "name drop contract owner commitment",
    );
    h.advance(60);
    h.ok(
        0,
        names,
        Names::registerCall {
            name: "callback".into(),
            owner: receiver,
            salt,
            relayer: Address::ZERO,
        }
        .abi_encode(),
        U256::from(90_000_000_000_000_000u64),
        "name drop contract owner registration",
    );
    ok!(
        h,
        0,
        receiver,
        via(
            names,
            Names::setAddrCall {
                name: "callback".into(),
                a: receiver
            },
            U256::ZERO
        ),
        "name drop contract forward"
    );
    ok!(
        h,
        0,
        receiver,
        via(
            names,
            Names::setReverseCall {
                name: "callback".into()
            },
            U256::ZERO
        ),
        "name drop contract reverse"
    );
    let drop = h.deploy(
        "toolbox/NameGatedDrop",
        (names, receiver, uint(10), uint(1000)).abi_encode_params(),
    );
    let deadline = NameDrop::deadlineCall::abi_decode_returns(&h.view(
        0,
        drop,
        NameDrop::deadlineCall {}.abi_encode(),
    ))
    .unwrap()
    .to::<u64>();
    h.ok(0, drop, vec![], uint(21), "name drop callback pool funded");
    ok!(
        h,
        0,
        receiver,
        Callback::setRejectPaymentCall { reject: true },
        "name drop receiver rejects"
    );
    no!(
        h,
        0,
        receiver,
        via(drop, NameDrop::claimCall {}, U256::ZERO),
        "name drop failed claim rollback"
    );
    assert!(!NameDrop::claimedCall::abi_decode_returns(&h.view(
        0,
        drop,
        NameDrop::claimedCall { account: receiver }.abi_encode()
    ))
    .unwrap());
    ok!(
        h,
        0,
        receiver,
        Callback::setRejectPaymentCall { reject: false },
        "name drop receiver accepts"
    );
    ok!(
        h,
        0,
        receiver,
        Callback::configureCall {
            target_: drop,
            data_: NameDrop::claimCall {}.abi_encode().into()
        },
        "name drop claim reentry configured"
    );
    ok!(
        h,
        0,
        receiver,
        via(drop, NameDrop::claimCall {}, U256::ZERO),
        "name drop claim nonReentrant"
    );
    assert_reentry_error(&h, receiver, "ReentrancyGuardReentrantCall()");
    h.at(deadline + 1);
    ok!(
        h,
        0,
        receiver,
        Callback::setRejectPaymentCall { reject: true },
        "name drop sweep receiver rejects"
    );
    no!(
        h,
        2,
        drop,
        NameDrop::sweepCall {},
        "name drop failed sweep rollback"
    );
    ok!(
        h,
        0,
        receiver,
        Callback::setRejectPaymentCall { reject: false },
        "name drop sweep receiver accepts"
    );
    ok!(
        h,
        0,
        receiver,
        Callback::configureCall {
            target_: drop,
            data_: NameDrop::sweepCall {}.abi_encode().into()
        },
        "name drop sweep reentry configured"
    );
    ok!(
        h,
        2,
        drop,
        NameDrop::sweepCall {},
        "name drop sweep nonReentrant"
    );
    assert_reentry_error(&h, receiver, "ReentrancyGuardReentrantCall()");
}

#[test]
fn names_refund_and_editions_creator_withdraw_callback_failures() {
    let mut h = Harness::new();
    let receiver = h.deploy("support/NativeCallback", vec![]);
    let names = h.deploy("core/EastSeaNames", vec![]);
    let salt = B256::repeat_byte(8);
    let hash = commitment("refund", receiver, salt, Address::ZERO);
    let bond = U256::from(10_000_000_000_000_000u64);
    let due = U256::from(90_000_000_000_000_000u64);
    h.ok(
        0,
        receiver,
        via(names, Names::commitCall { commitment: hash }, bond).abi_encode(),
        bond,
        "names contract committer bond",
    );
    h.advance(60);
    let reveal = Names::registerCall {
        name: "refund".into(),
        owner: receiver,
        salt,
        relayer: Address::ZERO,
    };
    ok!(
        h,
        0,
        receiver,
        Callback::setRejectPaymentCall { reject: true },
        "names refund rejected"
    );
    h.revert(
        0,
        receiver,
        via(names, reveal.clone(), due + uint(1)).abi_encode(),
        due + uint(1),
        "names failed register refund rollback",
    );
    ok!(
        h,
        0,
        receiver,
        Callback::setRejectPaymentCall { reject: false },
        "names refund accepted"
    );
    ok!(
        h,
        0,
        receiver,
        Callback::configureCall {
            target_: names,
            data_: reveal.abi_encode().into()
        },
        "names register refund reentry configured"
    );
    h.ok(
        0,
        receiver,
        via(names, reveal, due + uint(1)).abi_encode(),
        due + uint(1),
        "names refund cannot consume commitment twice",
    );
    assert_reentry_error(&h, receiver, "UnknownCommitment()");
    ok!(
        h,
        0,
        receiver,
        Callback::setRejectPaymentCall { reject: true },
        "names renew refund rejected"
    );
    let renew = Names::renewCall {
        name: "refund".into(),
    };
    let fee = U256::from(100_000_000_000_000_000u64);
    h.revert(
        0,
        receiver,
        via(names, renew.clone(), fee + uint(1)).abi_encode(),
        fee + uint(1),
        "names failed renew refund rollback",
    );
    ok!(
        h,
        0,
        receiver,
        Callback::setRejectPaymentCall { reject: false },
        "names renew refund accepted"
    );
    h.ok(
        0,
        receiver,
        via(names, renew, fee).abi_encode(),
        fee,
        "names renew exact fee no refund",
    );
    let editions = h.deploy("toolbox/Editions1155", (h.addr(0),).abi_encode_params());
    let create = Editions::createEditionCall {
        name: "callback edition".into(),
        cap: uint(3),
        maxPerWallet: uint(1),
        price: uint(10),
        feeBps: 0,
    };
    ok!(
        h,
        0,
        receiver,
        via(editions, create, U256::ZERO),
        "editions contract creator"
    );
    h.ok(
        1,
        editions,
        Editions::mintCall { editionId: uint(1) }.abi_encode(),
        uint(10),
        "editions callback sale funds",
    );
    ok!(
        h,
        0,
        receiver,
        Callback::setRejectPaymentCall { reject: true },
        "editions creator rejects payout"
    );
    no!(
        h,
        0,
        receiver,
        via(
            editions,
            Editions::withdrawCall { editionId: uint(1) },
            U256::ZERO
        ),
        "editions withdraw failed payout rollback"
    );
    ok!(
        h,
        0,
        receiver,
        Callback::setRejectPaymentCall { reject: false },
        "editions creator accepts payout"
    );
    ok!(
        h,
        0,
        receiver,
        Callback::configureCall {
            target_: editions,
            data_: Editions::withdrawCall { editionId: uint(1) }
                .abi_encode()
                .into()
        },
        "editions withdraw reentry configured"
    );
    ok!(
        h,
        0,
        receiver,
        via(
            editions,
            Editions::withdrawCall { editionId: uint(1) },
            U256::ZERO
        ),
        "editions withdraw nonReentrant"
    );
    assert_reentry_error(&h, receiver, "ReentrancyGuardReentrantCall()");
}
