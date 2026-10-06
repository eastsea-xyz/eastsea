//! DeFi, time-dependent distribution, and signed governance through execute_block.
use super::harness::*;
use aether_crypto::{address_of, Secp256k1Signer, Signer};
use alloy_primitives::keccak256;
use alloy_sol_types::{sol, SolCall, SolValue};

sol! {
    interface Token {
        function mint(address to, uint256 amount) external;
        function approve(address spender, uint256 amount) external returns (bool);
        function transfer(address to, uint256 amount) external returns (bool);
        function balanceOf(address user) external view returns (uint256);
        function totalSupply() external view returns (uint256);
        function setFeeBps(uint16 bps) external;
        function setFailTransfers(bool fail) external;
        function setCallback(address target, bytes data) external;
        function callbackAttempted() external view returns (bool);
        function innerSuccess() external view returns (bool);
    }
    interface Brake { function engageBrake(uint8 state) external; }
    interface Factory {
        function createPair(address tokenA, address tokenB) external returns (address);
        function getPair(address tokenA, address tokenB) external view returns (address);
        function allPairsLength() external view returns (uint256);
    }
    interface Pair {
        function mint(address to) external returns (uint256);
        function burn(address to) external returns (uint256, uint256);
        function swap(uint256 amount0Out, uint256 amount1Out, address to) external;
        function skim(address to) external;
        function sync() external;
        function getReserves() external view returns (uint112, uint112, uint32);
        function price0CumulativeLast() external view returns (uint256);
    }
    interface Router {
        function addLiquidity(address tokenA, address tokenB, uint256 amountADesired, uint256 amountBDesired,
            uint256 amountAMin, uint256 amountBMin, address to, uint256 deadline) external returns (uint256,uint256,uint256);
        function removeLiquidity(address tokenA, address tokenB, uint256 liquidity,
            uint256 amountAMin, uint256 amountBMin, address to, uint256 deadline) external returns (uint256,uint256);
        function swapExactTokensForTokens(uint256 amountIn, uint256 amountOutMin, address[] path,
            address to, uint256 deadline) external returns (uint256[]);
        function swapSupportingFeeOnTransfer(uint256 amountIn, uint256 amountOutMin, address[] path,
            address to, uint256 deadline) external;
    }
    struct LaunchConfig {
        string name; string symbol; uint256 tokenSupply; uint256 quoteFloor; uint256 tokenFloor;
        uint256 graduationTarget; uint256 feeBps; uint256 snipeTaxBps; uint256 snipeWindow;
        uint256 perBuyerCap; address treasury;
    }
    interface Launch {
        function buy(uint256 quoteIn, uint256 minTokensOut) external;
        function sell(uint256 tokenIn, uint256 minQuoteOut) external;
        function graduate() external;
        function curveToken() external view returns (address);
        function graduatePair() external view returns (address);
        function graduated() external view returns (bool);
        function getBuyQuoteOut(uint256 quoteIn) external view returns (uint256);
    }
    interface Rewards {
        function fundRewards(uint256 amount, uint256 durationSec) external;
        function stake(uint256 amount) external;
        function unstake(uint256 amount) external;
        function claim() external returns (uint256);
        function earned(address user) external view returns (uint256);
        function userStaked(address user) external view returns (uint256);
        function userOwed(address user) external view returns (uint256);
        function totalStaked() external view returns (uint256);
    }
    interface Lock {
        function lockFor(address beneficiary, uint256 amount, uint256 cliffSec, uint256 durationSec) external;
        function release() external;
        function releasable(address beneficiary) external view returns (uint256);
    }
    interface Vesting {
        function claim() external returns (uint256);
        function claimable() external view returns (uint256);
    }
    interface Raffle {
        function enter() external payable;
        function reveal(bytes32 seed) external;
        function drawWithoutSeed() external;
        function playerCount() external view returns (uint256);
        function winner() external view returns (address);
        function drawn() external view returns (bool);
    }
    interface Callback {
        function configure(address target, bytes data) external;
        function setRejectPayment(bool reject) external;
        function execute(address target, bytes data, uint256 value) external payable returns (bool, bytes);
        function callbackAttempted() external view returns (bool);
        function innerSuccess() external view returns (bool);
    }
    interface Dao {
        function propose(bytes32 executionHash) external returns (uint256);
        function execute(uint256 proposalId, address target, uint256 value, bytes data, bytes[] signatures)
            external returns (bytes);
        function state(uint256 proposalId) external view returns (uint8);
        function getVoteHash(uint256 proposalId) external view returns (bytes32);
    }
    interface Multisig {
        function execute(address to, uint256 value, bytes data, uint256 nonce, bytes[] signatures)
            external returns (bytes);
        function domainSeparator() external view returns (bytes32);
        function executed(bytes32 h) external view returns (bool);
        function getTransactionHash(address to, uint256 value, bytes data, uint256 nonce)
            external view returns (bytes32);
    }
}

fn n(value: u64) -> U256 {
    U256::from(value)
}
fn call<C: SolCall>(c: C) -> Vec<u8> {
    c.abi_encode()
}
fn read(h: &mut Harness, to: Address, data: Vec<u8>) -> U256 {
    let bytes = h.view(0, to, data);
    assert_eq!(bytes.len(), 32);
    U256::from_be_slice(&bytes)
}
fn read_address(h: &mut Harness, to: Address, data: Vec<u8>) -> Address {
    let bytes = h.view(0, to, data);
    assert_eq!(bytes.len(), 32);
    Address::from_slice(&bytes[12..])
}
fn balance(h: &mut Harness, token: Address, user: Address) -> U256 {
    read(h, token, call(Token::balanceOfCall { user }))
}
fn token(h: &mut Harness, fee_bps: u16) -> Address {
    h.deploy("support/TestToken", (fee_bps,).abi_encode_params())
}
fn mint(h: &mut Harness, token: Address, user: Address, amount: U256) {
    h.ok(
        0,
        token,
        call(Token::mintCall { to: user, amount }),
        U256::ZERO,
        "test token fund",
    );
}
fn approve(h: &mut Harness, actor: u8, token: Address, spender: Address) {
    h.ok(
        actor,
        token,
        call(Token::approveCall {
            spender,
            amount: U256::MAX,
        }),
        U256::ZERO,
        "token approval",
    );
}
fn brake_cases(h: &mut Harness, contract: Address) {
    for (actor, state, label) in [
        (1, 1, "brake wrong guardian"),
        (0, 0, "brake zero"),
        (0, 3, "brake out of range"),
    ] {
        h.revert(
            actor,
            contract,
            call(Brake::engageBrakeCall { state }),
            U256::ZERO,
            label,
        );
    }
    h.ok(
        0,
        contract,
        call(Brake::engageBrakeCall { state: 1 }),
        U256::ZERO,
        "brake new entry",
    );
    h.revert(
        0,
        contract,
        call(Brake::engageBrakeCall { state: 1 }),
        U256::ZERO,
        "brake cannot repeat",
    );
    h.ok(
        0,
        contract,
        call(Brake::engageBrakeCall { state: 2 }),
        U256::ZERO,
        "brake strengthen",
    );
    h.revert(
        0,
        contract,
        call(Brake::engageBrakeCall { state: 1 }),
        U256::ZERO,
        "brake cannot weaken",
    );
}
fn assert_guarded_callback(h: &mut Harness, token: Address) {
    assert_eq!(read(h, token, call(Token::callbackAttemptedCall {})), n(1));
    assert_eq!(read(h, token, call(Token::innerSuccessCall {})), U256::ZERO);
}

#[test]
fn token_lock_cliff_linear_release_rollback_and_braked_exit() {
    let mut h = Harness::new();
    let owner = h.addr(0);
    let beneficiary = h.addr(1);
    let t = token(&mut h, 0);
    let lock = h.deploy("toolbox/TokenTimeLock", (owner, t).abi_encode_params());
    mint(&mut h, t, owner, n(100_000));
    approve(&mut h, 0, t, lock);
    for (amount, cliff, duration, label) in [
        (U256::ZERO, 0, 100, "lock zero amount"),
        (n(1), 0, 0, "lock zero duration"),
        (n(1), 101, 100, "lock cliff exceeds duration"),
        (U256::MAX, 0, 100, "lock exceeds uint128"),
    ] {
        h.revert(
            0,
            lock,
            call(Lock::lockForCall {
                beneficiary,
                amount,
                cliffSec: n(cliff),
                durationSec: n(duration),
            }),
            U256::ZERO,
            label,
        );
    }
    h.ok(
        0,
        t,
        call(Token::setFeeBpsCall { bps: 1000 }),
        U256::ZERO,
        "enable taxed exact deposit",
    );
    h.revert(
        0,
        lock,
        call(Lock::lockForCall {
            beneficiary,
            amount: n(1000),
            cliffSec: n(1000),
            durationSec: n(2000),
        }),
        U256::ZERO,
        "lock rejects fee on transfer",
    );
    h.ok(
        0,
        t,
        call(Token::setFeeBpsCall { bps: 0 }),
        U256::ZERO,
        "restore exact deposit",
    );
    h.ok(
        0,
        t,
        call(Token::setCallbackCall {
            target: lock,
            data: call(Lock::releaseCall {}).into(),
        }),
        U256::ZERO,
        "lock callback setup",
    );
    h.ok(
        0,
        lock,
        call(Lock::lockForCall {
            beneficiary,
            amount: n(10_000),
            cliffSec: n(1000),
            durationSec: n(2000),
        }),
        U256::ZERO,
        "create linear token lock",
    );
    let start = h.timestamp() - 1;
    assert_guarded_callback(&mut h, t);
    h.revert(
        0,
        lock,
        call(Lock::lockForCall {
            beneficiary,
            amount: n(1),
            cliffSec: n(0),
            durationSec: n(1),
        }),
        U256::ZERO,
        "active lock cannot be overwritten",
    );
    h.revert(
        2,
        lock,
        call(Lock::releaseCall {}),
        U256::ZERO,
        "non beneficiary has nothing to release",
    );
    h.at(start + 1000);
    h.revert(
        1,
        lock,
        call(Lock::releaseCall {}),
        U256::ZERO,
        "cliff boundary remains locked",
    );
    h.at(start + 1500);
    assert_eq!(
        read(&mut h, lock, call(Lock::releasableCall { beneficiary })),
        n(5000)
    );
    h.ok(
        0,
        t,
        call(Token::setFailTransfersCall { fail: true }),
        U256::ZERO,
        "lock payout failure setup",
    );
    h.revert(
        1,
        lock,
        call(Lock::releaseCall {}),
        U256::ZERO,
        "lock failed payout rolls released back",
    );
    h.ok(
        0,
        t,
        call(Token::setFailTransfersCall { fail: false }),
        U256::ZERO,
        "lock restore payout",
    );
    h.ok(
        1,
        lock,
        call(Lock::releaseCall {}),
        U256::ZERO,
        "partial token unlock",
    );
    assert!(balance(&mut h, t, beneficiary) >= n(5000));
    brake_cases(&mut h, lock);
    h.revert(
        0,
        lock,
        call(Lock::lockForCall {
            beneficiary: h.addr(2),
            amount: n(1),
            cliffSec: n(0),
            durationSec: n(1),
        }),
        U256::ZERO,
        "lock new entry braked",
    );
    h.at(start + 2000);
    h.ok(
        1,
        lock,
        call(Lock::releaseCall {}),
        U256::ZERO,
        "full release survives brake",
    );
    assert_eq!(balance(&mut h, t, beneficiary), n(10_000));
    h.revert(
        1,
        lock,
        call(Lock::releaseCall {}),
        U256::ZERO,
        "lock double release",
    );
    h.deploy_revert(
        "toolbox/TokenTimeLock",
        (owner, h.addr(3)).abi_encode_params(),
        "lock token without code",
    );
    // An inactive, fully paid grant may be replaced without another contract.
    let reusable = h.deploy("toolbox/TokenTimeLock", (owner, t).abi_encode_params());
    approve(&mut h, 0, t, reusable);
    h.ok(
        0,
        reusable,
        call(Lock::lockForCall {
            beneficiary,
            amount: n(100),
            cliffSec: n(0),
            durationSec: n(10),
        }),
        U256::ZERO,
        "first reusable token grant",
    );
    h.advance(10);
    h.ok(
        1,
        reusable,
        call(Lock::releaseCall {}),
        U256::ZERO,
        "complete reusable token grant",
    );
    h.ok(
        0,
        reusable,
        call(Lock::lockForCall {
            beneficiary,
            amount: n(200),
            cliffSec: n(0),
            durationSec: n(10),
        }),
        U256::ZERO,
        "regression completed lock accepts next grant",
    );
    h.advance(10);
    h.ok(
        1,
        reusable,
        call(Lock::releaseCall {}),
        U256::ZERO,
        "release replacement token grant",
    );
    assert_eq!(balance(&mut h, t, beneficiary), n(10_300));
}

#[test]
fn linear_vesting_preapproval_donations_and_public_claim() {
    let mut h = Harness::new();
    let owner = h.addr(0);
    let beneficiary = h.addr(1);
    let t = token(&mut h, 0);
    mint(&mut h, t, owner, n(30_000));
    // Approval consumes one nonce before the constructor pulls the grant.
    let predicted = owner.create(h.nonce(0) + 1);
    approve(&mut h, 0, t, predicted);
    let vest = h.deploy(
        "toolbox/LinearVesting",
        (t, beneficiary, n(10_000), n(1000), n(2000)).abi_encode_params(),
    );
    let start = h.timestamp() - 1;
    assert_eq!(vest, predicted);
    h.ok(
        2,
        vest,
        call(Vesting::claimCall {}),
        U256::ZERO,
        "vesting early claim pays zero",
    );
    h.at(start + 1000);
    assert_eq!(
        read(&mut h, vest, call(Vesting::claimableCall {})),
        U256::ZERO
    );
    h.at(start + 1500);
    h.ok(
        2,
        vest,
        call(Vesting::claimCall {}),
        U256::ZERO,
        "vesting public partial claim",
    );
    assert_eq!(balance(&mut h, t, beneficiary), n(5000));
    h.ok(
        0,
        t,
        call(Token::transferCall {
            to: vest,
            amount: n(10_000),
        }),
        U256::ZERO,
        "vesting unsolicited donation",
    );
    h.ok(
        0,
        t,
        call(Token::setCallbackCall {
            target: vest,
            data: call(Vesting::claimCall {}).into(),
        }),
        U256::ZERO,
        "vesting reentry setup",
    );
    h.at(start + 2000);
    h.ok(
        0,
        t,
        call(Token::setFailTransfersCall { fail: true }),
        U256::ZERO,
        "vesting payout failure setup",
    );
    h.revert(
        3,
        vest,
        call(Vesting::claimCall {}),
        U256::ZERO,
        "vesting failed transfer preserves claimed amount",
    );
    h.ok(
        0,
        t,
        call(Token::setFailTransfersCall { fail: false }),
        U256::ZERO,
        "vesting payout recovery",
    );
    h.ok(
        3,
        vest,
        call(Vesting::claimCall {}),
        U256::ZERO,
        "vesting final beneficiary payout",
    );
    assert_guarded_callback(&mut h, t);
    h.ok(
        3,
        vest,
        call(Vesting::claimCall {}),
        U256::ZERO,
        "vesting repeated claim pays zero",
    );
    assert_eq!(balance(&mut h, t, beneficiary), n(10_000));
    assert_eq!(balance(&mut h, t, vest), n(10_000));
    for (who, amount, cliff, duration, label) in [
        (Address::ZERO, n(1), n(0), n(1), "vesting zero beneficiary"),
        (beneficiary, U256::ZERO, n(0), n(1), "vesting zero amount"),
        (
            beneficiary,
            U256::MAX,
            n(0),
            n(1),
            "vesting oversized amount",
        ),
        (beneficiary, n(1), n(0), n(0), "vesting zero duration"),
        (beneficiary, n(1), n(2), n(1), "vesting invalid cliff"),
    ] {
        h.deploy_revert(
            "toolbox/LinearVesting",
            (t, who, amount, cliff, duration).abi_encode_params(),
            label,
        );
    }
    h.deploy_revert(
        "toolbox/LinearVesting",
        (h.addr(2), beneficiary, n(1), n(0), n(1)).abi_encode_params(),
        "vesting token without code",
    );
    h.ok(
        0,
        t,
        call(Token::setFeeBpsCall { bps: 1000 }),
        U256::ZERO,
        "vesting taxed constructor setup",
    );
    let predicted = owner.create(h.nonce(0) + 1);
    approve(&mut h, 0, t, predicted);
    h.deploy_revert(
        "toolbox/LinearVesting",
        (t, beneficiary, n(100), n(0), n(10)).abi_encode_params(),
        "vesting rejects taxed constructor grant",
    );
}

#[test]
fn rewards_taxed_staking_checkpoint_claim_and_emergency_exit() {
    let mut h = Harness::new();
    let owner = h.addr(0);
    let user = h.addr(1);
    let stake = token(&mut h, 1000);
    let reward = token(&mut h, 0);
    let pool = h.deploy(
        "toolbox/RewardDistributor",
        (owner, stake, reward).abi_encode_params(),
    );
    mint(&mut h, stake, user, n(20_000));
    mint(&mut h, reward, owner, n(1_000_000));
    approve(&mut h, 1, stake, pool);
    approve(&mut h, 0, reward, pool);
    h.revert(
        1,
        pool,
        call(Rewards::stakeCall { amount: U256::ZERO }),
        U256::ZERO,
        "rewards zero stake",
    );
    h.revert(
        0,
        pool,
        call(Rewards::fundRewardsCall {
            amount: U256::ZERO,
            durationSec: n(1000),
        }),
        U256::ZERO,
        "rewards zero funding",
    );
    h.revert(
        0,
        pool,
        call(Rewards::fundRewardsCall {
            amount: n(1),
            durationSec: U256::ZERO,
        }),
        U256::ZERO,
        "rewards zero duration",
    );
    h.revert(
        1,
        pool,
        call(Rewards::unstakeCall { amount: n(1) }),
        U256::ZERO,
        "rewards excess withdrawal",
    );
    h.ok(
        0,
        stake,
        call(Token::setCallbackCall {
            target: pool,
            data: call(Rewards::claimCall {}).into(),
        }),
        U256::ZERO,
        "staking reentry setup",
    );
    h.ok(
        1,
        pool,
        call(Rewards::stakeCall { amount: n(10_000) }),
        U256::ZERO,
        "stake credited from actual taxed arrival",
    );
    assert_guarded_callback(&mut h, stake);
    assert_eq!(
        read(&mut h, pool, call(Rewards::userStakedCall { user })),
        n(9000)
    );
    h.ok(
        0,
        reward,
        call(Token::setFeeBpsCall { bps: 1000 }),
        U256::ZERO,
        "taxed reward funding setup",
    );
    h.revert(
        0,
        pool,
        call(Rewards::fundRewardsCall {
            amount: n(100_000),
            durationSec: n(1000),
        }),
        U256::ZERO,
        "reward funding must be exact",
    );
    h.ok(
        0,
        reward,
        call(Token::setFeeBpsCall { bps: 0 }),
        U256::ZERO,
        "exact reward funding restore",
    );
    h.ok(
        0,
        pool,
        call(Rewards::fundRewardsCall {
            amount: n(100_000),
            durationSec: n(1000),
        }),
        U256::ZERO,
        "fund one reward period",
    );
    let funded = h.timestamp() - 1;
    h.at(funded + 500);
    assert!(read(&mut h, pool, call(Rewards::earnedCall { user })) >= n(49_999));
    h.ok(
        1,
        pool,
        call(Rewards::claimCall {}),
        U256::ZERO,
        "claim accrued staking rewards",
    );
    brake_cases(&mut h, pool);
    h.revert(
        1,
        pool,
        call(Rewards::stakeCall { amount: n(1) }),
        U256::ZERO,
        "brake rejects fresh staking",
    );
    h.revert(
        0,
        pool,
        call(Rewards::fundRewardsCall {
            amount: n(1),
            durationSec: n(1),
        }),
        U256::ZERO,
        "brake rejects fresh funding",
    );
    h.ok(
        1,
        pool,
        call(Rewards::unstakeCall { amount: n(9000) }),
        U256::ZERO,
        "full emergency unstake checkpoints rewards",
    );
    assert_eq!(
        read(&mut h, pool, call(Rewards::totalStakedCall {})),
        U256::ZERO
    );
    let owed = read(&mut h, pool, call(Rewards::userOwedCall { user }));
    assert!(owed > U256::ZERO);
    let before = balance(&mut h, reward, user);
    h.ok(
        0,
        reward,
        call(Token::setFailTransfersCall { fail: true }),
        U256::ZERO,
        "reward payout failure setup",
    );
    h.revert(
        1,
        pool,
        call(Rewards::claimCall {}),
        U256::ZERO,
        "failed reward payout preserves owed",
    );
    assert_eq!(
        read(&mut h, pool, call(Rewards::userOwedCall { user })),
        owed
    );
    h.ok(
        0,
        reward,
        call(Token::setFailTransfersCall { fail: false }),
        U256::ZERO,
        "reward payout restore",
    );
    h.ok(
        1,
        pool,
        call(Rewards::claimCall {}),
        U256::ZERO,
        "owed survives complete unstake",
    );
    assert_eq!(balance(&mut h, reward, user), before + owed);
    h.ok(
        1,
        pool,
        call(Rewards::claimCall {}),
        U256::ZERO,
        "double reward claim pays zero",
    );
    h.ok(
        1,
        pool,
        call(Rewards::unstakeCall { amount: U256::ZERO }),
        U256::ZERO,
        "zero unstake remains safe",
    );
    h.deploy_revert(
        "toolbox/RewardDistributor",
        (owner, h.addr(2), reward).abi_encode_params(),
        "reward staking token without code",
    );
    h.deploy_revert(
        "toolbox/RewardDistributor",
        (owner, stake, h.addr(2)).abi_encode_params(),
        "reward token without code",
    );
}

fn raffle(h: &mut Harness, seed: B256) -> Address {
    h.deploy(
        "toolbox/CommitRevealRaffle",
        (h.addr(0), keccak256(seed), n(1000), n(1000), n(1000)).abi_encode_params(),
    )
}

#[test]
fn raffle_commit_reveal_deadlines_withholding_and_payout_rollback() {
    let mut h = Harness::new();
    let seed = B256::repeat_byte(42);
    let r = raffle(&mut h, seed);
    let start = h.timestamp() - 1;
    h.revert(
        1,
        r,
        Vec::new(),
        n(1000),
        "raffle rejects unsolicited native payment",
    );
    h.revert(
        1,
        r,
        call(Raffle::enterCall {}),
        U256::ZERO,
        "raffle incorrect ticket payment",
    );
    h.revert(
        1,
        r,
        call(Raffle::enterCall {}),
        n(1001),
        "raffle excessive ticket payment",
    );
    h.revert(
        1,
        r,
        call(Raffle::revealCall { seed }),
        U256::ZERO,
        "raffle reveal before close",
    );
    h.revert(
        1,
        r,
        call(Raffle::drawWithoutSeedCall {}),
        U256::ZERO,
        "raffle cannot skip reveal window",
    );
    h.ok(
        1,
        r,
        call(Raffle::enterCall {}),
        n(1000),
        "raffle first ticket",
    );
    h.ok(
        2,
        r,
        call(Raffle::enterCall {}),
        n(1000),
        "raffle second ticket",
    );
    assert_eq!(read(&mut h, r, call(Raffle::playerCountCall {})), n(2));
    brake_cases(&mut h, r);
    h.revert(
        3,
        r,
        call(Raffle::enterCall {}),
        n(1000),
        "brake rejects new raffle ticket",
    );
    h.at(start + 1001);
    h.revert(
        1,
        r,
        call(Raffle::revealCall {
            seed: B256::repeat_byte(41),
        }),
        U256::ZERO,
        "raffle wrong seed rollback",
    );
    h.ok(
        3,
        r,
        call(Raffle::revealCall { seed }),
        U256::ZERO,
        "raffle reveal survives brake",
    );
    let winner = read_address(&mut h, r, call(Raffle::winnerCall {}));
    assert!(winner == h.addr(1) || winner == h.addr(2));
    assert_eq!(h.state.balance(&r), U256::ZERO);
    h.revert(
        1,
        r,
        call(Raffle::revealCall { seed }),
        U256::ZERO,
        "raffle cannot redraw with seed",
    );
    h.revert(
        1,
        r,
        call(Raffle::drawWithoutSeedCall {}),
        U256::ZERO,
        "raffle cannot redraw without seed",
    );

    let withheld = raffle(&mut h, seed);
    let start = h.timestamp() - 1;
    h.ok(
        1,
        withheld,
        call(Raffle::enterCall {}),
        n(1000),
        "withheld raffle ticket",
    );
    h.at(start + 1001);
    h.revert(
        1,
        withheld,
        call(Raffle::enterCall {}),
        n(1000),
        "raffle entry after deadline",
    );
    h.at(start + 2001);
    h.revert(
        1,
        withheld,
        call(Raffle::revealCall { seed }),
        U256::ZERO,
        "raffle expired reveal",
    );
    h.ok(
        2,
        withheld,
        call(Raffle::drawWithoutSeedCall {}),
        U256::ZERO,
        "withheld seed cannot strand prize",
    );
    assert_eq!(
        read_address(&mut h, withheld, call(Raffle::winnerCall {})),
        h.addr(1)
    );

    let empty = raffle(&mut h, seed);
    let start = h.timestamp() - 1;
    h.at(start + 1001);
    h.revert(
        1,
        empty,
        call(Raffle::revealCall { seed }),
        U256::ZERO,
        "empty raffle reveal",
    );
    h.at(start + 2001);
    h.revert(
        1,
        empty,
        call(Raffle::drawWithoutSeedCall {}),
        U256::ZERO,
        "empty raffle fallback",
    );

    let receiver = h.deploy("support/NativeCallback", Vec::new());
    let failing = raffle(&mut h, seed);
    let start = h.timestamp() - 1;
    let entry = h.ok(
        1,
        receiver,
        call(Callback::executeCall {
            target: failing,
            data: call(Raffle::enterCall {}).into(),
            value: n(1000),
        }),
        n(1000),
        "contract raffle participant",
    );
    assert_eq!(U256::from_be_slice(&entry.output[..32]), n(1));
    h.ok(
        0,
        receiver,
        call(Callback::setRejectPaymentCall { reject: true }),
        U256::ZERO,
        "reject raffle payout",
    );
    h.at(start + 1001);
    h.revert(
        1,
        failing,
        call(Raffle::revealCall { seed }),
        U256::ZERO,
        "raffle payout failure preserves tickets and drawn flag",
    );
    assert_eq!(
        read(&mut h, failing, call(Raffle::drawnCall {})),
        U256::ZERO
    );
    h.ok(
        0,
        receiver,
        call(Callback::setRejectPaymentCall { reject: false }),
        U256::ZERO,
        "accept raffle payout again",
    );
    h.ok(
        0,
        receiver,
        call(Callback::configureCall {
            target: failing,
            data: call(Raffle::revealCall { seed }).into(),
        }),
        U256::ZERO,
        "raffle payout reentry setup",
    );
    h.ok(
        1,
        failing,
        call(Raffle::revealCall { seed }),
        U256::ZERO,
        "raffle payout after receiver recovery",
    );
    assert_eq!(
        read(&mut h, receiver, call(Callback::callbackAttemptedCall {})),
        n(1)
    );
    assert_eq!(
        read(&mut h, receiver, call(Callback::innerSuccessCall {})),
        U256::ZERO
    );
    for (commit, price, enter, window, label) in [
        (B256::ZERO, n(1), n(1), n(1), "raffle zero commitment"),
        (keccak256(seed), U256::ZERO, n(1), n(1), "raffle zero price"),
        (
            keccak256(seed),
            n(1),
            U256::ZERO,
            n(1),
            "raffle zero entry period",
        ),
        (
            keccak256(seed),
            n(1),
            n(1),
            U256::ZERO,
            "raffle zero reveal period",
        ),
    ] {
        h.deploy_revert(
            "toolbox/CommitRevealRaffle",
            (h.addr(0), commit, price, enter, window).abi_encode_params(),
            label,
        );
    }
}

fn personal_signature(signer: &Secp256k1Signer, inner: B256) -> Bytes {
    let mut preimage = b"\x19Ethereum Signed Message:\n32".to_vec();
    preimage.extend_from_slice(inner.as_slice());
    let mut signature = signer.sign(&preimage).expect("deterministic secp signer");
    signature[64] += 27;
    signature.into()
}
fn secp(seed: u8) -> Secp256k1Signer {
    Secp256k1Signer::from_seed(&[seed; 32]).unwrap()
}
fn multisig_signature(
    h: &mut Harness,
    wallet: Address,
    signer: &Secp256k1Signer,
    to: Address,
    value: U256,
    data: &[u8],
    nonce: U256,
) -> Bytes {
    let domain = B256::from_slice(&h.view(0, wallet, call(Multisig::domainSeparatorCall {})));
    let inner = keccak256((domain, to, value, keccak256(data), nonce).abi_encode_params());
    personal_signature(signer, inner)
}

#[test]
fn multisig_signatures_order_threshold_replay_failure_and_native_reentry() {
    let mut h = Harness::new();
    let a = secp(81);
    let b = secp(82);
    let outsider = secp(83);
    let aa = address_of(&a.public_key()).unwrap();
    let bb = address_of(&b.public_key()).unwrap();
    let mut owners = vec![aa, bb];
    owners.sort();
    let wallet = h.deploy(
        "toolbox/SimpleMultisig",
        (owners.clone(), n(2)).abi_encode_params(),
    );
    h.ok(0, wallet, Vec::new(), n(10_000), "fund multisig treasury");
    let to = h.addr(1);
    let value = n(1000);
    let nonce = n(7);
    let sa = multisig_signature(&mut h, wallet, &a, to, value, &[], nonce);
    let sb = multisig_signature(&mut h, wallet, &b, to, value, &[], nonce);
    let signatures = if aa < bb {
        vec![sa.clone(), sb.clone()]
    } else {
        vec![sb.clone(), sa.clone()]
    };
    for (sigs, label) in [
        (Vec::new(), "multisig missing confirmations"),
        (vec![sa.clone()], "multisig below threshold"),
        (
            vec![Bytes::new(), Bytes::new()],
            "multisig malformed signatures",
        ),
        (
            vec![Bytes::from(vec![0u8; 65]), Bytes::from(vec![0u8; 65])],
            "multisig invalid recovery",
        ),
        (vec![sa.clone(), sa.clone()], "multisig duplicate signer"),
        (
            vec![signatures[1].clone(), signatures[0].clone()],
            "multisig unsorted signatures",
        ),
    ] {
        h.revert(
            3,
            wallet,
            call(Multisig::executeCall {
                to,
                value,
                data: Bytes::new(),
                nonce,
                signatures: sigs,
            }),
            U256::ZERO,
            label,
        );
    }
    let so = multisig_signature(&mut h, wallet, &outsider, to, value, &[], nonce);
    h.revert(
        3,
        wallet,
        call(Multisig::executeCall {
            to,
            value,
            data: Bytes::new(),
            nonce,
            signatures: vec![so, sa],
        }),
        U256::ZERO,
        "multisig non owner signature",
    );
    let input = call(Multisig::executeCall {
        to,
        value,
        data: Bytes::new(),
        nonce,
        signatures: signatures.clone(),
    });
    let before = h.state.balance(&to);
    h.ok(
        3,
        wallet,
        input.clone(),
        U256::ZERO,
        "relayed multisig payout",
    );
    assert_eq!(h.state.balance(&to), before + n(1000));
    h.revert(3, wallet, input, U256::ZERO, "multisig exact replay");
    h.revert(
        3,
        wallet,
        call(Multisig::executeCall {
            to,
            value,
            data: Bytes::new(),
            nonce: n(8),
            signatures,
        }),
        U256::ZERO,
        "multisig nonce domain separation",
    );

    let receiver = h.deploy("support/NativeCallback", Vec::new());
    let nonce = n(9);
    let sa = multisig_signature(&mut h, wallet, &a, receiver, value, &[], nonce);
    let sb = multisig_signature(&mut h, wallet, &b, receiver, value, &[], nonce);
    let sigs = if aa < bb { vec![sa, sb] } else { vec![sb, sa] };
    let input = call(Multisig::executeCall {
        to: receiver,
        value,
        data: Bytes::new(),
        nonce,
        signatures: sigs,
    });
    let hash = B256::from_slice(&h.view(
        0,
        wallet,
        call(Multisig::getTransactionHashCall {
            to: receiver,
            value,
            data: Bytes::new(),
            nonce,
        }),
    ));
    h.ok(
        0,
        receiver,
        call(Callback::setRejectPaymentCall { reject: true }),
        U256::ZERO,
        "multisig failed recipient setup",
    );
    h.revert(
        3,
        wallet,
        input.clone(),
        U256::ZERO,
        "multisig failed call rolls executed back",
    );
    assert_eq!(
        read(&mut h, wallet, call(Multisig::executedCall { h: hash })),
        U256::ZERO
    );
    h.ok(
        0,
        receiver,
        call(Callback::setRejectPaymentCall { reject: false }),
        U256::ZERO,
        "multisig recipient recovery",
    );
    h.ok(
        0,
        receiver,
        call(Callback::configureCall {
            target: wallet,
            data: input.clone().into(),
        }),
        U256::ZERO,
        "multisig same action reentry setup",
    );
    h.ok(
        3,
        wallet,
        input,
        U256::ZERO,
        "multisig payout rejects same action reentry",
    );
    assert_eq!(
        read(&mut h, receiver, call(Callback::callbackAttemptedCall {})),
        n(1)
    );
    assert_eq!(
        read(&mut h, receiver, call(Callback::innerSuccessCall {})),
        U256::ZERO
    );
    h.deploy_revert(
        "toolbox/SimpleMultisig",
        (owners.clone(), U256::ZERO).abi_encode_params(),
        "multisig zero threshold",
    );
    h.deploy_revert(
        "toolbox/SimpleMultisig",
        (owners.clone(), n(3)).abi_encode_params(),
        "multisig threshold exceeds owners",
    );
    h.deploy_revert(
        "toolbox/SimpleMultisig",
        (Vec::<Address>::new(), n(1)).abi_encode_params(),
        "multisig no owners",
    );
    h.deploy_revert(
        "toolbox/SimpleMultisig",
        (vec![Address::ZERO], n(1)).abi_encode_params(),
        "multisig zero owner",
    );
    h.deploy_revert(
        "toolbox/SimpleMultisig",
        (vec![aa, aa], n(1)).abi_encode_params(),
        "multisig duplicate owners",
    );
    owners.reverse();
    h.deploy_revert(
        "toolbox/SimpleMultisig",
        (owners, n(1)).abi_encode_params(),
        "multisig unsorted owners",
    );
}

fn dao_signature(h: &Harness, dao: Address, proposal_id: U256, signer: &Secp256k1Signer) -> Bytes {
    let tag = keccak256(b"eastsea-toolbox.dao.vote.v1");
    let inner = keccak256((n(h.chain_id()), dao, proposal_id, tag).abi_encode_params());
    personal_signature(signer, inner)
}
fn proposal(h: &mut Harness, dao: Address, to: Address, value: U256, data: Bytes) -> (U256, u64) {
    let output = h
        .ok(
            1,
            dao,
            call(Dao::proposeCall {
                executionHash: keccak256((to, value, data).abi_encode_params()),
            }),
            U256::ZERO,
            "permissionless DAO proposal",
        )
        .output;
    (U256::from_be_slice(&output), h.timestamp() - 1)
}

#[test]
fn dao_vote_signatures_weight_timelock_expiry_and_call_rollback() {
    let mut h = Harness::new();
    let owner = h.addr(0);
    let target = h.addr(2);
    let votes = token(&mut h, 0);
    let voter = secp(85);
    let voter_addr = address_of(&voter.public_key()).unwrap();
    mint(&mut h, votes, voter_addr, n(100));
    let dao = h.deploy(
        "toolbox/SimpleDAO",
        (owner, votes, n(100), n(1000), n(1000), n(1000)).abi_encode_params(),
    );
    h.ok(0, dao, Vec::new(), n(10_000), "DAO treasury funding");
    let (id, ts) = proposal(&mut h, dao, target, n(1000), Bytes::new());
    let sig = dao_signature(&h, dao, id, &voter);
    let execute = call(Dao::executeCall {
        proposalId: id,
        target,
        value: n(1000),
        data: Bytes::new(),
        signatures: vec![sig.clone()],
    });
    h.revert(
        3,
        dao,
        call(Dao::executeCall {
            proposalId: n(999),
            target,
            value: U256::ZERO,
            data: Bytes::new(),
            signatures: vec![],
        }),
        U256::ZERO,
        "DAO nonexistent proposal",
    );
    h.revert(
        3,
        dao,
        execute.clone(),
        U256::ZERO,
        "DAO cannot skip vote and timelock",
    );
    assert_eq!(
        read(&mut h, dao, call(Dao::stateCall { proposalId: id })),
        n(1)
    );
    h.at(ts + 1000);
    assert_eq!(
        read(&mut h, dao, call(Dao::stateCall { proposalId: id })),
        n(2)
    );
    h.revert(
        3,
        dao,
        execute.clone(),
        U256::ZERO,
        "DAO vote end remains timelocked",
    );
    h.at(ts + 2000);
    assert_eq!(
        read(&mut h, dao, call(Dao::stateCall { proposalId: id })),
        n(3)
    );
    for (signatures, label) in [
        (vec![], "DAO empty quorum"),
        (vec![Bytes::new()], "DAO malformed vote"),
        (vec![Bytes::from(vec![0u8; 65])], "DAO zero recovered voter"),
        (vec![sig.clone(), sig.clone()], "DAO duplicate voter"),
    ] {
        h.revert(
            3,
            dao,
            call(Dao::executeCall {
                proposalId: id,
                target,
                value: n(1000),
                data: Bytes::new(),
                signatures,
            }),
            U256::ZERO,
            label,
        );
    }
    let no_weight = secp(86);
    let zero_weight_sig = dao_signature(&h, dao, id, &no_weight);
    h.revert(
        3,
        dao,
        call(Dao::executeCall {
            proposalId: id,
            target,
            value: n(1000),
            data: Bytes::new(),
            signatures: vec![zero_weight_sig.clone()],
        }),
        U256::ZERO,
        "DAO signed vote without token weight",
    );
    let no_weight_addr = address_of(&no_weight.public_key()).unwrap();
    let descending = if voter_addr > no_weight_addr {
        vec![sig.clone(), zero_weight_sig]
    } else {
        vec![zero_weight_sig, sig.clone()]
    };
    h.revert(
        3,
        dao,
        call(Dao::executeCall {
            proposalId: id,
            target,
            value: n(1000),
            data: Bytes::new(),
            signatures: descending,
        }),
        U256::ZERO,
        "DAO unsorted voters",
    );
    h.revert(
        3,
        dao,
        call(Dao::executeCall {
            proposalId: id,
            target: h.addr(3),
            value: n(1000),
            data: Bytes::new(),
            signatures: vec![sig.clone()],
        }),
        U256::ZERO,
        "DAO execution hash mismatch",
    );
    brake_cases(&mut h, dao);
    h.revert(
        1,
        dao,
        call(Dao::proposeCall {
            executionHash: B256::repeat_byte(1),
        }),
        U256::ZERO,
        "DAO brake blocks new proposal",
    );
    let before = h.state.balance(&target);
    h.ok(
        3,
        dao,
        execute.clone(),
        U256::ZERO,
        "DAO signed quorum executes at timelock through brake",
    );
    assert_eq!(h.state.balance(&target), before + n(1000));
    assert_eq!(
        read(&mut h, dao, call(Dao::stateCall { proposalId: id })),
        n(4)
    );
    h.revert(
        3,
        dao,
        execute,
        U256::ZERO,
        "DAO proposal cannot execute twice",
    );

    let dao2 = h.deploy(
        "toolbox/SimpleDAO",
        (owner, votes, n(100), n(1000), n(1000), n(1000)).abi_encode_params(),
    );
    let (id, ts) = proposal(&mut h, dao2, target, n(20_000), Bytes::new());
    h.at(ts + 2000);
    let sig = dao_signature(&h, dao2, id, &voter);
    h.revert(
        3,
        dao2,
        call(Dao::executeCall {
            proposalId: id,
            target,
            value: n(20_000),
            data: Bytes::new(),
            signatures: vec![sig],
        }),
        U256::ZERO,
        "DAO insufficient treasury",
    );
    h.at(ts + 3001);
    h.revert(
        3,
        dao2,
        call(Dao::executeCall {
            proposalId: id,
            target,
            value: n(20_000),
            data: Bytes::new(),
            signatures: vec![],
        }),
        U256::ZERO,
        "DAO expired proposal",
    );
    assert_eq!(
        read(&mut h, dao2, call(Dao::stateCall { proposalId: id })),
        n(5)
    );

    let receiver = h.deploy("support/NativeCallback", Vec::new());
    h.ok(0, dao2, Vec::new(), n(1000), "DAO retry treasury funding");
    let (id, ts) = proposal(&mut h, dao2, receiver, n(1000), Bytes::new());
    let sig = dao_signature(&h, dao2, id, &voter);
    let execute = call(Dao::executeCall {
        proposalId: id,
        target: receiver,
        value: n(1000),
        data: Bytes::new(),
        signatures: vec![sig],
    });
    h.ok(
        0,
        receiver,
        call(Callback::setRejectPaymentCall { reject: true }),
        U256::ZERO,
        "DAO failed recipient setup",
    );
    h.at(ts + 2000);
    h.revert(
        3,
        dao2,
        execute.clone(),
        U256::ZERO,
        "DAO failed external call rolls executed back",
    );
    assert_eq!(
        read(&mut h, dao2, call(Dao::stateCall { proposalId: id })),
        n(3)
    );
    h.ok(
        0,
        receiver,
        call(Callback::setRejectPaymentCall { reject: false }),
        U256::ZERO,
        "DAO recipient recovery",
    );
    h.ok(
        0,
        receiver,
        call(Callback::configureCall {
            target: dao2,
            data: execute.clone().into(),
        }),
        U256::ZERO,
        "DAO same proposal reentry setup",
    );
    h.ok(
        3,
        dao2,
        execute,
        U256::ZERO,
        "DAO final call succeeds with reentry denied",
    );
    assert_eq!(
        read(&mut h, receiver, call(Callback::innerSuccessCall {})),
        U256::ZERO
    );
    for (tok, quorum, voting, delay, grace, label) in [
        (h.addr(4), n(1), n(1), n(1), n(1), "DAO token without code"),
        (votes, U256::ZERO, n(1), n(1), n(1), "DAO zero quorum"),
        (votes, n(1), U256::ZERO, n(1), n(1), "DAO zero vote period"),
        (votes, n(1), n(1), U256::ZERO, n(1), "DAO zero timelock"),
        (votes, n(1), n(1), n(1), U256::ZERO, "DAO zero grace"),
    ] {
        h.deploy_revert(
            "toolbox/SimpleDAO",
            (owner, tok, quorum, voting, delay, grace).abi_encode_params(),
            label,
        );
    }
}

fn add(
    a: Address,
    b: Address,
    to: Address,
    amount_a: U256,
    amount_b: U256,
    min_a: U256,
    min_b: U256,
    deadline: U256,
) -> Vec<u8> {
    call(Router::addLiquidityCall {
        tokenA: a,
        tokenB: b,
        amountADesired: amount_a,
        amountBDesired: amount_b,
        amountAMin: min_a,
        amountBMin: min_b,
        to,
        deadline,
    })
}
fn remove(
    a: Address,
    b: Address,
    to: Address,
    liquidity: U256,
    min_a: U256,
    min_b: U256,
    deadline: U256,
) -> Vec<u8> {
    call(Router::removeLiquidityCall {
        tokenA: a,
        tokenB: b,
        liquidity,
        amountAMin: min_a,
        amountBMin: min_b,
        to,
        deadline,
    })
}

#[test]
fn amm_factory_pair_router_liquidity_swaps_taxed_inputs_twap_and_braked_exit() {
    let mut h = Harness::new();
    let owner = h.addr(0);
    let recipient = h.addr(1);
    let a = token(&mut h, 0);
    let b = token(&mut h, 0);
    let (t0, t1) = if a < b { (a, b) } else { (b, a) };
    let factory = h.deploy("toolbox/AmmFactory", (owner,).abi_encode_params());
    let router = h.deploy("toolbox/AmmRouter", (factory,).abi_encode_params());
    let direct = h.deploy("toolbox/AmmPair", (t0, t1, factory).abi_encode_params());
    h.revert(
        0,
        direct,
        call(Pair::mintCall { to: owner }),
        U256::ZERO,
        "pair empty first mint",
    );
    h.revert(
        0,
        direct,
        call(Pair::burnCall { to: owner }),
        U256::ZERO,
        "pair empty burn",
    );
    h.ok(
        0,
        direct,
        call(Pair::syncCall {}),
        U256::ZERO,
        "empty pair sync",
    );
    for (token_a, token_b, label) in [
        (a, a, "factory identical tokens"),
        (Address::ZERO, b, "factory zero token"),
        (h.addr(1), b, "factory first token without code"),
        (a, h.addr(2), "factory second token without code"),
    ] {
        h.revert(
            0,
            factory,
            call(Factory::createPairCall {
                tokenA: token_a,
                tokenB: token_b,
            }),
            U256::ZERO,
            label,
        );
    }
    h.ok(
        0,
        factory,
        call(Factory::createPairCall {
            tokenA: a,
            tokenB: b,
        }),
        U256::ZERO,
        "factory pair creation",
    );
    let pair = read_address(
        &mut h,
        factory,
        call(Factory::getPairCall {
            tokenA: a,
            tokenB: b,
        }),
    );
    h.label(pair, "toolbox/AmmPair");
    assert_ne!(pair, Address::ZERO);
    assert_eq!(
        read_address(
            &mut h,
            factory,
            call(Factory::getPairCall {
                tokenA: b,
                tokenB: a
            })
        ),
        pair
    );
    assert_eq!(
        read(&mut h, factory, call(Factory::allPairsLengthCall {})),
        n(1)
    );
    h.revert(
        1,
        factory,
        call(Factory::createPairCall {
            tokenA: b,
            tokenB: a,
        }),
        U256::ZERO,
        "factory reversed duplicate pair",
    );
    mint(&mut h, a, owner, n(1_000_000));
    mint(&mut h, b, owner, n(1_000_000));
    approve(&mut h, 0, a, router);
    approve(&mut h, 0, b, router);
    let deadline = U256::MAX;
    h.revert(
        0,
        router,
        add(
            a,
            b,
            owner,
            n(100_000),
            n(100_000),
            U256::ZERO,
            U256::ZERO,
            U256::ZERO,
        ),
        U256::ZERO,
        "router expired add",
    );
    h.revert(
        0,
        router,
        add(
            a,
            b,
            owner,
            U256::ZERO,
            U256::ZERO,
            U256::ZERO,
            U256::ZERO,
            deadline,
        ),
        U256::ZERO,
        "router zero initial liquidity",
    );
    let initial = add(
        a,
        b,
        owner,
        n(100_000),
        n(100_000),
        U256::ZERO,
        U256::ZERO,
        deadline,
    );
    h.ok(
        0,
        a,
        call(Token::setCallbackCall {
            target: router,
            data: initial.clone().into(),
        }),
        U256::ZERO,
        "router liquidity callback setup",
    );
    h.ok(0, router, initial, U256::ZERO, "router initial liquidity");
    assert_guarded_callback(&mut h, a);
    h.ok(
        0,
        a,
        call(Token::setCallbackCall {
            target: Address::ZERO,
            data: Bytes::new(),
        }),
        U256::ZERO,
        "disable router liquidity callback",
    );
    assert_eq!(balance(&mut h, pair, owner), n(99_000));
    h.revert(
        0,
        router,
        add(a, b, owner, n(1000), n(1000), U256::ZERO, n(2000), deadline),
        U256::ZERO,
        "router excessive A versus minimum B",
    );
    h.revert(
        0,
        router,
        add(
            a,
            b,
            owner,
            n(10000),
            n(1000),
            n(2000),
            U256::ZERO,
            deadline,
        ),
        U256::ZERO,
        "router excessive B versus minimum A",
    );
    h.ok(
        0,
        router,
        add(
            a,
            b,
            owner,
            n(10_000),
            n(10_000),
            n(9000),
            n(9000),
            deadline,
        ),
        U256::ZERO,
        "router proportional additional liquidity",
    );
    h.revert(
        0,
        pair,
        call(Pair::mintCall { to: owner }),
        U256::ZERO,
        "pair cannot mint without incoming liquidity",
    );
    h.revert(
        0,
        pair,
        call(Pair::burnCall { to: owner }),
        U256::ZERO,
        "pair cannot burn without LP",
    );
    for (out0, out1, label) in [
        (U256::ZERO, U256::ZERO, "pair zero swap outputs"),
        (n(110_000), U256::ZERO, "pair exceeds reserve output"),
        (n(1), U256::ZERO, "pair swap missing input"),
    ] {
        h.revert(
            0,
            pair,
            call(Pair::swapCall {
                amount0Out: out0,
                amount1Out: out1,
                to: recipient,
            }),
            U256::ZERO,
            label,
        );
    }
    for (path, label) in [
        (vec![], "router empty swap path"),
        (vec![a], "router one token swap path"),
        (vec![a, b, a, b, a], "router overlong swap path"),
    ] {
        h.revert(
            0,
            router,
            call(Router::swapExactTokensForTokensCall {
                amountIn: n(100),
                amountOutMin: U256::ZERO,
                path: path.clone(),
                to: recipient,
                deadline,
            }),
            U256::ZERO,
            label,
        );
        h.revert(
            0,
            router,
            call(Router::swapSupportingFeeOnTransferCall {
                amountIn: n(100),
                amountOutMin: U256::ZERO,
                path,
                to: recipient,
                deadline,
            }),
            U256::ZERO,
            label,
        );
    }
    h.revert(
        0,
        router,
        call(Router::swapExactTokensForTokensCall {
            amountIn: U256::ZERO,
            amountOutMin: U256::ZERO,
            path: vec![a, b],
            to: recipient,
            deadline,
        }),
        U256::ZERO,
        "router zero swap input",
    );
    h.revert(
        0,
        router,
        call(Router::swapExactTokensForTokensCall {
            amountIn: n(1000),
            amountOutMin: U256::MAX,
            path: vec![a, b],
            to: recipient,
            deadline,
        }),
        U256::ZERO,
        "router swap slippage rollback",
    );
    h.revert(
        0,
        router,
        call(Router::swapSupportingFeeOnTransferCall {
            amountIn: n(1000),
            amountOutMin: U256::MAX,
            path: vec![a, b],
            to: recipient,
            deadline,
        }),
        U256::ZERO,
        "router taxed swap slippage rollback",
    );
    h.revert(
        0,
        router,
        call(Router::swapExactTokensForTokensCall {
            amountIn: n(1000),
            amountOutMin: U256::ZERO,
            path: vec![a, b],
            to: recipient,
            deadline: U256::ZERO,
        }),
        U256::ZERO,
        "router expired exact swap",
    );
    h.revert(
        0,
        router,
        call(Router::swapSupportingFeeOnTransferCall {
            amountIn: n(1000),
            amountOutMin: U256::ZERO,
            path: vec![a, b],
            to: recipient,
            deadline: U256::ZERO,
        }),
        U256::ZERO,
        "router expired taxed swap",
    );
    h.ok(
        0,
        router,
        call(Router::swapExactTokensForTokensCall {
            amountIn: n(1000),
            amountOutMin: n(1),
            path: vec![a, b],
            to: recipient,
            deadline,
        }),
        U256::ZERO,
        "router exact input swap",
    );
    let received = balance(&mut h, b, recipient);
    assert!(received > U256::ZERO);
    h.ok(
        0,
        a,
        call(Token::setFeeBpsCall { bps: 1000 }),
        U256::ZERO,
        "enable taxed AMM input",
    );
    h.revert(
        0,
        router,
        call(Router::swapExactTokensForTokensCall {
            amountIn: n(1000),
            amountOutMin: U256::ZERO,
            path: vec![a, b],
            to: recipient,
            deadline,
        }),
        U256::ZERO,
        "ordinary swap refuses taxed input",
    );
    h.ok(
        0,
        router,
        call(Router::swapSupportingFeeOnTransferCall {
            amountIn: n(1000),
            amountOutMin: n(1),
            path: vec![a, b],
            to: recipient,
            deadline,
        }),
        U256::ZERO,
        "supporting swap observes taxed arrivals",
    );
    assert!(balance(&mut h, b, recipient) > received);
    h.ok(
        0,
        a,
        call(Token::setFeeBpsCall { bps: 0 }),
        U256::ZERO,
        "disable taxed AMM input",
    );
    let cumulative = read(&mut h, pair, call(Pair::price0CumulativeLastCall {}));
    h.advance(10);
    h.ok(
        0,
        pair,
        call(Pair::syncCall {}),
        U256::ZERO,
        "pair multi block TWAP sync",
    );
    assert!(read(&mut h, pair, call(Pair::price0CumulativeLastCall {})) > cumulative);
    h.ok(
        0,
        a,
        call(Token::transferCall {
            to: pair,
            amount: n(100),
        }),
        U256::ZERO,
        "pair unsolicited token donation",
    );
    let before = balance(&mut h, a, recipient);
    h.ok(
        0,
        pair,
        call(Pair::skimCall { to: recipient }),
        U256::ZERO,
        "pair skim excess without changing reserves",
    );
    assert_eq!(balance(&mut h, a, recipient), before + n(100));
    h.ok(
        0,
        t0,
        call(Token::transferCall {
            to: pair,
            amount: n(1),
        }),
        U256::ZERO,
        "pair small swap input",
    );
    h.revert(
        0,
        pair,
        call(Pair::swapCall {
            amount0Out: U256::ZERO,
            amount1Out: n(50_000),
            to: recipient,
        }),
        U256::ZERO,
        "pair constant product violation rollback",
    );
    h.ok(
        0,
        pair,
        call(Pair::skimCall { to: owner }),
        U256::ZERO,
        "pair recover unused swap input",
    );
    approve(&mut h, 0, pair, router);
    h.revert(
        0,
        router,
        remove(a, b, owner, n(1000), U256::ZERO, U256::ZERO, U256::ZERO),
        U256::ZERO,
        "router expired removal",
    );
    h.revert(
        0,
        router,
        remove(a, b, owner, n(1000), U256::MAX, U256::ZERO, deadline),
        U256::ZERO,
        "router removal A slippage rollback",
    );
    h.revert(
        0,
        router,
        remove(a, b, owner, n(1000), U256::ZERO, U256::MAX, deadline),
        U256::ZERO,
        "router removal B slippage rollback",
    );
    h.ok(
        0,
        a,
        call(Token::setFailTransfersCall { fail: true }),
        U256::ZERO,
        "pair failed token output setup",
    );
    h.revert(
        0,
        router,
        remove(a, b, owner, n(1000), U256::ZERO, U256::ZERO, deadline),
        U256::ZERO,
        "pair failed burn output restores LP",
    );
    h.revert(
        0,
        pair,
        call(Pair::skimCall { to: owner }),
        U256::ZERO,
        "pair failed skim transfer",
    );
    h.ok(
        0,
        a,
        call(Token::setFailTransfersCall { fail: false }),
        U256::ZERO,
        "restore pair token output",
    );
    brake_cases(&mut h, factory);
    h.revert(
        0,
        factory,
        call(Factory::createPairCall {
            tokenA: a,
            tokenB: b,
        }),
        U256::ZERO,
        "factory new pair braked",
    );
    h.revert(
        0,
        pair,
        call(Pair::mintCall { to: owner }),
        U256::ZERO,
        "pair mint follows factory brake",
    );
    h.revert(
        0,
        pair,
        call(Pair::swapCall {
            amount0Out: n(1),
            amount1Out: U256::ZERO,
            to: owner,
        }),
        U256::ZERO,
        "pair swap follows factory brake",
    );
    h.ok(
        0,
        a,
        call(Token::setCallbackCall {
            target: pair,
            data: call(Pair::syncCall {}).into(),
        }),
        U256::ZERO,
        "pair burn reentry setup",
    );
    h.ok(
        0,
        router,
        remove(a, b, owner, n(10_000), U256::ZERO, U256::ZERO, deadline),
        U256::ZERO,
        "router removal remains open through full factory brake",
    );
    assert_guarded_callback(&mut h, a);
    h.ok(
        0,
        a,
        call(Token::setCallbackCall {
            target: Address::ZERO,
            data: Bytes::new(),
        }),
        U256::ZERO,
        "disable pair callback",
    );
    // The uint112 reserve ceiling must produce a clean revert, not a halted block.
    mint(&mut h, t0, direct, U256::from(1u128 << 112));
    h.revert(
        0,
        direct,
        call(Pair::syncCall {}),
        U256::ZERO,
        "pair uint112 reserve overflow rollback",
    );
    h.advance(1);
}

fn launch_config(treasury: Address, cap: U256) -> LaunchConfig {
    LaunchConfig {
        name: "Executor curve".into(),
        symbol: "EC".into(),
        tokenSupply: n(1_000_000),
        quoteFloor: n(100_000),
        tokenFloor: n(100_000),
        graduationTarget: n(100_000),
        feeBps: n(100),
        snipeTaxBps: n(1000),
        snipeWindow: n(1000),
        perBuyerCap: cap,
        treasury,
    }
}

#[test]
fn bonding_curve_buy_sell_tax_decay_graduation_and_emergency_exit() {
    let mut h = Harness::new();
    let owner = h.addr(0);
    let user = h.addr(1);
    let treasury = h.addr(2);
    let quote = token(&mut h, 100);
    let factory = h.deploy("toolbox/AmmFactory", (owner,).abi_encode_params());
    let cfg = launch_config(treasury, U256::ZERO);
    let launch = h.deploy(
        "toolbox/BondingLaunchpad",
        (owner, quote, factory, cfg.clone()).abi_encode_params(),
    );
    let launch_at = h.timestamp() - 1;
    let curve = read_address(&mut h, launch, call(Launch::curveTokenCall {}));
    h.label(curve, "toolbox/FixedSupplyToken");
    mint(&mut h, quote, user, n(1_000_000));
    approve(&mut h, 1, quote, launch);
    approve(&mut h, 1, curve, launch);
    h.revert(
        1,
        launch,
        call(Launch::buyCall {
            quoteIn: U256::ZERO,
            minTokensOut: U256::ZERO,
        }),
        U256::ZERO,
        "launch zero buy input",
    );
    h.revert(
        1,
        launch,
        call(Launch::buyCall {
            quoteIn: n(1000),
            minTokensOut: U256::MAX,
        }),
        U256::ZERO,
        "launch buy slippage rollback",
    );
    h.revert(
        1,
        launch,
        call(Launch::sellCall {
            tokenIn: U256::ZERO,
            minQuoteOut: U256::ZERO,
        }),
        U256::ZERO,
        "launch zero sell input",
    );
    h.revert(
        1,
        launch,
        call(Launch::graduateCall {}),
        U256::ZERO,
        "launch graduation below target",
    );
    let taxed_quote = read(
        &mut h,
        launch,
        call(Launch::getBuyQuoteOutCall { quoteIn: n(10_000) }),
    );
    h.at(launch_at + 1000);
    assert!(
        read(
            &mut h,
            launch,
            call(Launch::getBuyQuoteOutCall { quoteIn: n(10_000) })
        ) > taxed_quote
    );
    h.ok(
        0,
        quote,
        call(Token::setCallbackCall {
            target: launch,
            data: call(Launch::graduateCall {}).into(),
        }),
        U256::ZERO,
        "curve reentry setup",
    );
    h.ok(
        1,
        launch,
        call(Launch::buyCall {
            quoteIn: n(20_000),
            minTokensOut: n(1),
        }),
        U256::ZERO,
        "curve buy with actual taxed quote arrival",
    );
    assert_guarded_callback(&mut h, quote);
    assert!(balance(&mut h, curve, user) > U256::ZERO);
    assert!(balance(&mut h, quote, treasury) > U256::ZERO);
    h.revert(
        1,
        launch,
        call(Launch::sellCall {
            tokenIn: n(1000),
            minQuoteOut: U256::MAX,
        }),
        U256::ZERO,
        "curve sell slippage rollback",
    );
    h.ok(
        1,
        launch,
        call(Launch::sellCall {
            tokenIn: n(1000),
            minQuoteOut: n(1),
        }),
        U256::ZERO,
        "curve sell actual quote payout",
    );
    h.ok(
        0,
        quote,
        call(Token::setCallbackCall {
            target: Address::ZERO,
            data: Bytes::new(),
        }),
        U256::ZERO,
        "disable curve callback",
    );
    h.ok(
        1,
        launch,
        call(Launch::buyCall {
            quoteIn: n(200_000),
            minTokensOut: n(1),
        }),
        U256::ZERO,
        "curve reaches graduation target",
    );
    h.revert(
        1,
        launch,
        call(Launch::buyCall {
            quoteIn: n(1),
            minTokensOut: U256::ZERO,
        }),
        U256::ZERO,
        "curve cannot buy past target",
    );
    h.ok(
        3,
        launch,
        call(Launch::graduateCall {}),
        U256::ZERO,
        "permissionless curve graduation seeds AMM",
    );
    assert_eq!(read(&mut h, launch, call(Launch::graduatedCall {})), n(1));
    let pair = read_address(&mut h, launch, call(Launch::graduatePairCall {}));
    assert_ne!(pair, Address::ZERO);
    h.label(pair, "toolbox/AmmPair");
    let dead = Address::from_slice(&[
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xde, 0xad,
    ]);
    assert!(balance(&mut h, pair, dead) > n(1000));
    h.revert(
        1,
        launch,
        call(Launch::graduateCall {}),
        U256::ZERO,
        "curve cannot graduate twice",
    );
    h.revert(
        1,
        launch,
        call(Launch::buyCall {
            quoteIn: n(1),
            minTokensOut: U256::ZERO,
        }),
        U256::ZERO,
        "graduated curve buy closed",
    );
    h.revert(
        1,
        launch,
        call(Launch::sellCall {
            tokenIn: n(1),
            minQuoteOut: U256::ZERO,
        }),
        U256::ZERO,
        "graduated curve sell closed",
    );

    let limited = h.deploy(
        "toolbox/BondingLaunchpad",
        (owner, quote, factory, launch_config(treasury, n(1))).abi_encode_params(),
    );
    approve(&mut h, 1, quote, limited);
    h.revert(
        1,
        limited,
        call(Launch::buyCall {
            quoteIn: n(1000),
            minTokensOut: U256::ZERO,
        }),
        U256::ZERO,
        "curve buyer holding cap",
    );
    let braked = h.deploy(
        "toolbox/BondingLaunchpad",
        (owner, quote, factory, cfg.clone()).abi_encode_params(),
    );
    approve(&mut h, 1, quote, braked);
    let curve2 = read_address(&mut h, braked, call(Launch::curveTokenCall {}));
    approve(&mut h, 1, curve2, braked);
    h.ok(
        1,
        braked,
        call(Launch::buyCall {
            quoteIn: n(10_000),
            minTokensOut: n(1),
        }),
        U256::ZERO,
        "curve position before brake",
    );
    brake_cases(&mut h, braked);
    h.revert(
        1,
        braked,
        call(Launch::buyCall {
            quoteIn: n(1),
            minTokensOut: U256::ZERO,
        }),
        U256::ZERO,
        "brake rejects curve buy",
    );
    h.revert(
        1,
        braked,
        call(Launch::graduateCall {}),
        U256::ZERO,
        "brake rejects curve graduation",
    );
    h.ok(
        1,
        braked,
        call(Launch::sellCall {
            tokenIn: n(1000),
            minQuoteOut: n(1),
        }),
        U256::ZERO,
        "curve emergency sell survives full brake",
    );
    for (token_supply, quote_floor, token_floor, target, fee, tax, window, treas, label) in [
        (
            U256::ZERO,
            n(1),
            n(1),
            n(1),
            n(0),
            n(0),
            n(0),
            treasury,
            "launch zero supply",
        ),
        (
            n(1),
            U256::ZERO,
            n(1),
            n(1),
            n(0),
            n(0),
            n(0),
            treasury,
            "launch zero quote floor",
        ),
        (
            n(1),
            n(1),
            U256::ZERO,
            n(1),
            n(0),
            n(0),
            n(0),
            treasury,
            "launch zero token floor",
        ),
        (
            n(1),
            n(1),
            n(1),
            U256::ZERO,
            n(0),
            n(0),
            n(0),
            treasury,
            "launch zero target",
        ),
        (
            n(1),
            n(1),
            n(1),
            n(1),
            n(10_001),
            n(0),
            n(0),
            treasury,
            "launch excessive aggregate fees",
        ),
        (
            n(1),
            n(1),
            n(1),
            n(1),
            n(0),
            n(1),
            n(0),
            treasury,
            "launch tax without decay window",
        ),
        (
            n(1),
            n(1),
            n(1),
            n(1),
            n(0),
            n(0),
            n(0),
            Address::ZERO,
            "launch zero treasury",
        ),
    ] {
        let mut invalid = cfg.clone();
        invalid.tokenSupply = token_supply;
        invalid.quoteFloor = quote_floor;
        invalid.tokenFloor = token_floor;
        invalid.graduationTarget = target;
        invalid.feeBps = fee;
        invalid.snipeTaxBps = tax;
        invalid.snipeWindow = window;
        invalid.treasury = treas;
        h.deploy_revert(
            "toolbox/BondingLaunchpad",
            (owner, quote, factory, invalid).abi_encode_params(),
            label,
        );
    }
    h.deploy_revert(
        "toolbox/BondingLaunchpad",
        (owner, h.addr(3), factory, cfg.clone()).abi_encode_params(),
        "launch quote token without code",
    );
    h.deploy_revert(
        "toolbox/BondingLaunchpad",
        (owner, quote, h.addr(3), cfg).abi_encode_params(),
        "launch factory without code",
    );
}

#[test]
fn launchpad_precreated_pair_reuse_poisoned_pair_and_zero_output() {
    let mut h = Harness::new();
    let owner = h.addr(0);
    let user = h.addr(1);
    let treasury = h.addr(2);
    let quote = token(&mut h, 0);
    let factory = h.deploy("toolbox/AmmFactory", (owner,).abi_encode_params());
    let router = h.deploy("toolbox/AmmRouter", (factory,).abi_encode_params());
    mint(&mut h, quote, user, n(1_000_000));
    approve(&mut h, 1, quote, router);
    let mut cfg = launch_config(treasury, U256::ZERO);
    cfg.feeBps = U256::ZERO;
    cfg.snipeTaxBps = U256::ZERO;
    cfg.graduationTarget = n(20_000);

    let reusable = h.deploy(
        "toolbox/BondingLaunchpad",
        (owner, quote, factory, cfg.clone()).abi_encode_params(),
    );
    let curve = read_address(&mut h, reusable, call(Launch::curveTokenCall {}));
    approve(&mut h, 1, quote, reusable);
    h.ok(
        3,
        factory,
        call(Factory::createPairCall {
            tokenA: curve,
            tokenB: quote,
        }),
        U256::ZERO,
        "precreate empty graduation pair",
    );
    let expected = read_address(
        &mut h,
        factory,
        call(Factory::getPairCall {
            tokenA: curve,
            tokenB: quote,
        }),
    );
    h.label(expected, "toolbox/AmmPair");
    h.ok(
        1,
        reusable,
        call(Launch::buyCall {
            quoteIn: n(20_000),
            minTokensOut: n(1),
        }),
        U256::ZERO,
        "empty pair curve reaches target",
    );
    h.ok(
        3,
        reusable,
        call(Launch::graduateCall {}),
        U256::ZERO,
        "graduation safely reuses precreated empty pair",
    );
    assert_eq!(
        read_address(&mut h, reusable, call(Launch::graduatePairCall {})),
        expected
    );

    let poisoned = h.deploy(
        "toolbox/BondingLaunchpad",
        (owner, quote, factory, cfg.clone()).abi_encode_params(),
    );
    let curve = read_address(&mut h, poisoned, call(Launch::curveTokenCall {}));
    approve(&mut h, 1, quote, poisoned);
    approve(&mut h, 1, curve, poisoned);
    approve(&mut h, 1, curve, router);
    h.ok(
        1,
        poisoned,
        call(Launch::buyCall {
            quoteIn: n(1000),
            minTokensOut: n(1),
        }),
        U256::ZERO,
        "buy token before third party liquidity",
    );
    h.revert(
        1,
        poisoned,
        call(Launch::sellCall {
            tokenIn: n(1),
            minQuoteOut: U256::ZERO,
        }),
        U256::ZERO,
        "curve sell dust produces zero output",
    );
    h.ok(
        1,
        router,
        add(
            curve,
            quote,
            user,
            n(5000),
            n(5000),
            U256::ZERO,
            U256::ZERO,
            U256::MAX,
        ),
        U256::ZERO,
        "third party seeds graduation pair with redeemable LP",
    );
    h.ok(
        1,
        poisoned,
        call(Launch::buyCall {
            quoteIn: n(20_000),
            minTokensOut: n(1),
        }),
        U256::ZERO,
        "poisoned pair curve reaches target",
    );
    h.revert(
        3,
        poisoned,
        call(Launch::graduateCall {}),
        U256::ZERO,
        "graduation refuses preseeded redeemable liquidity",
    );
    assert_eq!(
        read(&mut h, poisoned, call(Launch::graduatedCall {})),
        U256::ZERO
    );
    h.ok(
        1,
        poisoned,
        call(Launch::sellCall {
            tokenIn: n(1000),
            minQuoteOut: n(1),
        }),
        U256::ZERO,
        "failed graduation retains curve sell exit",
    );

    cfg.feeBps = n(10_000);
    let full_fee = h.deploy(
        "toolbox/BondingLaunchpad",
        (owner, quote, factory, cfg).abi_encode_params(),
    );
    approve(&mut h, 1, quote, full_fee);
    h.revert(
        1,
        full_fee,
        call(Launch::buyCall {
            quoteIn: n(1000),
            minTokensOut: U256::ZERO,
        }),
        U256::ZERO,
        "fully taxed curve buy refuses zero effective input",
    );
}

#[test]
fn amm_multihop_missing_pairs_empty_reserves_and_taxed_liquidity() {
    let mut h = Harness::new();
    let owner = h.addr(0);
    let user = h.addr(1);
    let a = token(&mut h, 1000);
    let b = token(&mut h, 0);
    let c = token(&mut h, 0);
    let factory = h.deploy("toolbox/AmmFactory", (owner,).abi_encode_params());
    let router = h.deploy("toolbox/AmmRouter", (factory,).abi_encode_params());
    for t in [a, b, c] {
        mint(&mut h, t, owner, n(1_000_000));
        approve(&mut h, 0, t, router);
    }
    h.revert(
        0,
        router,
        remove(a, b, owner, n(1), U256::ZERO, U256::ZERO, U256::MAX),
        U256::ZERO,
        "router refuses removal from missing pair",
    );
    h.revert(
        0,
        router,
        call(Router::swapExactTokensForTokensCall {
            amountIn: n(1000),
            amountOutMin: U256::ZERO,
            path: vec![a, b],
            to: user,
            deadline: U256::MAX,
        }),
        U256::ZERO,
        "router refuses swap through missing pair",
    );
    h.ok(
        0,
        factory,
        call(Factory::createPairCall {
            tokenA: a,
            tokenB: b,
        }),
        U256::ZERO,
        "create empty taxed pair",
    );
    h.revert(
        0,
        router,
        call(Router::swapExactTokensForTokensCall {
            amountIn: n(1000),
            amountOutMin: U256::ZERO,
            path: vec![a, b],
            to: user,
            deadline: U256::MAX,
        }),
        U256::ZERO,
        "router refuses swap through zero reserves",
    );
    h.ok(
        0,
        router,
        add(
            a,
            b,
            owner,
            n(100_000),
            n(100_000),
            U256::ZERO,
            U256::ZERO,
            U256::MAX,
        ),
        U256::ZERO,
        "AMM liquidity minted from taxed actual deposits",
    );
    let pair = read_address(
        &mut h,
        factory,
        call(Factory::getPairCall {
            tokenA: a,
            tokenB: b,
        }),
    );
    h.label(pair, "toolbox/AmmPair");
    // Two transfers (user->router->pair) each charge ten percent.
    assert_eq!(balance(&mut h, a, pair), n(81_000));
    h.ok(
        0,
        router,
        add(
            b,
            c,
            owner,
            n(100_000),
            n(100_000),
            U256::ZERO,
            U256::ZERO,
            U256::MAX,
        ),
        U256::ZERO,
        "second hop AMM liquidity",
    );
    h.ok(
        0,
        router,
        call(Router::swapSupportingFeeOnTransferCall {
            amountIn: n(1000),
            amountOutMin: n(1),
            path: vec![a, b, c],
            to: user,
            deadline: U256::MAX,
        }),
        U256::ZERO,
        "taxed input multihop uses observed arrivals at every pair",
    );
    assert!(balance(&mut h, c, user) > U256::ZERO);
    h.ok(
        0,
        a,
        call(Token::setFeeBpsCall { bps: 0 }),
        U256::ZERO,
        "restore exact multihop input",
    );
    let before = balance(&mut h, c, user);
    h.ok(
        0,
        router,
        call(Router::swapExactTokensForTokensCall {
            amountIn: n(1000),
            amountOutMin: n(1),
            path: vec![a, b, c],
            to: user,
            deadline: U256::MAX,
        }),
        U256::ZERO,
        "exact input multihop routes outputs directly between pairs",
    );
    assert!(balance(&mut h, c, user) > before);
}

#[test]
fn dao_signatures_use_execution_time_weight_and_instance_domain() {
    let mut h = Harness::new();
    let owner = h.addr(0);
    let target = h.addr(1);
    let votes = token(&mut h, 0);
    let voter = secp(87);
    let voter_addr = address_of(&voter.public_key()).unwrap();
    let args = (owner, votes, n(100), n(1000), n(1000), n(1000)).abi_encode_params();
    let dao = h.deploy("toolbox/SimpleDAO", args.clone());
    let other = h.deploy("toolbox/SimpleDAO", args);
    let (id, ts) = proposal(&mut h, dao, target, U256::ZERO, Bytes::new());
    let signature = dao_signature(&h, dao, id, &voter);
    let execute = call(Dao::executeCall {
        proposalId: id,
        target,
        value: U256::ZERO,
        data: Bytes::new(),
        signatures: vec![signature.clone()],
    });
    let (other_id, other_ts) = proposal(&mut h, other, target, U256::ZERO, Bytes::new());
    h.at((ts + 2000).max(other_ts + 2000));
    h.revert(
        2,
        dao,
        execute.clone(),
        U256::ZERO,
        "DAO signature with zero execution time weight",
    );
    // Votes deliberately do not snapshot the signing-time balance.
    mint(&mut h, votes, voter_addr, n(100));
    h.ok(
        2,
        dao,
        execute,
        U256::ZERO,
        "DAO tokens acquired after signing count at execution",
    );
    h.revert(
        2,
        other,
        call(Dao::executeCall {
            proposalId: other_id,
            target,
            value: U256::ZERO,
            data: Bytes::new(),
            signatures: vec![signature],
        }),
        U256::ZERO,
        "DAO vote signature cannot replay across instances",
    );
}
