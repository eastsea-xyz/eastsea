// SPDX-License-Identifier: MIT
pragma solidity ^0.8.19;

// MerkleDistributor, its factory and TokenBatch, without forge-std (this
// toolchain predates the JSON cheatcodes): asserts revert on failure, negative
// cases use try/catch, and the fixture test reads the JSON built by
// scripts/merkle-build.mjs with the small extractors at the bottom of this file.

import {IERC20, MerkleDistributor, MerkleDistributorFactory, TokenBatch} from "../src/MerkleDistributor.sol";

interface Vm {
    function warp(uint256) external;
    function readFile(string calldata) external returns (bytes memory);
}

abstract contract Test {
    /// Solidity `require` reverts carry this selector (Error(string)).
    bytes4 constant ERROR_STRING = 0x08c379a0;

    function ok(bool c, string memory why) internal pure {
        if (!c) revert(why);
    }

    function eq(uint256 a, uint256 b, string memory why) internal pure {
        ok(a == b, why);
    }

    function eq(bytes32 a, bytes32 b, string memory why) internal pure {
        ok(a == b, why);
    }

    function eq(address a, address b, string memory why) internal pure {
        ok(a == b, why);
    }

    /// The call reverted with the expected selector (custom errors and
    /// Error(string) both start with it).
    function revertedWith(bytes memory reason, bytes4 sel, string memory why) internal pure {
        ok(bytes4(reason) == sel, why);
    }
}

contract TestToken {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);

    function mint(address to, uint256 v) external {
        balanceOf[to] += v;
        emit Transfer(address(0), to, v);
    }

    function approve(address spender, uint256 v) external returns (bool) {
        allowance[msg.sender][spender] = v;
        emit Approval(msg.sender, spender, v);
        return true;
    }

    function transfer(address to, uint256 v) external returns (bool) {
        return _move(msg.sender, to, v);
    }

    function transferFrom(address from, address to, uint256 v) external returns (bool) {
        uint256 left = allowance[from][msg.sender];
        if (left < v) revert("insufficient allowance");
        allowance[from][msg.sender] = left - v;
        return _move(from, to, v);
    }

    function _move(address from, address to, uint256 v) private returns (bool) {
        if (balanceOf[from] < v) revert("insufficient balance");
        balanceOf[from] -= v;
        balanceOf[to] += v;
        emit Transfer(from, to, v);
        return true;
    }
}

/// A second account, so tests show that anyone may relay a claim or be
/// refused a sweep without a vm.prank.
contract Actor {
    function sweep(address d) external {
        MerkleDistributor(d).sweep();
    }

    function claim(address d, uint256 index, address account, uint256 amount, bytes32[] calldata proof) external {
        MerkleDistributor(d).claim(index, account, amount, proof);
    }
}

/// A fee-on-transfer token: every move skims 1% to a collector (F-02).
contract FeeOnTransferToken {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    function mint(address to, uint256 v) external {
        balanceOf[to] += v;
    }

    function approve(address spender, uint256 v) external returns (bool) {
        allowance[msg.sender][spender] = v;
        return true;
    }

    function transfer(address to, uint256 v) external returns (bool) {
        _skim(msg.sender, to, v);
        return true;
    }

    function transferFrom(address from, address to, uint256 v) external returns (bool) {
        allowance[from][msg.sender] -= v;
        _skim(from, to, v);
        return true;
    }

    function _skim(address from, address to, uint256 v) private {
        uint256 fee = v / 100;
        balanceOf[from] -= v;
        balanceOf[to] += v - fee;
        balanceOf[address(0xFEE)] += fee;
    }
}

/// A dishonest token (F-03): every call reports success, balanceOf is
/// honest, but no balance ever moves.
contract LiarToken {
    mapping(address => uint256) public balanceOf;

    function mint(address to, uint256 v) external {
        balanceOf[to] += v;
    }

    function approve(address, uint256) external pure returns (bool) {
        return true;
    }

    function transfer(address, uint256) external pure returns (bool) {
        return true;
    }

    function transferFrom(address, address, uint256) external pure returns (bool) {
        return true;
    }
}

/// An over-delivering token (F-02): every arrival pays 1% interest, so the
/// campaign would hold more than its tree promises.
contract BonusToken {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    function mint(address to, uint256 v) external {
        balanceOf[to] += v;
    }

    function approve(address spender, uint256 v) external returns (bool) {
        allowance[msg.sender][spender] = v;
        return true;
    }

    function transfer(address to, uint256 v) external returns (bool) {
        balanceOf[msg.sender] -= v;
        balanceOf[to] += v + v / 100;
        return true;
    }

    function transferFrom(address from, address to, uint256 v) external returns (bool) {
        allowance[from][msg.sender] -= v;
        balanceOf[from] -= v;
        balanceOf[to] += v + v / 100;
        return true;
    }
}

contract MerkleDistributorTest is Test {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    TestToken token;
    MerkleDistributorFactory factory;
    TokenBatch batch;

    uint64 constant NOW = 1_700_000_000;

    struct Entry {
        uint256 index;
        address account;
        uint256 amount;
    }

    function setUp() public {
        vm.warp(NOW);
        token = new TestToken();
        factory = new MerkleDistributorFactory();
        batch = new TokenBatch();
    }

    // ---- the fixture campaign this file shares with scripts/merkle-build.mjs ----

    /// Five entries, odd so the tree repeats its last node, with one address
    /// paid twice (only the index tells the two leaves apart).
    function campaign() internal pure returns (Entry[] memory) {
        Entry[] memory e = new Entry[](5);
        e[0] = Entry(0, address(0x1), 1_000);
        e[1] = Entry(1, address(0x2), 2_000);
        e[2] = Entry(2, address(0x4), 3_000);
        e[3] = Entry(3, address(0x4), 4_000);
        e[4] = Entry(4, address(0x9), 5_000);
        return e;
    }

    /// The token as the tools see it (an explicit hop between the mock and IERC20).
    function erc() internal view returns (IERC20) {
        return IERC20(address(token));
    }

    function total(Entry[] memory e) internal pure returns (uint256 t) {
        for (uint256 i = 0; i < e.length; i++) t += e[i].amount;
    }

    /// Deploy and fully fund a campaign for these entries in one go.
    function launch(Entry[] memory e, uint64 ends) internal returns (MerkleDistributor d, bytes32[][] memory proofs) {
        bytes32 root;
        (root, proofs) = build(leaves(e));
        token.mint(address(this), total(e));
        token.approve(address(factory), total(e));
        d = factory.create(erc(), root, ends, total(e));
    }

    // ---- factory ----

    function testCreateFundsTheCampaign() external {
        Entry[] memory e = campaign();
        (bytes32 root,) = build(leaves(e));
        token.mint(address(this), total(e));
        token.approve(address(factory), total(e));
        MerkleDistributor d = factory.create(erc(), root, 0, total(e));

        eq(factory.count(), 1, "one campaign");
        ok(address(factory.campaigns(0)) == address(d), "campaign is listed");
        ok(address(d.token()) == address(token), "token");
        ok(d.creator() == address(this), "creator");
        eq(d.merkleRoot(), root, "root");
        eq(d.ends(), 0, "no end");
        eq(token.balanceOf(address(d)), total(e), "fully funded at birth");
        ok(!d.isClaimed(0), "nothing claimed yet");
    }

    function testCreateWithoutEnoughAllowanceReverts() external {
        Entry[] memory e = campaign();
        (bytes32 root,) = build(leaves(e));
        token.mint(address(this), total(e));
        token.approve(address(factory), total(e) - 1);

        try factory.create(erc(), root, 0, total(e)) {
            revert("expected a revert");
        } catch (bytes memory r) {
            revertedWith(r, ERROR_STRING, "the underfunded transferFrom must revert");
        }
        eq(factory.count(), 0, "no campaign was left behind");
    }

    function testCreateChecksEndAndTotal() external {
        token.mint(address(this), 1);
        token.approve(address(factory), 1);

        try factory.create(erc(), bytes32(uint256(1)), NOW - 1, 1) {
            revert("expected a revert");
        } catch (bytes memory r) {
            revertedWith(r, MerkleDistributorFactory.BadEnd.selector, "end in the past");
        }
        try factory.create(erc(), bytes32(uint256(1)), NOW, 1) {
            revert("expected a revert");
        } catch (bytes memory r) {
            revertedWith(r, MerkleDistributorFactory.BadEnd.selector, "end now");
        }
        try factory.create(erc(), bytes32(uint256(1)), 0, 0) {
            revert("expected a revert");
        } catch (bytes memory r) {
            revertedWith(r, MerkleDistributorFactory.BadTotal.selector, "zero total");
        }
    }

    // ---- claims ----

    function testClaimPaysTheAccountNotTheRelayer() external {
        (MerkleDistributor d, bytes32[][] memory proofs) = launch(campaign(), 0);
        Actor relayer = new Actor();
        relayer.claim(address(d), 2, address(0x4), 3_000, proofs[2]);

        eq(token.balanceOf(address(0x4)), 3_000, "the account was paid");
        eq(token.balanceOf(address(relayer)), 0, "the relayer was not");
        ok(d.isClaimed(2), "index 2 is spent");
        ok(!d.isClaimed(3), "the sibling entry is not");
    }

    function testEveryEntryClaimsAndDrainsTheCampaign() external {
        Entry[] memory e = campaign();
        (MerkleDistributor d, bytes32[][] memory proofs) = launch(e, 0);
        for (uint256 i = 0; i < e.length; i++) {
            d.claim(e[i].index, e[i].account, e[i].amount, proofs[i]);
        }
        eq(token.balanceOf(address(0x1)), 1_000, "first paid in full");
        eq(token.balanceOf(address(0x2)), 2_000, "second paid in full");
        eq(token.balanceOf(address(0x4)), 7_000, "the twice-listed address got both grants");
        eq(token.balanceOf(address(0x9)), 5_000, "last paid in full");
        eq(token.balanceOf(address(d)), 0, "drained");
    }

    function testSingleEntryCampaign() external {
        Entry[] memory e = new Entry[](1);
        e[0] = Entry(0, address(0x5), 9);
        (bytes32 root,) = build(leaves(e));
        eq(root, leaves(e)[0], "the root of one leaf is the leaf");
        (MerkleDistributor d, bytes32[][] memory proofs) = launch(e, 0);
        d.claim(0, address(0x5), 9, proofs[0]);
        eq(token.balanceOf(address(0x5)), 9, "claimed with an empty proof");
    }

    function testWrongClaimsRevert() external {
        Entry[] memory e = campaign();
        (MerkleDistributor d, bytes32[][] memory proofs) = launch(e, 0);

        bytes32[] memory oneShort = new bytes32[](proofs[2].length - 1);
        for (uint256 i = 0; i < oneShort.length; i++) oneShort[i] = proofs[2][i];

        // index, account, amount and proof must all belong to the same entry.
        try d.claim(3, address(0x4), 3_000, proofs[2]) {
            revert("expected a revert");
        } catch (bytes memory r) {
            revertedWith(r, MerkleDistributor.BadProof.selector, "wrong index");
        }
        try d.claim(2, address(0x4), 3_001, proofs[2]) {
            revert("expected a revert");
        } catch (bytes memory r) {
            revertedWith(r, MerkleDistributor.BadProof.selector, "wrong amount");
        }
        try d.claim(2, address(0x3), 3_000, proofs[2]) {
            revert("expected a revert");
        } catch (bytes memory r) {
            revertedWith(r, MerkleDistributor.BadProof.selector, "wrong account");
        }
        try d.claim(2, address(0x4), 3_000, oneShort) {
            revert("expected a revert");
        } catch (bytes memory r) {
            revertedWith(r, MerkleDistributor.BadProof.selector, "truncated proof");
        }
        try d.claim(2, address(0x4), 3_000, proofs[1]) {
            revert("expected a revert");
        } catch (bytes memory r) {
            revertedWith(r, MerkleDistributor.BadProof.selector, "someone else's proof");
        }

        eq(token.balanceOf(address(0x4)), 0, "nothing moved");
        ok(!d.isClaimed(2), "still unclaimed");
        // The untouched entry still claims fine afterwards.
        d.claim(0, address(0x1), 1_000, proofs[0]);
        eq(token.balanceOf(address(0x1)), 1_000, "healthy entry still pays");
    }

    function testDoubleClaimReverts() external {
        (MerkleDistributor d, bytes32[][] memory proofs) = launch(campaign(), 0);
        d.claim(1, address(0x2), 2_000, proofs[1]);
        try d.claim(1, address(0x2), 2_000, proofs[1]) {
            revert("expected a revert");
        } catch (bytes memory r) {
            revertedWith(r, MerkleDistributor.AlreadyClaimed.selector, "second claim");
        }
        eq(token.balanceOf(address(0x2)), 2_000, "paid exactly once");
    }

    // ---- end time and sweep ----

    function testEndStopsClaimsAndUnlocksSweep() external {
        uint64 ends = NOW + 1000;
        Entry[] memory e = campaign();
        (MerkleDistributor d, bytes32[][] memory proofs) = launch(e, ends);

        d.claim(0, address(0x1), 1_000, proofs[0]);
        vm.warp(NOW + 999);
        d.claim(1, address(0x2), 2_000, proofs[1]); // the last second still pays

        vm.warp(ends);
        try d.claim(2, address(0x4), 3_000, proofs[2]) {
            revert("expected a revert");
        } catch (bytes memory r) {
            revertedWith(r, MerkleDistributor.Ended.selector, "claims stop at the end time");
        }

        d.sweep(); // at exactly `ends` the creator may take the rest
        eq(token.balanceOf(address(d)), 0, "nothing left in the campaign");
        eq(token.balanceOf(address(this)), total(e) - 3_000, "the creator got the unclaimed part");
    }

    function testSweepBeforeTheEndReverts() external {
        (MerkleDistributor d,) = launch(campaign(), NOW + 1000);
        try d.sweep() {
            revert("expected a revert");
        } catch (bytes memory r) {
            revertedWith(r, MerkleDistributor.NotYet.selector, "too early");
        }
        eq(token.balanceOf(address(d)), total(campaign()), "the campaign still holds its tokens");
    }

    function testOnlyTheCreatorMaySweep() external {
        (MerkleDistributor d,) = launch(campaign(), NOW + 1000);
        vm.warp(NOW + 2000);
        Actor other = new Actor();
        try other.sweep(address(d)) {
            revert("expected a revert");
        } catch (bytes memory r) {
            revertedWith(r, MerkleDistributor.OnlyCreator.selector, "not the creator");
        }
        d.sweep();
        eq(token.balanceOf(address(this)), total(campaign()), "the creator swept");
    }

    function testCampaignWithoutEndRunsForever() external {
        (MerkleDistributor d, bytes32[][] memory proofs) = launch(campaign(), 0);
        vm.warp(type(uint64).max); // 584 billion years in
        d.claim(4, address(0x9), 5_000, proofs[4]);
        try d.sweep() {
            revert("expected a revert");
        } catch (bytes memory r) {
            revertedWith(r, MerkleDistributor.NoEnd.selector, "nothing to sweep without an end");
        }
    }

    // ---- TokenBatch ----

    function testBatchSendMovesTokensStraightToRecipients() external {
        token.mint(address(this), 1_000);
        token.approve(address(batch), 1_000);
        address[3] memory to = [address(0x1), address(0x2), address(0x3)];
        uint256[3] memory amount = [uint256(100), 200, 300];
        batch.send(erc(), _dyn(to), _dyn(amount));

        eq(token.balanceOf(address(0x1)), 100, "first");
        eq(token.balanceOf(address(0x2)), 200, "second");
        eq(token.balanceOf(address(0x3)), 300, "third");
        eq(token.balanceOf(address(this)), 400, "sender kept the rest");
        eq(token.balanceOf(address(batch)), 0, "the batch helper never holds tokens");
    }

    function testBatchSendWithInsufficientAllowanceIsAtomic() external {
        token.mint(address(this), 1_000);
        token.approve(address(batch), 150); // covers the first send only
        address[2] memory to = [address(0x1), address(0x2)];
        uint256[2] memory amount = [uint256(100), 200];
        try batch.send(erc(), _dyn(to), _dyn(amount)) {
            revert("expected a revert");
        } catch (bytes memory r) {
            revertedWith(r, ERROR_STRING, "allowance runs out mid-batch");
        }
        eq(token.balanceOf(address(0x1)), 0, "no partial send");
        eq(token.balanceOf(address(0x2)), 0, "no partial send");
        eq(token.balanceOf(address(this)), 1_000, "the sender kept everything");
        eq(token.allowance(address(this), address(batch)), 150, "allowance untouched");
    }

    function testBatchSendChecksLengths() external {
        token.mint(address(this), 1_000);
        token.approve(address(batch), 1_000);
        address[2] memory to = [address(0x1), address(0x2)];
        uint256[1] memory amount = [uint256(100)];
        try batch.send(erc(), _dyn(to), _dyn(amount)) {
            revert("expected a revert");
        } catch (bytes memory r) {
            revertedWith(r, TokenBatch.LengthMismatch.selector, "mismatched arrays");
        }
        eq(token.balanceOf(address(this)), 1_000, "nothing moved");
    }

    // ---- F-02/F-03: funding that does not arrive must not create a campaign ----

    /// A perpetual campaign (ends == 0) funded with a 1%-fee token delivers
    /// 990 of 1_000 promised tokens: every leaf claim would revert and, with
    /// no end time, the 990 received tokens could never be swept back.
    function testFeeOnTransferFundingIsRefused() external {
        FeeOnTransferToken fee = new FeeOnTransferToken();
        fee.mint(address(this), 1_000);
        fee.approve(address(factory), 1_000);
        Entry[] memory e = new Entry[](1);
        e[0] = Entry(0, address(0x5), 1_000);
        (bytes32 root,) = build(leaves(e));

        try factory.create(IERC20(address(fee)), root, 0, 1_000) {
            revert("expected a revert");
        } catch (bytes memory r) {
            revertedWith(r, MerkleDistributorFactory.Underfunded.selector, "a campaign is funded in full or not at all");
        }
        eq(factory.count(), 0, "no campaign was left behind");
    }

    /// The funded-total check is EQUALITY, not a minimum: a reward token
    /// over-delivering on arrival strands a surplus no leaf can claim, so it
    /// is refused just the same. (Under this policy a fee-on-transfer token
    /// can never fund a campaign — creators use a standard token.)
    function testCreateRefusesAnOverdeliveringToken() external {
        BonusToken bonus = new BonusToken();
        bonus.mint(address(this), 1_000);
        bonus.approve(address(factory), 1_000);
        Entry[] memory e = new Entry[](1);
        e[0] = Entry(0, address(0x5), 100);
        (bytes32 root,) = build(leaves(e));

        try factory.create(IERC20(address(bonus)), root, 0, 100) {
            revert("expected a revert");
        } catch (bytes memory r) {
            revertedWith(r, MerkleDistributorFactory.Underfunded.selector, "a surplus is owed to no leaf");
        }
        eq(factory.count(), 0, "no campaign was left behind");
    }

    /// An EOA passed as the token: the raw transferFrom "succeeds" (empty
    /// return) and a zero-funded campaign would emit Campaign (F-03).
    function testCreateRefusesATokenWithoutCode() external {
        Entry[] memory e = new Entry[](1);
        e[0] = Entry(0, address(0x5), 100);
        (bytes32 root,) = build(leaves(e));

        try factory.create(IERC20(address(0xDEAD)), root, 0, 100) {
            revert("expected a revert");
        } catch (bytes memory r) {
            revertedWith(r, MerkleDistributorFactory.NotAContract.selector, "an EOA is not a token");
        }
        eq(factory.count(), 0, "no campaign was left behind");
    }

    /// A contract that reports success and pays nothing: only the balance
    /// delta exposes it (F-03).
    function testCreateRefusesATokenThatPaysNothing() external {
        LiarToken liar = new LiarToken();
        liar.mint(address(this), 1_000);
        liar.approve(address(factory), 1_000);
        Entry[] memory e = new Entry[](1);
        e[0] = Entry(0, address(0x5), 100);
        (bytes32 root,) = build(leaves(e));

        try factory.create(IERC20(address(liar)), root, 0, 100) {
            revert("expected a revert");
        } catch (bytes memory r) {
            revertedWith(r, MerkleDistributorFactory.Underfunded.selector, "nothing arrived, so nothing is funded");
        }
        eq(factory.count(), 0, "no campaign was left behind");
    }

    // ---- F-03: TokenBatch must not announce sends that never happened ----

    function testBatchSendRefusesATokenWithoutCode() external {
        address[] memory to = new address[](1);
        to[0] = address(0x1);
        uint256[] memory amount = new uint256[](1);
        amount[0] = 100;
        try batch.send(IERC20(address(0xDEAD)), to, amount) {
            revert("expected a revert");
        } catch (bytes memory r) {
            revertedWith(r, TokenBatch.NotAContract.selector, "an EOA is not a token");
        }
    }

    function testBatchSendRefusesATokenThatMovesNothing() external {
        LiarToken liar = new LiarToken();
        liar.mint(address(this), 1_000);
        liar.approve(address(batch), 1_000);
        address[] memory to = new address[](1);
        to[0] = address(0x1);
        uint256[] memory amount = new uint256[](1);
        amount[0] = 100;
        try batch.send(IERC20(address(liar)), to, amount) {
            revert("expected a revert");
        } catch (bytes memory r) {
            revertedWith(r, TokenBatch.NotMoved.selector, "a send that moves nothing is not a send");
        }
        eq(liar.balanceOf(address(this)), 1_000, "the sender kept everything");
    }

    /// Fee tokens stay usable: the sender pays the whole amount even when
    /// the recipient's share is skimmed on arrival.
    function testBatchSendAcceptsFeeOnTransferToken() external {
        FeeOnTransferToken fee = new FeeOnTransferToken();
        fee.mint(address(this), 1_000);
        fee.approve(address(batch), 1_000);
        address[] memory to = new address[](1);
        to[0] = address(0x1);
        uint256[] memory amount = new uint256[](1);
        amount[0] = 100;
        batch.send(IERC20(address(fee)), to, amount);
        eq(fee.balanceOf(address(this)), 900, "the sender paid the whole amount");
        eq(fee.balanceOf(address(0x1)), 99, "the recipient got what the token delivered");
    }

    // ---- the CLI's output, verified on chain ----

    /// The fixture in test/fixtures was built by scripts/merkle-build.mjs; every
    /// claim in it must pay on chain against the root the CLI computed, with no
    /// Solidity-side tree involved at all.
    function testFixtureFromTheCliClaimsOnChain() external {
        bytes memory json = vm.readFile("test/fixtures/merkle.json");
        bytes32 root = hexWord(valueOf(json, "root", 0));
        uint256 totalFixed = decUint(valueOf(json, "total", 0));
        uint256 count = decUint(valueOf(json, "count", 0));
        ok(root != bytes32(0), "a root is there");
        eq(count, 5, "the fixture campaign size");

        token.mint(address(this), totalFixed);
        token.approve(address(factory), totalFixed);
        MerkleDistributor d = factory.create(erc(), root, 0, totalFixed);

        uint256 sum = 0;
        for (uint256 i = 0; i < count; i++) {
            uint256 index = decUint(valueOf(json, "index", i));
            address account = addressOf(valueOf(json, "address", i));
            uint256 amount = decUint(valueOf(json, "amount", i));
            d.claim(index, account, amount, proofOf(json, i));

            // an address can be listed twice: compare against everything it
            // has been paid so far, not just this entry
            uint256 paid = 0;
            for (uint256 j = 0; j <= i; j++) {
                if (addressOf(valueOf(json, "address", j)) == account) paid += decUint(valueOf(json, "amount", j));
            }
            eq(token.balanceOf(account), paid, "the CLI's claim paid");
            sum += amount;
        }
        eq(sum, totalFixed, "the CLI's total is the sum of its claims");
        eq(token.balanceOf(address(d)), 0, "and it exactly drains the campaign");
    }

    // ---- a Solidity mirror of the CLI's tree, for the campaigns above ----

    function leaves(Entry[] memory e) internal pure returns (bytes32[] memory out) {
        out = new bytes32[](e.length);
        for (uint256 i = 0; i < e.length; i++) {
            out[i] = keccak256(abi.encodePacked(e[i].index, e[i].account, e[i].amount));
        }
    }

    function build(bytes32[] memory leafs) internal pure returns (bytes32 root, bytes32[][] memory proofs) {
        proofs = new bytes32[][](leafs.length);
        for (uint256 i = 0; i < leafs.length; i++) proofs[i] = proofFor(leafs, i);
        root = rootOf(leafs);
    }

    function rootOf(bytes32[] memory leafs) private pure returns (bytes32) {
        bytes32[] memory level = leafs;
        while (level.length > 1) level = _up(level);
        return level[0];
    }

    function proofFor(bytes32[] memory leafs, uint256 target) private pure returns (bytes32[] memory proof) {
        bytes32[64] memory path;
        uint256 depth = 0;
        bytes32[] memory level = leafs;
        uint256 at = target;
        while (level.length > 1) {
            path[depth++] = (at ^ 1) < level.length ? level[at ^ 1] : level[at]; // odd level: itself
            level = _up(level);
            at /= 2;
        }
        proof = new bytes32[](depth);
        for (uint256 i = 0; i < depth; i++) proof[i] = path[i];
    }

    function _up(bytes32[] memory level) private pure returns (bytes32[] memory next) {
        next = new bytes32[]((level.length + 1) / 2);
        for (uint256 i = 0; i < level.length; i += 2) {
            bytes32 a = level[i];
            bytes32 b = i + 1 < level.length ? level[i + 1] : a;
            next[i / 2] = a <= b ? keccak256(abi.encodePacked(a, b)) : keccak256(abi.encodePacked(b, a));
        }
    }

    function _dyn(address[3] memory a) internal pure returns (address[] memory out) {
        out = new address[](3);
        for (uint256 i = 0; i < 3; i++) out[i] = a[i];
    }

    function _dyn(uint256[3] memory a) internal pure returns (uint256[] memory out) {
        out = new uint256[](3);
        for (uint256 i = 0; i < 3; i++) out[i] = a[i];
    }

    function _dyn(address[2] memory a) internal pure returns (address[] memory out) {
        out = new address[](2);
        for (uint256 i = 0; i < 2; i++) out[i] = a[i];
    }

    function _dyn(uint256[2] memory a) internal pure returns (uint256[] memory out) {
        out = new uint256[](2);
        for (uint256 i = 0; i < 2; i++) out[i] = a[i];
    }

    function _dyn(uint256[1] memory a) internal pure returns (uint256[] memory out) {
        out = new uint256[](1);
        out[0] = a[0];
    }

    // ---- reading the fixture JSON (flat keys, claims one per line) ----

    function valueOf(bytes memory json, string memory key, uint256 occurrence) internal pure returns (string memory) {
        bytes memory pat = bytes(string.concat('"', key, '":'));
        uint256 at = _find(json, pat, 0);
        for (uint256 i = 0; i < occurrence && at != type(uint256).max; i++) at = _find(json, pat, at + 1);
        require(at != type(uint256).max, "key not in fixture");
        uint256 v = at + pat.length;
        while (json[v] == " ") v++;
        uint256 end = v;
        if (json[v] == '"') {
            v++;
            while (json[end + 1] != '"') end++; // end stays on the closing quote's slot
            end++;
        } else {
            while (json[end] != "," && json[end] != "}" && json[end] != "]") end++;
            while (json[end - 1] == " ") end--; // a bare number (count, index)
        }
        bytes memory out = new bytes(end - v);
        for (uint256 i = 0; i < out.length; i++) out[i] = bytes1(json[v + i]);
        return string(out);
    }

    /// The `n`-th `"proof": [ ... ]` array, as the sibling words it lists.
    function proofOf(bytes memory json, uint256 n) internal pure returns (bytes32[] memory proof) {
        bytes memory pat = bytes('"proof":[');
        uint256 at = _find(json, pat, 0);
        for (uint256 i = 0; i < n && at != type(uint256).max; i++) at = _find(json, pat, at + 1);
        require(at != type(uint256).max, "proof not in fixture");
        uint256 quotes = 0;
        uint256 p = at + pat.length;
        while (json[p] != "]") {
            if (json[p] == '"') quotes++;
            p++;
        }
        proof = new bytes32[](quotes / 2); // each word opens and closes one quote
        p = at + pat.length;
        for (uint256 i = 0; i < proof.length; i++) {
            while (json[p] != '"') p++;
            require(json[p + 1] == "0" && json[p + 2] == "x", "word is not 0x");
            p += 3; // onto the first hex digit
            bytes32 word;
            for (uint256 b = 0; b < 64; b++) word = bytes32(uint256(word) << 4 | _hex(json[p + b]));
            proof[i] = word;
            p += 65; // past the word and its closing quote, onto the next scan
        }
    }

    function _find(bytes memory hay, bytes memory needle, uint256 from) private pure returns (uint256) {
        for (uint256 i = from; i + needle.length <= hay.length; i++) {
            bool same = true;
            for (uint256 j = 0; j < needle.length; j++) {
                if (hay[i + j] != needle[j]) {
                    same = false;
                    break;
                }
            }
            if (same) return i;
        }
        return type(uint256).max;
    }

    function _hex(bytes1 c) private pure returns (uint256) {
        uint8 v = uint8(c);
        require((v >= 48 && v <= 57) || (v >= 97 && v <= 102), "not a hex digit");
        return v <= 57 ? v - 48 : v - 87;
    }

    function decUint(string memory s) internal pure returns (uint256 v) {
        bytes memory b = bytes(s);
        for (uint256 i = 0; i < b.length; i++) {
            uint8 c = uint8(b[i]);
            require(c >= 48 && c <= 57, "not a decimal");
            v = v * 10 + (c - 48);
        }
    }

    function hexWord(string memory s) internal pure returns (bytes32 word) {
        bytes memory b = bytes(s);
        require(b.length == 66 && b[0] == "0" && b[1] == "x", "not a 0x word");
        for (uint256 i = 2; i < 66; i++) word = bytes32(uint256(word) << 4 | _hex(b[i]));
    }

    function addressOf(string memory s) internal pure returns (address) {
        bytes memory b = bytes(s);
        require(b.length == 42 && b[0] == "0" && b[1] == "x", "not a 0x address");
        uint160 v = 0;
        for (uint256 i = 2; i < 42; i++) v = uint160(uint256(v) << 4 | _hex(b[i]));
        return address(v);
    }
}
