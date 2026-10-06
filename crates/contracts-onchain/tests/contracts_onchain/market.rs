//! ERC-721 escrow, royalties and pull-payment callbacks on the real chain.
use super::harness::*;
use alloy_primitives::keccak256;
use alloy_sol_types::{sol, SolCall, SolValue};

sol! {
    interface Market {
        function list(address token, uint256 tokenId, uint256 price);
        function buy(uint256 listingId);
        function cancel(uint256 listingId);
        function withdraw();
        function engageBrake(uint8 state);
        function credits(address account) returns (uint256 value);
        function listingOf(uint256 listingId) returns (address seller, address token, uint256 tokenId, uint256 price, bool active);
        function onERC721Received(address operator, address from, uint256 tokenId, bytes data) returns (bytes4 value);
    }
    interface Nft {
        function mint(address to, uint8 color, uint8 shape, uint8 pattern, uint8 halo);
        function approve(address to, uint256 tokenId);
        function setApprovalForAll(address operator, bool approved);
        function ownerOf(uint256 tokenId) returns (address owner);
    }
    interface MockNFT {
        function mint(address to);
        function configureRoyalty(uint8 mode, address receiver, uint256 amount);
        function setFailTransfers(bool fail);
    }
    interface Receiver {
        function executeOrRevert(address target_, bytes data_, uint256 value) returns (bytes result);
        function configure(address target_, bytes data_);
        function setRejectPayment(bool reject);
        function setNFTCallback(bool enabled);
        function setRejectNFT(bool reject);
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
fn u(n: u64) -> U256 {
    U256::from(n)
}
fn via(target: Address, call: impl SolCall, value: U256) -> Receiver::executeOrRevertCall {
    Receiver::executeOrRevertCall {
        target_: target,
        data_: call.abi_encode().into(),
        value,
    }
}
fn credit(h: &Harness, market: Address, who: Address) -> U256 {
    Market::creditsCall::abi_decode_returns(&h.view(
        0,
        market,
        Market::creditsCall { account: who }.abi_encode(),
    ))
    .unwrap()
}
fn active(h: &Harness, market: Address, id: u64) -> bool {
    Market::listingOfCall::abi_decode_returns(&h.view(
        0,
        market,
        Market::listingOfCall { listingId: u(id) }.abi_encode(),
    ))
    .unwrap()
    .active
}
fn nft_owner(h: &Harness, nft: Address, id: u64) -> Address {
    Nft::ownerOfCall::abi_decode_returns(&h.view(
        0,
        nft,
        Nft::ownerOfCall { tokenId: u(id) }.abi_encode(),
    ))
    .unwrap()
}
fn solvency(h: &Harness, market: Address, accounts: &[Address]) {
    let sum = accounts
        .iter()
        .fold(U256::ZERO, |n, who| n + credit(h, market, *who));
    assert_eq!(
        sum,
        h.state.balance(&market),
        "escrow credits must equal payable balance"
    );
}
fn reentry_guard(h: &Harness, receiver: Address) {
    assert!(Receiver::callbackAttemptedCall::abi_decode_returns(&h.view(
        0,
        receiver,
        Receiver::callbackAttemptedCall {}.abi_encode()
    ))
    .unwrap());
    assert!(!Receiver::innerSuccessCall::abi_decode_returns(&h.view(
        0,
        receiver,
        Receiver::innerSuccessCall {}.abi_encode()
    ))
    .unwrap());
    let output = Receiver::innerOutputCall::abi_decode_returns(&h.view(
        0,
        receiver,
        Receiver::innerOutputCall {}.abi_encode(),
    ))
    .unwrap();
    assert_eq!(
        &output[..4],
        &keccak256(b"ReentrancyGuardReentrantCall()")[..4]
    );
}

#[test]
fn market_royalty_escrow_all_methods_wrong_calls_and_braked_exits() {
    let mut h = Harness::new();
    let creator = h.addr(0);
    let seller = h.addr(1);
    let buyer = h.addr(2);
    let market = h.deploy("toolbox/FixedPriceMarket", (creator,).abi_encode_params());
    let nft = h.deploy(
        "toolbox/OnchainNFT",
        ("Market NFT", "MKT", u(5), u(500), creator).abi_encode_params(),
    );
    for _ in 0..3 {
        ok!(
            h,
            0,
            nft,
            Nft::mintCall {
                to: seller,
                color: 0,
                shape: 0,
                pattern: 0,
                halo: 0
            },
            "market mint inventory"
        );
    }
    let list = Market::listCall {
        token: nft,
        tokenId: u(1),
        price: u(100),
    };
    let mut invalid = list.clone();
    invalid.price = U256::ZERO;
    no!(h, 1, market, invalid, "list zero price");
    no!(h, 2, market, list.clone(), "list wrong owner");
    no!(h, 1, market, list.clone(), "list lacks approval");
    no!(
        h,
        1,
        market,
        Market::listCall {
            token: nft,
            tokenId: u(99),
            price: u(100)
        },
        "list unknown NFT"
    );
    no!(
        h,
        1,
        market,
        Market::listCall {
            token: h.addr(15),
            tokenId: u(1),
            price: u(100)
        },
        "list no-code token rejects ABI return"
    );
    let editions = h.deploy("toolbox/Editions1155", (creator,).abi_encode_params());
    no!(
        h,
        1,
        market,
        Market::listCall {
            token: editions,
            tokenId: u(1),
            price: u(100)
        },
        "list ERC1155 unsupported rejects cleanly"
    );
    ok!(
        h,
        1,
        nft,
        Nft::approveCall {
            to: market,
            tokenId: u(1)
        },
        "market approve token"
    );
    ok!(h, 1, market, list.clone(), "list escrows approved token");
    assert_eq!(nft_owner(&h, nft, 1), market);
    let listing = Market::listingOfCall::abi_decode_returns(&h.view(
        0,
        market,
        Market::listingOfCall { listingId: u(1) }.abi_encode(),
    ))
    .unwrap();
    assert_eq!(listing.seller, seller);
    assert_eq!(listing.token, nft);
    assert_eq!(listing.tokenId, u(1));
    assert_eq!(listing.price, u(100));
    assert!(listing.active);
    no!(
        h,
        1,
        market,
        list,
        "list double escrow no longer owns token"
    );
    no!(
        h,
        2,
        market,
        Market::buyCall { listingId: u(99) },
        "buy unknown listing"
    );
    no!(
        h,
        2,
        market,
        Market::buyCall {
            listingId: U256::MAX
        },
        "buy maximum listing ID"
    );
    for value in [0, 99, 101] {
        h.revert(
            2,
            market,
            Market::buyCall { listingId: u(1) }.abi_encode(),
            u(value),
            "buy wrong exact price",
        );
    }
    h.ok(
        2,
        market,
        Market::buyCall { listingId: u(1) }.abi_encode(),
        u(100),
        "buy five percent royalty",
    );
    assert_eq!(nft_owner(&h, nft, 1), buyer);
    assert!(!active(&h, market, 1));
    assert_eq!(credit(&h, market, seller), u(95));
    assert_eq!(credit(&h, market, creator), u(5));
    solvency(&h, market, &[seller, creator]);
    h.revert(
        3,
        market,
        Market::buyCall { listingId: u(1) }.abi_encode(),
        u(100),
        "buy sold listing",
    );
    no!(
        h,
        1,
        market,
        Market::cancelCall { listingId: u(1) },
        "cancel sold listing"
    );
    no!(h, 2, market, Market::withdrawCall {}, "withdraw no credit");
    ok!(
        h,
        1,
        nft,
        Nft::setApprovalForAllCall {
            operator: market,
            approved: true
        },
        "market operator approval"
    );
    ok!(
        h,
        1,
        market,
        Market::listCall {
            token: nft,
            tokenId: u(2),
            price: U256::MAX
        },
        "list maximum price cancelable"
    );
    no!(
        h,
        2,
        market,
        Market::cancelCall { listingId: u(2) },
        "cancel wrong seller"
    );
    no!(
        h,
        1,
        market,
        Market::cancelCall { listingId: u(99) },
        "cancel unknown listing"
    );
    ok!(
        h,
        1,
        market,
        Market::cancelCall { listingId: u(2) },
        "cancel escrow returns NFT"
    );
    assert_eq!(nft_owner(&h, nft, 2), seller);
    assert!(!active(&h, market, 2));
    no!(
        h,
        1,
        market,
        Market::cancelCall { listingId: u(2) },
        "cancel double"
    );
    ok!(
        h,
        1,
        market,
        Market::listCall {
            token: nft,
            tokenId: u(3),
            price: u(100)
        },
        "list before brake"
    );
    no!(
        h,
        1,
        market,
        Market::engageBrakeCall { state: 1 },
        "market brake wrong guardian"
    );
    no!(
        h,
        0,
        market,
        Market::engageBrakeCall { state: 0 },
        "market brake zero state"
    );
    no!(
        h,
        0,
        market,
        Market::engageBrakeCall { state: 3 },
        "market brake invalid state"
    );
    ok!(
        h,
        0,
        market,
        Market::engageBrakeCall { state: 1 },
        "market entry brake"
    );
    no!(
        h,
        1,
        market,
        Market::listCall {
            token: nft,
            tokenId: u(2),
            price: u(100)
        },
        "list braked"
    );
    h.revert(
        2,
        market,
        Market::buyCall { listingId: u(3) }.abi_encode(),
        u(100),
        "buy braked",
    );
    no!(
        h,
        0,
        market,
        Market::engageBrakeCall { state: 1 },
        "market brake cannot repeat"
    );
    ok!(
        h,
        0,
        market,
        Market::engageBrakeCall { state: 2 },
        "market full brake"
    );
    no!(
        h,
        0,
        market,
        Market::engageBrakeCall { state: 1 },
        "market brake cannot weaken"
    );
    ok!(
        h,
        1,
        market,
        Market::cancelCall { listingId: u(3) },
        "cancel remains open under full brake"
    );
    ok!(
        h,
        1,
        market,
        Market::withdrawCall {},
        "seller withdraw remains open under full brake"
    );
    ok!(
        h,
        0,
        market,
        Market::withdrawCall {},
        "royalty receiver withdraw under full brake"
    );
    no!(h, 1, market, Market::withdrawCall {}, "withdraw double");
    solvency(&h, market, &[seller, creator]);
    assert_eq!(
        Market::onERC721ReceivedCall::abi_decode_returns(
            &h.view(
                0,
                market,
                Market::onERC721ReceivedCall {
                    operator: creator,
                    from: seller,
                    tokenId: U256::MAX,
                    data: Bytes::new()
                }
                .abi_encode()
            )
        )
        .unwrap()
        .as_slice(),
        &[0x15, 0x0b, 0x7a, 0x02]
    );
}

#[test]
fn market_royalty_fallback_clamp_and_transfer_failure_rollback() {
    let mut h = Harness::new();
    let seller = h.addr(1);
    let royalty = h.addr(3);
    let market = h.deploy("toolbox/FixedPriceMarket", (h.addr(0),).abi_encode_params());
    let nft = h.deploy("support/MarketNFT", vec![]);
    for _ in 0..5 {
        ok!(
            h,
            0,
            nft,
            MockNFT::mintCall { to: seller },
            "market mock inventory"
        );
    }
    ok!(
        h,
        1,
        nft,
        Nft::setApprovalForAllCall {
            operator: market,
            approved: true
        },
        "market mock approval"
    );
    ok!(
        h,
        0,
        nft,
        MockNFT::setFailTransfersCall { fail: true },
        "market injected NFT transfer failure"
    );
    no!(
        h,
        1,
        market,
        Market::listCall {
            token: nft,
            tokenId: u(1),
            price: u(100)
        },
        "list NFT transfer fails restores ID and escrow"
    );
    assert!(!active(&h, market, 1));
    assert_eq!(nft_owner(&h, nft, 1), seller);
    ok!(
        h,
        0,
        nft,
        MockNFT::setFailTransfersCall { fail: false },
        "market NFT transfer enabled"
    );
    for (id, mode, amount) in [(1, 0, 100), (2, 2, 100), (3, 1, 250), (4, 1, 0)] {
        ok!(
            h,
            0,
            nft,
            MockNFT::configureRoyaltyCall {
                mode,
                receiver: royalty,
                amount: u(amount)
            },
            "market royalty mode"
        );
        ok!(
            h,
            1,
            market,
            Market::listCall {
                token: nft,
                tokenId: u(id),
                price: u(100)
            },
            "market royalty branch listing"
        );
        h.ok(
            2,
            market,
            Market::buyCall { listingId: u(id) }.abi_encode(),
            u(100),
            match mode {
                0 => "buy no royalty interface all seller",
                2 => "buy reverting ERC165 falls back",
                _ if amount > 100 => "buy overroyalty clamped",
                _ => "buy supported zero royalty",
            },
        );
        solvency(&h, market, &[seller, royalty]);
    }
    assert_eq!(credit(&h, market, seller), u(300));
    assert_eq!(credit(&h, market, royalty), u(100));
    ok!(
        h,
        0,
        nft,
        MockNFT::configureRoyaltyCall {
            mode: 3,
            receiver: royalty,
            amount: u(5)
        },
        "market throwing royaltyInfo"
    );
    ok!(
        h,
        1,
        market,
        Market::listCall {
            token: nft,
            tokenId: u(5),
            price: u(100)
        },
        "market royalty failure listing"
    );
    h.revert(
        2,
        market,
        Market::buyCall { listingId: u(5) }.abi_encode(),
        u(100),
        "buy reverting royaltyInfo preserves escrow and credits",
    );
    assert!(active(&h, market, 5));
    assert_eq!(nft_owner(&h, nft, 5), market);
    solvency(&h, market, &[seller, royalty]);
    ok!(
        h,
        0,
        nft,
        MockNFT::configureRoyaltyCall {
            mode: 0,
            receiver: royalty,
            amount: u(5)
        },
        "market royalty reset"
    );
    ok!(
        h,
        0,
        nft,
        MockNFT::setFailTransfersCall { fail: true },
        "market outbound transfer rejects"
    );
    h.revert(
        2,
        market,
        Market::buyCall { listingId: u(5) }.abi_encode(),
        u(100),
        "buy token delivery fails rollback",
    );
    no!(
        h,
        1,
        market,
        Market::cancelCall { listingId: u(5) },
        "cancel token return fails rollback"
    );
    assert!(active(&h, market, 5));
    solvency(&h, market, &[seller, royalty]);
    ok!(
        h,
        0,
        nft,
        MockNFT::setFailTransfersCall { fail: false },
        "market outbound transfer accepts"
    );
    ok!(
        h,
        1,
        market,
        Market::cancelCall { listingId: u(5) },
        "cancel after NFT failure recovered"
    );
}

#[test]
fn market_buyer_seller_callbacks_cannot_reenter_and_rejections_rollback() {
    let mut h = Harness::new();
    let seller = h.addr(1);
    let market = h.deploy("toolbox/FixedPriceMarket", (h.addr(0),).abi_encode_params());
    let nft = h.deploy("support/MarketNFT", vec![]);
    let receiver = h.deploy("support/NativeCallback", vec![]);
    ok!(
        h,
        0,
        nft,
        MockNFT::mintCall { to: seller },
        "market buyer callback inventory"
    );
    ok!(
        h,
        1,
        nft,
        Nft::approveCall {
            to: market,
            tokenId: u(1)
        },
        "market buyer callback approval"
    );
    ok!(
        h,
        1,
        market,
        Market::listCall {
            token: nft,
            tokenId: u(1),
            price: u(100)
        },
        "market buyer callback escrow"
    );
    ok!(
        h,
        0,
        receiver,
        Receiver::setRejectNFTCall { reject: true },
        "market buyer rejects NFT"
    );
    h.revert(
        0,
        receiver,
        via(market, Market::buyCall { listingId: u(1) }, u(100)).abi_encode(),
        u(100),
        "buy rejected NFT receiver rolls back paid credit and listing",
    );
    assert!(active(&h, market, 1));
    assert_eq!(credit(&h, market, seller), U256::ZERO);
    assert_eq!(h.state.balance(&market), U256::ZERO);
    ok!(
        h,
        0,
        receiver,
        Receiver::setRejectNFTCall { reject: false },
        "market buyer accepts NFT"
    );
    ok!(
        h,
        0,
        receiver,
        Receiver::setNFTCallbackCall { enabled: true },
        "market NFT callback enabled"
    );
    ok!(
        h,
        0,
        receiver,
        Receiver::configureCall {
            target_: market,
            data_: Market::buyCall { listingId: u(1) }.abi_encode().into()
        },
        "market buyer reentry configured"
    );
    h.ok(
        0,
        receiver,
        via(market, Market::buyCall { listingId: u(1) }, u(100)).abi_encode(),
        u(100),
        "market buyer reentry guarded",
    );
    reentry_guard(&h, receiver);
    assert_eq!(nft_owner(&h, nft, 1), receiver);
    solvency(&h, market, &[seller, receiver]);
    ok!(
        h,
        0,
        receiver,
        Receiver::setNFTCallbackCall { enabled: false },
        "market receiver disable NFT callback"
    );
    ok!(
        h,
        0,
        receiver,
        via(
            nft,
            Nft::approveCall {
                to: market,
                tokenId: u(1)
            },
            U256::ZERO
        ),
        "market contract seller approval"
    );
    ok!(
        h,
        0,
        receiver,
        via(
            market,
            Market::listCall {
                token: nft,
                tokenId: u(1),
                price: u(100)
            },
            U256::ZERO
        ),
        "market contract seller escrow"
    );
    h.ok(
        2,
        market,
        Market::buyCall { listingId: u(2) }.abi_encode(),
        u(100),
        "market contract seller credited",
    );
    ok!(
        h,
        0,
        receiver,
        Receiver::setRejectPaymentCall { reject: true },
        "market seller rejects payout"
    );
    no!(
        h,
        0,
        receiver,
        via(market, Market::withdrawCall {}, U256::ZERO),
        "market withdrawal failed native callback rollback"
    );
    assert_eq!(credit(&h, market, receiver), u(100));
    solvency(&h, market, &[seller, receiver]);
    ok!(
        h,
        0,
        receiver,
        Receiver::setRejectPaymentCall { reject: false },
        "market seller accepts payout"
    );
    ok!(
        h,
        0,
        receiver,
        Receiver::configureCall {
            target_: market,
            data_: Market::withdrawCall {}.abi_encode().into()
        },
        "market withdraw reentry configured"
    );
    ok!(
        h,
        0,
        receiver,
        via(market, Market::withdrawCall {}, U256::ZERO),
        "market withdraw reentry guarded"
    );
    reentry_guard(&h, receiver);
    assert_eq!(credit(&h, market, receiver), U256::ZERO);
    solvency(&h, market, &[seller, receiver]);
    ok!(
        h,
        0,
        nft,
        MockNFT::mintCall { to: receiver },
        "market contract cancel inventory"
    );
    ok!(
        h,
        0,
        receiver,
        via(
            nft,
            Nft::approveCall {
                to: market,
                tokenId: u(2)
            },
            U256::ZERO
        ),
        "market contract cancel approval"
    );
    ok!(
        h,
        0,
        receiver,
        via(
            market,
            Market::listCall {
                token: nft,
                tokenId: u(2),
                price: u(100)
            },
            U256::ZERO
        ),
        "market contract cancel escrow"
    );
    ok!(
        h,
        0,
        receiver,
        Receiver::setRejectNFTCall { reject: true },
        "market cancel receiver rejects"
    );
    no!(
        h,
        0,
        receiver,
        via(market, Market::cancelCall { listingId: u(3) }, U256::ZERO),
        "cancel failed recipient callback restores listing"
    );
    assert!(active(&h, market, 3));
    ok!(
        h,
        0,
        receiver,
        Receiver::setRejectNFTCall { reject: false },
        "market cancel receiver accepts"
    );
    ok!(
        h,
        0,
        receiver,
        Receiver::setNFTCallbackCall { enabled: true },
        "market cancel callback enabled"
    );
    ok!(
        h,
        0,
        receiver,
        Receiver::configureCall {
            target_: market,
            data_: Market::cancelCall { listingId: u(3) }.abi_encode().into()
        },
        "market cancel reentry configured"
    );
    ok!(
        h,
        0,
        receiver,
        via(market, Market::cancelCall { listingId: u(3) }, U256::ZERO),
        "market cancel reentry guarded"
    );
    reentry_guard(&h, receiver);
    assert!(!active(&h, market, 3));
    assert_eq!(nft_owner(&h, nft, 2), receiver);
}

#[test]
fn market_invalid_zero_royalty_receiver_proceeds_remain_withdrawable() {
    let mut h = Harness::new();
    let seller = h.addr(1);
    let market = h.deploy("toolbox/FixedPriceMarket", (h.addr(0),).abi_encode_params());
    let nft = h.deploy("support/MarketNFT", vec![]);
    ok!(
        h,
        0,
        nft,
        MockNFT::mintCall { to: seller },
        "market invalid royalty inventory"
    );
    ok!(
        h,
        0,
        nft,
        MockNFT::configureRoyaltyCall {
            mode: 1,
            receiver: Address::ZERO,
            amount: u(50)
        },
        "market invalid zero royalty receiver"
    );
    ok!(
        h,
        1,
        nft,
        Nft::approveCall {
            to: market,
            tokenId: u(1)
        },
        "market invalid royalty approve"
    );
    ok!(
        h,
        1,
        market,
        Market::listCall {
            token: nft,
            tokenId: u(1),
            price: u(100)
        },
        "market invalid royalty list"
    );
    h.ok(
        2,
        market,
        Market::buyCall { listingId: u(1) }.abi_encode(),
        u(100),
        "buy malformed optional royalty ignored regression",
    );
    assert_eq!(credit(&h, market, seller), u(100));
    assert_eq!(credit(&h, market, Address::ZERO), U256::ZERO);
    solvency(&h, market, &[seller]);
    let before = h.state.balance(&seller);
    let receipt = ok!(
        h,
        1,
        market,
        Market::withdrawCall {},
        "market invalid royalty seller withdraws full proceeds"
    );
    assert_eq!(
        h.state.balance(&seller),
        before + u(100) - receipt.state_fee
    );
    assert_eq!(h.state.balance(&market), U256::ZERO);
}
