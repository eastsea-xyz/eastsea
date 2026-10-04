// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.19;

import {AetherNames} from "../src/AetherNames.sol";

interface Vm {
    function warp(uint256) external;
    function deal(address, uint256) external;
    function prank(address) external;
    function expectEmit(bool, bool, bool, bool) external;
    function expectRevert(bytes calldata) external;
}

/// A payer whose receive() calls back into the names contract while its
/// refund is in flight. `armCall` re-sends an arbitrary raw call (zero
/// value); `armRegister` registers a second, separately committed name with
/// real value. Both modes record whether the inner call landed.
contract ReentrantPayer {
    AetherNames public names;
    bytes public callData;
    string public rName;
    address public rOwner;
    bytes32 public rSalt;
    uint256 public rValue;
    bool public armedCall;
    bool public armedRegister;
    uint256 public innerOk; // 1 = the re-entrant call succeeded

    constructor(AetherNames n) {
        names = n;
    }

    function armCall(bytes calldata c) external {
        callData = c;
        armedCall = true;
    }

    function armRegister(string calldata n, address o, bytes32 s, uint256 v) external {
        rName = n;
        rOwner = o;
        rSalt = s;
        rValue = v;
        armedRegister = true;
    }

    function go(string calldata n, address o, bytes32 s) external payable {
        names.register{value: msg.value}(n, o, s);
    }

    receive() external payable {
        if (armedCall) {
            armedCall = false;
            (bool ok,) = address(names).call(callData);
            if (ok) innerOk = 1;
        } else if (armedRegister) {
            armedRegister = false;
            try names.register{value: rValue}(rName, rOwner, rSalt) {
                innerOk = 1;
            } catch {
                innerOk = 2;
            }
        }
    }
}

/// A contract owner. The names contract never calls its owners, so `touched`
/// staying false through a full lifecycle is the proof: owning a name grants
/// no callback surface to attack.
contract StubOwner {
    AetherNames public names;
    bool public touched;

    constructor(AetherNames n) {
        names = n;
    }

    function propose(string calldata n, address to) external {
        names.transferPropose(n, to);
    }

    function setAddr(string calldata n, address a) external {
        names.setAddr(n, a);
    }

    fallback() external payable {
        touched = true;
    }
}

contract AetherNamesTest {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    address alice = address(0xA11CE);
    address bob = address(0xB0B);
    address carol = address(0xC0);
    address eve = address(0xE1E);

    bytes32 constant SALT = bytes32(uint256(0xa11ce));

    AetherNames names;

    event Registered(string name, bytes32 indexed node, address indexed owner, uint64 expires, uint256 fee);
    event Renewed(bytes32 indexed node, uint64 newExpires, uint256 fee);
    event Burned(uint256 amount);
    event TransferProposed(bytes32 indexed node, address indexed from, address indexed to);
    event TransferAccepted(bytes32 indexed node, address indexed from, address indexed to);
    event AddrSet(bytes32 indexed node, address indexed addr);
    event TextSet(bytes32 indexed node, string key, string value);
    event ReverseSet(address indexed account, bytes32 indexed node, string name);

    // ---- the stateful fuzz model (see testFuzz_StateInvariants) ----

    string[6] pool = ["abc", "abcd", "abcde", "a-b-c", "xn--x", "abcdefghijklmnopqrstuvwxyz123456"];
    address[3] payers;
    mapping(bytes32 => address) mOwner;
    mapping(bytes32 => uint64) mExpires;
    mapping(bytes32 => address) mPending;
    mapping(bytes32 => address) mAddr;
    uint256 mFees;

    function setUp() public {
        names = new AetherNames();
        vm.deal(alice, 1000 ether);
        vm.deal(bob, 1000 ether);
        vm.deal(carol, 1000 ether);
        vm.deal(eve, 1000 ether);
        vm.deal(address(this), 1000 ether);
        // mapping state survives setUp, so the fuzz model resets by hand
        payers[0] = alice;
        payers[1] = bob;
        payers[2] = carol;
        for (uint256 i = 0; i < pool.length; i++) {
            bytes32 node = names.nodeFor(pool[i]);
            delete mOwner[node];
            delete mExpires[node];
            delete mPending[node];
            delete mAddr[node];
        }
        mFees = 0;
    }

    // ---- helpers ----

    function assertEq(uint256 a, uint256 b) internal pure {
        if (a != b) revert("uint256 mismatch");
    }

    function assertEq(address a, address b) internal pure {
        if (a != b) revert("address mismatch");
    }

    function assertTrue(bool c) internal pure {
        if (!c) revert("expected true");
    }

    function err(bytes4 e) internal pure returns (bytes memory) {
        return abi.encodeWithSelector(e);
    }

    function commitFor(string memory name, address owner, bytes32 salt) internal view returns (bytes32) {
        return keccak256(abi.encodePacked(name, owner, salt));
    }

    /// Commit + age the commitment, so the caller can reveal right away.
    function commitAndAge(string memory name, address owner, bytes32 salt) internal {
        names.commit(commitFor(name, owner, salt));
        vm.warp(block.timestamp + names.MIN_COMMIT_AGE());
    }

    /// The independently written grammar reference for the fuzz test.
    function refValid(bytes memory b) private pure returns (bool) {
        if (b.length < 3 || b.length > 32) return false;
        bool charsOk = true;
        for (uint256 i = 0; i < b.length; i++) {
            bytes1 c = b[i];
            bool ok = (c >= 0x61 && c <= 0x7a) || (c >= 0x30 && c <= 0x39) || c == 0x2d;
            charsOk = charsOk && ok;
        }
        if (!charsOk) return false;
        if (b[0] == 0x2d || b[b.length - 1] == 0x2d) return false;
        if (b.length >= 4 && b[2] == 0x2d && b[3] == 0x2d) return false;
        return true;
    }

    function mLive(bytes32 node) private view returns (bool) {
        return mOwner[node] != address(0) && block.timestamp < uint256(mExpires[node]) + names.GRACE_PERIOD();
    }

    // ---- name grammar ----

    function test_NameGrammarRules() public {
        string[22] memory samples = [
            "abc",
            "007",
            "a-9",
            "a--b", // double hyphen at 2-3: only positions 3-4 are banned
            "a-b-c",
            "abcdefghijabcdefghijabcdefghij12", // 32 chars
            "ab",
            "abcdefghijabcdefghijabcdefghij123", // 33 chars
            "-ab",
            "ab-",
            "-a-",
            "ab--c", // double hyphen at 3-4: the punycode shape
            "xn--pay",
            "xn--",
            "Abc",
            "aBc",
            "ab c",
            "a.b",
            "a_b",
            "",
            "a",
            "ab\x80"
        ];
        bool[22] memory expected = [
            true,
            true,
            true,
            true,
            true,
            true,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false
        ];
        for (uint256 i = 0; i < samples.length; i++) {
            bool got = names.isValidName(samples[i]);
            if (got != expected[i]) revert("grammar table mismatch");
            bool ref = refValid(bytes(samples[i]));
            if (ref != expected[i]) revert("reference disagrees with the table");
        }
    }

    function test_InvalidNameRejectedAtRegister() public {
        vm.prank(alice);
        names.commit(commitFor("xn--pay", alice, SALT));
        vm.warp(block.timestamp + names.MIN_COMMIT_AGE());
        vm.prank(alice);
        vm.expectRevert(err(AetherNames.InvalidName.selector));
        names.register{value: 10 ether}("xn--pay", alice, SALT);
    }

    function test_ZeroOwnerRejectedAtRegister() public {
        vm.prank(alice);
        names.commit(commitFor("abc", address(0), SALT));
        vm.warp(block.timestamp + names.MIN_COMMIT_AGE());
        vm.prank(alice);
        vm.expectRevert(err(AetherNames.InvalidOwner.selector));
        names.register{value: 10 ether}("abc", address(0), SALT);
    }

    // ---- fees: fixed by length, burned, overpayment refunded ----

    function test_FeeBuckets() public {
        assertEq(names.FEE_3(), 2 ether);
        assertEq(names.FEE_4(), 0.5 ether);
        assertEq(names.FEE_5_PLUS(), 0.1 ether);
        assertEq(names.feeFor("abc"), 2 ether);
        assertEq(names.feeFor("abcd"), 0.5 ether);
        assertEq(names.feeFor("abcde"), 0.1 ether);
        assertEq(names.feeFor("abcdefghijabcdefghijabcdefghij12"), 0.1 ether);
    }

    function test_RegisterBurnsExactFeeAndRefundsOverpayment() public {
        vm.warp(1000);
        vm.prank(alice);
        names.commit(commitFor("abc", alice, SALT));
        vm.warp(1060);
        uint256 before = alice.balance;
        vm.prank(alice);
        names.register{value: 2.3 ether}("abc", alice, SALT);
        assertEq(alice.balance, before - 2 ether);
        assertEq(names.BURN_ADDRESS().balance, 2 ether);
        assertEq(names.totalBurned(), 2 ether);
        assertEq(address(names).balance, 0);
    }

    function test_RegisterUnderpayReverts() public {
        vm.prank(alice);
        names.commit(commitFor("abc", alice, SALT));
        vm.warp(block.timestamp + names.MIN_COMMIT_AGE());
        vm.prank(alice);
        vm.expectRevert(abi.encodeWithSelector(AetherNames.InsufficientFee.selector, 2 ether));
        names.register{value: 1.9 ether}("abc", alice, SALT);
    }

    // ---- commit-reveal ----

    function test_RegisterRequiresMatchingCommitment() public {
        vm.prank(alice);
        vm.expectRevert(err(AetherNames.UnknownCommitment.selector));
        names.register{value: 2 ether}("abc", alice, SALT);

        // a commitment over different parameters does not help
        vm.prank(alice);
        names.commit(commitFor("abcd", alice, SALT));
        vm.warp(block.timestamp + names.MIN_COMMIT_AGE());
        vm.prank(alice);
        vm.expectRevert(err(AetherNames.UnknownCommitment.selector));
        names.register{value: 2 ether}("abc", alice, SALT); // wrong name in hash
        vm.prank(alice);
        vm.expectRevert(err(AetherNames.UnknownCommitment.selector));
        names.register{value: 0.5 ether}("abcd", bob, SALT); // wrong owner in hash
    }

    function test_CommitAgeWindow() public {
        uint256 t0 = block.timestamp;
        vm.prank(alice);
        names.commit(commitFor("abc", alice, SALT));

        uint256 minAge = names.MIN_COMMIT_AGE();
        vm.warp(t0 + minAge - 1); // one second too early
        vm.prank(alice);
        vm.expectRevert(abi.encodeWithSelector(AetherNames.CommitTooNew.selector, minAge - 1));
        names.register{value: 2 ether}("abc", alice, SALT);

        vm.warp(t0 + names.MIN_COMMIT_AGE()); // exactly old enough
        vm.prank(alice);
        names.register{value: 2 ether}("abc", alice, SALT);
        assertEq(names.ownerOf(names.nodeFor("abc")), alice);

        // the other edge: a commitment expires after 24 h
        vm.prank(bob);
        names.commit(commitFor("abd", bob, SALT));
        vm.warp(block.timestamp + names.MAX_COMMIT_AGE() - 1); // a second to spare
        vm.prank(bob);
        names.register{value: 2 ether}("abd", bob, SALT); // still revealable

        vm.prank(bob);
        names.commit(commitFor("abe", bob, SALT));
        vm.warp(block.timestamp + names.MAX_COMMIT_AGE()); // exactly too old
        vm.prank(bob);
        vm.expectRevert(err(AetherNames.CommitTooOld.selector));
        names.register{value: 2 ether}("abe", bob, SALT);

        // re-committing the same hash refreshes the window
        vm.prank(bob);
        names.commit(commitFor("abe", bob, SALT));
        vm.warp(block.timestamp + names.MIN_COMMIT_AGE());
        vm.prank(bob);
        names.register{value: 2 ether}("abe", bob, SALT);
        assertEq(names.ownerOf(names.nodeFor("abe")), bob);
    }

    function test_CommitmentSpentOnRegister() public {
        commitAndAge("abc", alice, SALT);
        vm.prank(alice);
        names.register{value: 2 ether}("abc", alice, SALT);
        // the same commitment is deleted after the reveal: a replay fails
        // before the name is even checked
        vm.prank(alice);
        vm.expectRevert(err(AetherNames.UnknownCommitment.selector));
        names.register{value: 2 ether}("abc", alice, SALT);
    }

    function test_FrontRunCannotStealPendingRegistration() public {
        uint256 t0 = block.timestamp;
        vm.prank(alice);
        names.commit(commitFor("abc", alice, SALT)); // the hash hides the name
        vm.warp(t0 + names.MIN_COMMIT_AGE());

        // eve watches the mempool at reveal time and tries to copy it:
        // the victim's salt does not open a commitment bound to eve
        vm.prank(eve);
        vm.expectRevert(err(AetherNames.UnknownCommitment.selector));
        names.register{value: 2 ether}("abc", eve, SALT);

        // committing her own hash now and revealing in the same breath is
        // too fast: the commitment must age first, and the victim's reveal
        // (already aged) lands in between
        vm.prank(eve);
        names.commit(commitFor("abc", eve, bytes32(uint256(0xE1E))));
        vm.prank(eve);
        vm.expectRevert(abi.encodeWithSelector(AetherNames.CommitTooNew.selector, 0));
        names.register{value: 2 ether}("abc", eve, bytes32(uint256(0xE1E)));

        vm.prank(alice);
        names.register{value: 2 ether}("abc", alice, SALT);
        assertEq(names.ownerOf(names.nodeFor("abc")), alice);
    }

    // ---- registration ----

    function test_RegisterHappyPath() public {
        vm.warp(5000);
        commitAndAge("abc", address(this), SALT);
        bytes32 node = names.nodeFor("abc");
        // computed first: expectEmit latches onto the very next call
        uint64 exp = uint64(5000 + names.MIN_COMMIT_AGE() + 365 days);
        vm.expectEmit(true, true, true, true);
        emit Registered("abc", node, address(this), exp, 2 ether);
        names.register{value: 2 ether}("abc", address(this), SALT);
        assertEq(names.ownerOf(node), address(this));
        assertEq(uint256(names.expiresOf(node)), 5000 + names.MIN_COMMIT_AGE() + 365 days);
        assertEq(names.addrOf(node), address(0));
        assertEq(names.pendingOwnerOf(node), address(0));
    }

    function test_RegisterForAnotherAddress() public {
        commitAndAge("abcde", carol, SALT);
        vm.prank(alice); // the payer is not the owner
        names.register{value: 0.1 ether}("abcde", carol, SALT);
        bytes32 node = names.nodeFor("abcde");
        assertEq(names.ownerOf(node), carol);

        // only carol controls the name now
        vm.prank(alice);
        vm.expectRevert(err(AetherNames.NotOwner.selector));
        names.setAddr("abcde", alice);
    }

    function test_RegisterTakenName() public {
        commitAndAge("abc", alice, SALT);
        vm.prank(alice);
        names.register{value: 2 ether}("abc", alice, SALT);

        commitAndAge("abc", bob, SALT);
        vm.prank(bob);
        vm.expectRevert(err(AetherNames.NameTaken.selector));
        names.register{value: 2 ether}("abc", bob, SALT);
    }

    // ---- expiry: one year, 30-day grace, then release ----

    function test_ExpiryBoundaries() public {
        vm.warp(10_000);
        bytes32 node = names.nodeFor("abcde");
        commitAndAge("abcde", alice, SALT);
        vm.prank(alice);
        names.register{value: 0.1 ether}("abcde", alice, SALT);
        vm.prank(alice);
        names.setAddr("abcde", alice);
        uint64 e0 = names.expiresOf(node);
        assertEq(uint256(e0), block.timestamp + 365 days);

        vm.warp(uint256(e0) - 1); // the last second of the paid year
        assertEq(names.ownerOf(node), alice);
        assertEq(names.addrOf(node), alice);

        vm.warp(uint256(e0)); // expired: grace begins, still renewable
        assertEq(names.ownerOf(node), alice);
        assertEq(names.addrOf(node), alice);

        vm.warp(uint256(e0) + names.GRACE_PERIOD() - 1); // last grace second
        assertEq(names.ownerOf(node), alice);
        vm.prank(alice);
        names.renew{value: 0.1 ether}("abcde");
        assertEq(uint256(names.expiresOf(node)), uint256(e0) + 365 days);

        // a separate name rides all the way to release
        bytes32 gone = names.nodeFor("abcdef");
        commitAndAge("abcdef", alice, SALT);
        vm.prank(alice);
        names.register{value: 0.1 ether}("abcdef", alice, SALT);
        uint64 g0 = names.expiresOf(gone);
        vm.warp(uint256(g0) + names.GRACE_PERIOD()); // released
        assertEq(names.ownerOf(gone), address(0));
        assertEq(names.addrOf(gone), address(0));
        vm.prank(alice);
        vm.expectRevert(err(AetherNames.Released.selector));
        names.renew{value: 0.1 ether}("abcdef");
        vm.prank(alice);
        vm.expectRevert(err(AetherNames.NotOwner.selector));
        names.setAddr("abcdef", alice); // released: nobody owns it
    }

    function test_ReleasedNameRegistersFreshAndKeepsNoStaleData() public {
        vm.warp(20_000);
        bytes32 node = names.nodeFor("abcde");
        commitAndAge("abcde", alice, SALT);
        vm.prank(alice);
        names.register{value: 0.1 ether}("abcde", alice, SALT);
        vm.prank(alice);
        names.setAddr("abcde", alice);
        vm.prank(alice);
        names.setText("abcde", "note", "alice was here");
        vm.prank(alice);
        names.setReverse("abcde"); // reverse[alice] = node

        vm.warp(uint256(names.expiresOf(node)) + names.GRACE_PERIOD());
        commitAndAge("abcde", bob, SALT); // time passes; commitment ages
        vm.prank(bob);
        names.register{value: 0.1 ether}("abcde", bob, SALT);

        assertEq(names.ownerOf(node), bob);
        assertEq(names.addrOf(node), address(0)); // no stale resolver data
        assertTrue(bytes(names.textOf(node, "note")).length == 0);
        assertTrue(bytes(names.reverseOf(alice)).length == 0); // and no stale reverse
        assertEq(names.pendingOwnerOf(node), address(0));
    }

    // ---- renewal: anyone, before expiry and during grace ----

    function test_RenewalByAnyoneExtendsFromExpiry() public {
        vm.warp(30_000);
        bytes32 node = names.nodeFor("abcdef");
        commitAndAge("abcdef", alice, SALT);
        vm.prank(alice);
        names.register{value: 0.1 ether}("abcdef", alice, SALT);
        uint64 e0 = names.expiresOf(node);

        // a stranger pays: ownership does not move, expiry does
        uint256 before = names.BURN_ADDRESS().balance;
        vm.prank(carol);
        names.renew{value: 0.2 ether}("abcdef");
        assertEq(uint256(names.expiresOf(node)), uint256(e0) + 365 days); // from the old expiry, not from now
        assertEq(names.ownerOf(node), alice);
        assertEq(names.BURN_ADDRESS().balance, before + 0.1 ether);
        assertEq(names.totalBurned(), 0.2 ether);
        assertEq(address(names).balance, 0);

        // during grace the renewal still extends from the expiry date:
        // lapsed grace time is the owner's loss
        vm.warp(uint256(names.expiresOf(node)) + 10 days);
        vm.prank(alice);
        names.renew{value: 0.1 ether}("abcdef");
        assertEq(uint256(names.expiresOf(node)), uint256(e0) + 2 * 365 days);
    }

    function test_RenewUnknownName() public {
        vm.prank(alice);
        vm.expectRevert(err(AetherNames.Unregistered.selector));
        names.renew{value: 0.1 ether}("abcde");
    }

    // ---- transfer: propose then accept ----

    function test_TransferTwoStep() public {
        vm.warp(40_000);
        bytes32 node = names.nodeFor("abcde");
        commitAndAge("abcde", alice, SALT);
        vm.prank(alice);
        names.register{value: 0.1 ether}("abcde", alice, SALT);

        vm.prank(alice);
        names.transferPropose("abcde", bob);
        assertEq(names.pendingOwnerOf(node), bob);
        assertEq(names.ownerOf(node), alice); // not moved yet

        vm.expectEmit(true, true, true, true);
        emit TransferAccepted(node, alice, bob);
        vm.prank(bob);
        names.transferAccept("abcde");
        assertEq(names.ownerOf(node), bob);
        assertEq(names.pendingOwnerOf(node), address(0));

        // alice lost every owner right with the transfer
        vm.prank(alice);
        vm.expectRevert(err(AetherNames.NotOwner.selector));
        names.setAddr("abcde", alice);
    }

    function test_TransferGuards() public {
        bytes32 node = names.nodeFor("abcde");
        commitAndAge("abcde", alice, SALT);
        vm.prank(alice);
        names.register{value: 0.1 ether}("abcde", alice, SALT);

        // only the owner may propose
        vm.prank(bob);
        vm.expectRevert(err(AetherNames.NotOwner.selector));
        names.transferPropose("abcde", bob);

        // only the pending owner may accept
        vm.prank(alice);
        names.transferPropose("abcde", bob);
        vm.prank(carol);
        vm.expectRevert(err(AetherNames.NotPendingOwner.selector));
        names.transferAccept("abcde");

        // proposing zero cancels
        vm.prank(alice);
        names.transferPropose("abcde", address(0));
        assertEq(names.pendingOwnerOf(node), address(0));
        vm.prank(bob);
        vm.expectRevert(err(AetherNames.NotPendingOwner.selector));
        names.transferAccept("abcde");

        // a new proposal overwrites the old one
        vm.prank(alice);
        names.transferPropose("abcde", bob);
        vm.prank(alice);
        names.transferPropose("abcde", carol);
        vm.prank(bob);
        vm.expectRevert(err(AetherNames.NotPendingOwner.selector));
        names.transferAccept("abcde");
        vm.prank(carol);
        names.transferAccept("abcde");
        assertEq(names.ownerOf(node), carol);
    }

    // ---- the address record ----

    function test_SetAddrOwnerOnly() public {
        commitAndAge("abcde", alice, SALT);
        vm.prank(alice);
        names.register{value: 0.1 ether}("abcde", alice, SALT);
        bytes32 node = names.nodeFor("abcde");

        vm.prank(bob);
        vm.expectRevert(err(AetherNames.NotOwner.selector));
        names.setAddr("abcde", bob);

        vm.expectEmit(true, true, true, true);
        emit AddrSet(node, carol);
        vm.prank(alice);
        names.setAddr("abcde", carol);
        assertEq(names.addrOf(node), carol);
    }

    // ---- text records: bounded count, bounded sizes ----

    function test_TextRecordBounds() public {
        commitAndAge("abcde", alice, SALT);
        vm.prank(alice);
        names.register{value: 0.1 ether}("abcde", alice, SALT);
        bytes32 node = names.nodeFor("abcde");

        vm.prank(alice);
        names.setText("abcde", "email", "a@b.co");
        vm.prank(alice);
        names.setText("abcde", "url", "https://a.eth");
        vm.prank(alice);
        names.setText("abcde", "avatar", "ipfs://q");
        vm.prank(alice);
        names.setText("abcde", "note", "x");

        // the fifth distinct key does not fit
        vm.prank(alice);
        vm.expectRevert(err(AetherNames.TooManyTextRecords.selector));
        names.setText("abcde", "fifth", "y");

        // rewriting an existing key costs no slot
        vm.prank(alice);
        names.setText("abcde", "note", "rewritten");
        assertTrue(keccak256(bytes(names.textOf(node, "note"))) == keccak256("rewritten"));

        // deleting frees the slot again
        vm.prank(alice);
        names.setText("abcde", "note", "");
        assertTrue(bytes(names.textOf(node, "note")).length == 0);
        vm.prank(alice);
        names.setText("abcde", "fifth", "now it fits");

        // deleting a key that is not set is a harmless no-op
        vm.prank(alice);
        names.setText("abcde", "ghost", "");
    }

    function test_TextRecordValidation() public {
        commitAndAge("abcde", alice, SALT);
        vm.prank(alice);
        names.register{value: 0.1 ether}("abcde", alice, SALT);

        vm.prank(alice);
        vm.expectRevert(err(AetherNames.BadTextKey.selector));
        names.setText("abcde", "", "v"); // empty key

        vm.prank(alice);
        vm.expectRevert(err(AetherNames.BadTextKey.selector));
        names.setText("abcde", "abcdefghijklmnopqrstuvwxyz1234567", "v"); // 33 bytes

        vm.prank(alice);
        vm.expectRevert(err(AetherNames.BadTextKey.selector));
        names.setText("abcde", "Bad_Key", "v"); // charset is [a-z0-9-]

        vm.prank(alice);
        vm.expectRevert(err(AetherNames.TextValueTooLong.selector));
        names.setText("abcde", "k", rep("1", 129)); // one byte over

        // 128 bytes on the nose is fine
        vm.prank(alice);
        names.setText("abcde", "k", rep("1", 128));

        // owner-only, and the name must still be live
        vm.prank(bob);
        vm.expectRevert(err(AetherNames.NotOwner.selector));
        names.setText("abcde", "k", "v");
    }

    // ---- reverse records: only with a forward record pointing back ----

    function test_ReverseRequiresPointback() public {
        commitAndAge("abcde", alice, SALT);
        vm.prank(alice);
        names.register{value: 0.1 ether}("abcde", alice, SALT);
        bytes32 node = names.nodeFor("abcde");

        // no address record yet: nothing points back
        vm.prank(alice);
        vm.expectRevert(err(AetherNames.ReverseMismatch.selector));
        names.setReverse("abcde");

        vm.prank(alice);
        names.setAddr("abcde", alice);
        vm.prank(alice);
        names.setReverse("abcde");
        assertTrue(keccak256(bytes(names.reverseOf(alice))) == keccak256("abcde"));

        // moving the forward record invalidates the old reverse claim
        vm.prank(alice);
        names.setAddr("abcde", bob);
        assertTrue(bytes(names.reverseOf(alice)).length == 0);
        assertTrue(bytes(names.reverseOf(bob)).length == 0); // bob never claimed
    }

    function test_ReverseCannotClaimSomeoneElsAddress() public {
        commitAndAge("abcde", bob, SALT);
        vm.prank(bob);
        names.register{value: 0.1 ether}("abcde", bob, SALT);

        // bob points the name at alice's address...
        vm.prank(bob);
        names.setAddr("abcde", alice);

        // ...but neither can pin the name on alice's address: bob is not
        // alice (addr != msg.sender), and alice does not own the name
        vm.prank(bob);
        vm.expectRevert(err(AetherNames.ReverseMismatch.selector));
        names.setReverse("abcde");
        vm.prank(alice);
        vm.expectRevert(err(AetherNames.NotOwner.selector));
        names.setReverse("abcde");
        assertTrue(bytes(names.reverseOf(alice)).length == 0);

        // once alice owns the name, she may claim it herself
        vm.prank(bob);
        names.transferPropose("abcde", alice);
        vm.prank(alice);
        names.transferAccept("abcde");
        vm.prank(alice);
        names.setReverse("abcde");
        assertTrue(keccak256(bytes(names.reverseOf(alice))) == keccak256("abcde"));
    }

    function test_ReverseSurvivesTransferUntilForwardMoves() public {
        commitAndAge("abcde", alice, SALT);
        vm.prank(alice);
        names.register{value: 0.1 ether}("abcde", alice, SALT);
        vm.prank(alice);
        names.setAddr("abcde", alice);
        vm.prank(alice);
        names.setReverse("abcde");

        // the name still points at alice, so her claim stays honest
        vm.prank(alice);
        names.transferPropose("abcde", bob);
        vm.prank(bob);
        names.transferAccept("abcde");
        assertTrue(keccak256(bytes(names.reverseOf(alice))) == keccak256("abcde"));

        // the new owner redirects the name: the stale claim goes quiet
        vm.prank(bob);
        names.setAddr("abcde", bob);
        assertTrue(bytes(names.reverseOf(alice)).length == 0);
    }

    function test_ReverseOfUnclaimedAddress() public view {
        assertTrue(bytes(names.reverseOf(alice)).length == 0);
        assertTrue(bytes(names.reverseOf(address(0x1234))).length == 0);
    }

    // ---- reentrancy ----

    function test_ReentrantRefundCannotDoubleRegister() public {
        ReentrantPayer payer = new ReentrantPayer(names);
        vm.deal(address(payer), 100 ether);
        commitAndAge("abc", address(payer), SALT);
        bytes32 node = names.nodeFor("abc");

        // during the refund, the payer tries to register "abc" again with
        // different parameters (zero value, so InsufficientFee at best —
        // and UnknownCommitment anyway, the commitment was deleted)
        payer.armCall(abi.encodeWithSelector(names.register.selector, "abc", address(0xBAD), bytes32(uint256(0x1))));

        payer.go{value: 3 ether}("abc", address(payer), SALT);

        assertEq(names.ownerOf(node), address(payer)); // owner unchanged
        assertEq(names.totalBurned(), 2 ether); // exactly one fee
        assertEq(names.BURN_ADDRESS().balance, 2 ether);
        // the test sent 3 in, 2 burned, 1 refunded: 100 + 3 - 3 + 1
        assertEq(address(payer).balance, 101 ether);
        assertTrue(payer.innerOk() != 1); // the re-entrant register did not land
    }

    function test_ReentrantRefundLegitimateSecondRegister() public {
        ReentrantPayer payer = new ReentrantPayer(names);
        vm.deal(address(payer), 100 ether);
        commitAndAge("abc", address(payer), SALT);
        commitAndAge("abd", address(payer), SALT); // a second, aged commitment

        // during the first refund, the payer registers the second name with
        // its own value — a legitimate registration, fee and all
        payer.armRegister("abd", address(payer), SALT, 2 ether);
        payer.go{value: 2.5 ether}("abc", address(payer), SALT);

        assertEq(names.ownerOf(names.nodeFor("abc")), address(payer));
        assertEq(names.ownerOf(names.nodeFor("abd")), address(payer));
        assertEq(payer.innerOk(), 1);
        assertEq(names.totalBurned(), 4 ether); // two fees, no more
        assertEq(names.BURN_ADDRESS().balance, 4 ether);
        // 100 + 2.5 in - 2.5 out + 0.5 outer refund - 2 inner fee + 0 inner refund
        assertEq(address(payer).balance, 98.5 ether);
        assertEq(address(names).balance, 0);
    }

    function test_MaliciousOwnerContractIsInert() public {
        StubOwner stub = new StubOwner(names);
        commitAndAge("abc", address(stub), SALT);
        vm.prank(alice); // alice pays; the stub owns
        names.register{value: 2.3 ether}("abc", address(stub), SALT);

        assertEq(names.ownerOf(names.nodeFor("abc")), address(stub));

        // a contract owner acts through its own calls, exactly like an
        // AetherAccount would
        stub.setAddr("abc", address(stub));
        assertEq(names.addrOf(names.nodeFor("abc")), address(stub));
        stub.propose("abc", bob);
        vm.prank(bob);
        names.transferAccept("abc");
        assertEq(names.ownerOf(names.nodeFor("abc")), bob);

        // the contract never called back into the owner: nothing was touched
        assertTrue(!stub.touched());
    }

    // ---- fuzz: the grammar ----

    /// A string of n repeated bytes, for exact-size values.
    function rep(bytes1 c, uint256 n) internal pure returns (string memory) {
        bytes memory b = new bytes(n);
        for (uint256 i = 0; i < n; i++) b[i] = c;
        return string(b);
    }

    /// Clean alphabet: hyphen weighted x3 so the positional rules get hit.
    bytes constant CLEAN = "abcdefghijklmnopqrstuvwxyz0123456789--";
    /// Dirty alphabet: every third run draws from this, mixing invalid bytes.
    bytes constant DIRTY = "abcdefghijklmnopqrstuvwxyz0123456789--A._ \x80\x7f";

    /// Never accepts a name outside the grammar, never rejects one inside:
    /// the contract must agree with the independently written refValid on
    /// every generated string (length 2-36, hyphen-heavy, sometimes dirty).
    function testFuzz_NameValidationMatchesSpec(uint256 seed) public view {
        uint256 s = seed;
        uint256 len = 2 + (s % 35); // covers below 3 and beyond 32
        s = s / 35;
        bytes memory alphabet = (s % 3 == 0) ? DIRTY : CLEAN;
        s = s / 3;
        bytes memory b = new bytes(len);
        for (uint256 i = 0; i < len; i++) {
            b[i] = alphabet[s % alphabet.length];
            s = s / alphabet.length;
        }
        assertTrue(names.isValidName(string(b)) == refValid(b));
    }

    // ---- fuzz: stateful invariants ----

    /// A random interleaving of every operation, mirrored in a test-side
    /// model. After the run: the burned total equals the sum of fees (and
    /// equals the burn address balance), every name's owner matches the
    /// model (a name has at most one owner, exactly the expected one), and
    /// every reverse claim the views still honor points back at a live
    /// forward record.
    function testFuzz_StateInvariants(uint256 seed) public {
        uint256 s = seed;
        uint256 ops = 1 + (s % 10);
        s = s / 10;
        for (uint256 n = 0; n < ops; n++) {
            uint256 op = s % 8;
            s = s / 8;
            uint256 ni = s % pool.length;
            s = s / pool.length;
            uint256 ai = s % 3;
            s = s / 3;
            string memory nm = pool[ni];
            bytes32 node = names.nodeFor(nm);
            address who = payers[ai];
            if (op == 0) {
                // commit, age, register to a random owner, sometimes overpay
                address owner = payers[s % 3];
                s = s / 3;
                bytes32 salt = bytes32(s);
                names.commit(commitFor(nm, owner, salt));
                vm.warp(block.timestamp + names.MIN_COMMIT_AGE() + 1);
                vm.deal(who, who.balance + 5 ether);
                uint256 value = names.feeFor(nm) + (s % 2) * 0.07 ether;
                s = s / 2;
                vm.prank(who);
                try names.register{value: value}(nm, owner, salt) {
                    mOwner[node] = owner;
                    mExpires[node] = names.expiresOf(node);
                    delete mPending[node];
                    delete mAddr[node];
                    mFees += names.feeFor(nm);
                } catch {}
            } else if (op == 1) {
                vm.deal(who, who.balance + 5 ether);
                vm.prank(who);
                try names.renew{value: names.feeFor(nm)}(nm) {
                    mExpires[node] = names.expiresOf(node);
                    mFees += names.feeFor(nm);
                } catch {}
            } else if (op == 2) {
                vm.warp(block.timestamp + 1 + (s % 40 days));
                s = s / 40 days;
            } else if (op == 3) {
                if (mLive(node)) {
                    address to = payers[s % 3];
                    s = s / 3;
                    vm.prank(mOwner[node]);
                    try names.transferPropose(nm, to) {
                        mPending[node] = to;
                    } catch {}
                }
            } else if (op == 4) {
                if (mLive(node) && mPending[node] != address(0)) {
                    address po = mPending[node];
                    vm.prank(po);
                    try names.transferAccept(nm) {
                        mOwner[node] = po;
                        delete mPending[node];
                    } catch {}
                }
            } else if (op == 5) {
                if (mLive(node)) {
                    address a = payers[s % 3];
                    s = s / 3;
                    vm.prank(mOwner[node]);
                    try names.setAddr(nm, a) {
                        mAddr[node] = a;
                    } catch {}
                }
            } else if (op == 6) {
                // setReverse: the caller must own the name and be its addr
                if (mLive(node) && mOwner[node] != address(0) && mOwner[node] == mAddr[node]) {
                    vm.prank(mOwner[node]);
                    try names.setReverse(nm) {} catch {}
                }
            } else {
                // setText: bounded shapes; no invariant rides on texts
                if (mLive(node)) {
                    vm.prank(mOwner[node]);
                    try names.setText(nm, "note", "ok") {} catch {}
                }
            }
        }

        // 1. every fee ever charged is burned — no more, no less
        assertEq(names.totalBurned(), mFees);
        assertEq(names.BURN_ADDRESS().balance, mFees);
        // 2. ownership matches the model exactly (so at most one owner per
        //    name, and released names show zero)
        for (uint256 i = 0; i < pool.length; i++) {
            bytes32 node = names.nodeFor(pool[i]);
            assertEq(names.ownerOf(node), mLive(node) ? mOwner[node] : address(0));
            assertEq(names.pendingOwnerOf(node), mLive(node) ? mPending[node] : address(0));
            assertEq(names.addrOf(node), mLive(node) ? mAddr[node] : address(0));
        }
        // 3. every honored reverse claim points back at a live forward record
        for (uint256 a = 0; a < 3; a++) {
            string memory rn = names.reverseOf(payers[a]);
            if (bytes(rn).length != 0) {
                bytes32 rnNode = names.nodeFor(rn);
                assertEq(names.addrOf(rnNode), payers[a]);
                assertTrue(names.ownerOf(rnNode) != address(0));
            }
        }
    }
}
