// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.19;

import {EastSeaVault, EastSeaVaultFactory} from "../src/EastSeaVault.sol";

interface Vm {
    function warp(uint256) external;
    function deal(address, uint256) external;
    function etch(address, bytes calldata) external;
    function chainId(uint256) external;
    function expectEmit(bool, bool, bool, bool) external;
    function expectRevert(bytes calldata) external;
}

/// P-256 for the roles the tests play: deriving owner keys and signing digests
/// (the Secure Enclave's job in production); verification itself runs on the
/// real P256VERIFY precompile at 0x100 (the forge EVM runs osaka, like the
/// chain). Pinned against independently generated vectors in
/// test_P256KnownVector. Jacobian arithmetic (EFD dbl-2001-b / add-2007-bl),
/// inverses through the modexp precompile.
library LibP256 {
    uint256 constant p = 0xffffffff00000001000000000000000000000000ffffffffffffffffffffffff;
    uint256 constant n = 0xffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551;
    uint256 constant cb = 0x5ac635d8aa3a93e7b3ebbd55769886bc651d06b0cc53b0f63bce3c3e27d2604b;
    uint256 constant gx = 0x6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296;
    uint256 constant gy = 0x4fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5;

    /// x - y mod p. addmod is exact for any 256-bit operands, so this cannot underflow.
    function _subP(uint256 x, uint256 y) private pure returns (uint256) {
        return addmod(x, p - y, p);
    }

    /// x^-1 mod m (m prime) via the modexp precompile: sizes header, then
    /// base, exponent (m - 2, by Fermat), modulus.
    function _inv(uint256 x, uint256 m) private view returns (uint256 r) {
        (bool ok, bytes memory out) =
            address(5).staticcall(abi.encode(uint256(32), uint256(32), uint256(32), x, m - 2, m));
        require(ok && out.length == 32, "modexp");
        r = abi.decode(out, (uint256));
    }

    /// Jacobian point; (0, 0, 0) is the point at infinity. Points travel by
    /// reference so the arithmetic stays inside the old codegen's stack budget.
    struct Jac {
        uint256 x;
        uint256 y;
        uint256 z;
    }

    /// Jacobian doubling for a = -3 (EFD dbl-2001-b).
    function _dbl(Jac memory a) private pure returns (Jac memory) {
        if (a.z == 0 || a.y == 0) return Jac(0, 0, 0);
        uint256 delta = mulmod(a.z, a.z, p);
        uint256 gamma = mulmod(a.y, a.y, p);
        uint256 beta = mulmod(a.x, gamma, p);
        uint256 alpha = mulmod(3, mulmod(_subP(a.x, delta), addmod(a.x, delta, p), p), p);
        uint256 x3 = _subP(mulmod(alpha, alpha, p), mulmod(8, beta, p));
        uint256 y3 = _subP(mulmod(alpha, _subP(mulmod(4, beta, p), x3), p), mulmod(8, mulmod(gamma, gamma, p), p));
        return Jac(x3, y3, mulmod(2, mulmod(a.y, a.z, p), p));
    }

    /// Jacobian addition (EFD add-2007-bl).
    function _add(Jac memory a, Jac memory b) private pure returns (Jac memory) {
        if (a.z == 0) return b;
        if (b.z == 0) return a;
        Jac memory out = Jac(0, 0, 0);
        uint256 u1;
        uint256 s1;
        {
            uint256 z2z2 = mulmod(b.z, b.z, p);
            u1 = mulmod(a.x, z2z2, p);
            s1 = mulmod(mulmod(a.y, b.z, p), z2z2, p);
        }
        uint256 u2;
        uint256 s2;
        {
            uint256 z1z1 = mulmod(a.z, a.z, p);
            u2 = mulmod(b.x, z1z1, p);
            s2 = mulmod(mulmod(b.y, a.z, p), z1z1, p);
        }
        if (u1 == u2) {
            if (s1 != s2) return out; // P + (-P) is the point at infinity
            return _dbl(a);
        }
        uint256 h = _subP(u2, u1);
        uint256 i = mulmod(4, mulmod(h, h, p), p);
        uint256 j = mulmod(h, i, p);
        out.z = mulmod(mulmod(2, mulmod(a.z, b.z, p), p), h, p);
        uint256 r = mulmod(2, _subP(s2, s1), p);
        uint256 v = mulmod(u1, i, p);
        out.x = _subP(_subP(mulmod(r, r, p), j), mulmod(2, v, p));
        out.y = _subP(mulmod(r, _subP(v, out.x), p), mulmod(2, mulmod(s1, j, p), p));
        return out;
    }

    function _mulJ(uint256 k, uint256 x, uint256 y) private pure returns (Jac memory) {
        Jac memory acc = Jac(0, 0, 0);
        Jac memory g = Jac(x, y, 1);
        for (uint256 i = 0; i < 256; i++) {
            if (acc.z != 0) acc = _dbl(acc); // infinity stays infinity
            if (((k >> (255 - i)) & 1) == 1) acc = _add(acc, g);
        }
        return acc;
    }

    /// k * (x, y) in affine coordinates; (0, 0) for the point at infinity.
    function mul(uint256 k, uint256 x, uint256 y) internal view returns (uint256, uint256) {
        Jac memory r = _mulJ(k, x, y);
        if (r.z == 0) return (0, 0);
        uint256 zi = _inv(r.z, p);
        uint256 zi2 = mulmod(zi, zi, p);
        return (mulmod(r.x, zi2, p), mulmod(r.y, mulmod(zi2, zi, p), p));
    }

    function onCurve(uint256 x, uint256 y) internal pure returns (bool) {
        if (x >= p || y >= p) return false;
        uint256 lhs = mulmod(y, y, p);
        uint256 rhs = addmod(_subP(mulmod(mulmod(x, x, p), x, p), mulmod(3, x, p)), cb, p);
        return lhs == rhs;
    }

    /// d's public key, the way the vault stores owner keys.
    function derivePub(uint256 d) internal view returns (bytes32, bytes32) {
        (uint256 x, uint256 y) = mul(d, gx, gy);
        require(x != 0 && y != 0, "privkey");
        return (bytes32(x), bytes32(y));
    }

    /// ECDSA over a SHA-256 digest, with a chosen nonce k so tests are deterministic.
    function sign(uint256 d, bytes32 digest, uint256 k) internal view returns (bytes32, bytes32) {
        (uint256 rx, uint256 ry) = mul(k, gx, gy);
        require(rx != 0 && ry != 0, "nonce");
        uint256 r = rx % n;
        uint256 s = mulmod(_inv(k, n), addmod(uint256(digest) % n, mulmod(r, d, n), n), n);
        require(r != 0 && s != 0, "nonce");
        return (bytes32(r), bytes32(s));
    }

    /// What the 0x100 precompile checks.
    function verify(bytes32 digest, bytes32 rb, bytes32 sb, bytes32 xb, bytes32 yb) internal view returns (bool) {
        uint256 r = uint256(rb);
        uint256 s = uint256(sb);
        if (r == 0 || r >= n || s == 0 || s >= n) return false;
        if (!onCurve(uint256(xb), uint256(yb))) return false;
        return _recoverX(digest, r, s, xb, yb) == r;
    }

    /// x coordinate of u1*G + u2*Q, reduced mod n (0 for the point at infinity).
    function _recoverX(bytes32 digest, uint256 r, uint256 s, bytes32 xb, bytes32 yb) private view returns (uint256) {
        uint256 w = _inv(s, n);
        Jac memory p1 = _mulJ(mulmod(uint256(digest) % n, w, n), gx, gy);
        Jac memory p2 = _mulJ(mulmod(r, w, n), uint256(xb), uint256(yb));
        Jac memory q = _add(p1, p2);
        if (q.z == 0) return 0;
        uint256 zi = _inv(q.z, p);
        return mulmod(q.x, mulmod(zi, zi, p), p) % n;
    }
}

contract ERC20Mock {
    mapping(address => uint256) public balanceOf;

    function mint(address to, uint256 v) external {
        balanceOf[to] += v;
    }

    function transfer(address to, uint256 v) external returns (bool) {
        require(balanceOf[msg.sender] >= v, "balance");
        balanceOf[msg.sender] -= v;
        balanceOf[to] += v;
        return true;
    }
}

/// Calls back into the vault from inside transfer().
contract ReentrantERC20 {
    mapping(address => uint256) public balanceOf;
    EastSeaVault public vault;
    uint256 public attackId;
    bool public armed;
    bool public reentrySucceeded;

    function mint(address to, uint256 v) external {
        balanceOf[to] += v;
    }

    function arm(EastSeaVault v, uint256 id) external {
        vault = v;
        attackId = id;
        armed = true;
    }

    function transfer(address to, uint256 v) external returns (bool) {
        balanceOf[msg.sender] -= v;
        balanceOf[to] += v;
        if (armed) {
            armed = false;
            try vault.execute(attackId) {
                reentrySucceeded = true;
            } catch {}
        }
        return true;
    }
}

contract EastSeaVaultTest {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    // Owner private keys (d) and signing nonces (k), chosen away from every
    // edge case. test_P256KnownVector pins derivePub/sign to outside vectors.
    uint256 constant D0 = 0x1001;
    uint256 constant D1 = 0x1002;
    uint256 constant D2 = 0x1003;
    uint256 constant D3 = 0x1004;
    uint256 constant D4 = 0x1005;
    uint256 constant STRANGER = 0x9999;
    uint256 constant K = 0x2001;

    EastSeaVaultFactory factory;
    EastSeaVault vault;
    ERC20Mock token;
    address recipient = address(0xBEEF);

    event WithdrawalProposed(uint256 indexed id, address token, address indexed to, uint256 amount);
    event SettingsProposed(uint256 indexed id, EastSeaVault.Key[] newOwners, uint8 newThreshold, uint128 newDailyLimit, uint64 newDelay);
    event Approved(uint256 indexed id, uint256 indexed ownerIndex, uint256 approvals, uint64 readyAt);
    event WithdrawalExecuted(uint256 indexed id, address token, address indexed to, uint256 amount);
    event SettingsExecuted(uint256 indexed id, uint256 indexed era);
    event Canceled(uint256 indexed id, uint256 indexed ownerIndex);
    event Spent(uint256 indexed nonce, address indexed to, uint256 amount, uint256 indexed ownerIndex, uint128 spentToday);

    function setUp() public {
        factory = new EastSeaVaultFactory();
        vault = EastSeaVault(payable(factory.create(_owners(), 2, 1 ether, 48 hours, bytes32(uint256(0x51)))));
        vm.deal(address(vault), 100 ether);
        token = new ERC20Mock();
        token.mint(address(vault), 1000 ether);
    }

    // ---- helpers ----

    function key(uint256 d) internal view returns (EastSeaVault.Key memory) {
        (bytes32 x, bytes32 y) = LibP256.derivePub(d);
        return EastSeaVault.Key(x, y);
    }

    function _owners() internal view returns (EastSeaVault.Key[] memory ks) {
        ks = new EastSeaVault.Key[](3);
        ks[0] = key(D0);
        ks[1] = key(D1);
        ks[2] = key(D2);
    }

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

    function spendSig(address to, uint256 amount, uint256 nonce, uint256 d) internal view returns (bytes32 r, bytes32 s) {
        return LibP256.sign(d, vault.spendDigest(to, amount, nonce), K);
    }

    function withdrawSig(uint256 id, address tok, address to, uint256 amount, uint256 d)
        internal
        view
        returns (bytes32 r, bytes32 s)
    {
        return LibP256.sign(d, vault.withdrawDigest(id, tok, to, amount), K);
    }

    function settingsSig(uint256 id, EastSeaVault.Key[] memory ks, uint8 t, uint128 l, uint64 dly, uint256 d)
        internal
        view
        returns (bytes32 r, bytes32 s)
    {
        return LibP256.sign(d, vault.settingsDigest(id, ks, t, l, dly), K);
    }

    function cancelSig(uint256 id, uint256 nonce, uint256 d) internal view returns (bytes32 r, bytes32 s) {
        return LibP256.sign(d, vault.cancelDigest(id, nonce), K);
    }

    /// Propose a native withdrawal and approve it to the threshold (2-of-3).
    function readyWithdrawal(uint256 amount, uint256 id) internal {
        (bytes32 r, bytes32 s) = withdrawSig(id, address(0), recipient, amount, D0);
        vault.proposeWithdrawal(address(0), recipient, amount, 0, r, s);
        (r, s) = withdrawSig(id, address(0), recipient, amount, D1);
        vault.approve(id, 1, r, s);
    }

    // ---- the P-256 library against independently generated vectors ----

    function test_P256KnownVector() public view {
        bytes32 z = 0xc3268db9fdec1447c3b2cda3cfa8dab8f54872cd62fef7f04404fba4e3ef93f3;
        bytes32 px = 0x0f75e9e96a5bff7f4d75cf78cbd28ac9a3a3c1ad61166c45c0c9a94242df2c4e;
        bytes32 py = 0x5f9959a8f23a77ce255ca6afc6285db374a08f2704ab106843e2db302750a50d;
        bytes32 r = 0x2bec63180a2779a1376deada40885e56974272f54451b1c3770a4b4a6e502b3a;
        bytes32 s = 0x5b0ad1ccb82e3bfb70334cd87d70c52ed88ce96afdc9f44b24ca2711704da46f;

        (bytes32 dx, bytes32 dy) = LibP256.derivePub(0x1001);
        assertEq(uint256(dx), uint256(px));
        assertEq(uint256(dy), uint256(py));

        (bytes32 sr, bytes32 ss) = LibP256.sign(0x1001, z, 0x2001);
        assertEq(uint256(sr), uint256(r));
        assertEq(uint256(ss), uint256(s));

        assertTrue(LibP256.verify(z, r, s, px, py));
        assertTrue(!LibP256.verify(z, bytes32(uint256(r) + 1), s, px, py));
        assertTrue(!LibP256.verify(z, r, s, px, bytes32(uint256(py) ^ 1)));
        // tampered digest (a different message) must fail
        assertTrue(!LibP256.verify(bytes32(uint256(z) + 1), r, s, px, py));

        // ... and through the real 0x100 precompile, which is what the vault actually calls
        (bool ok, bytes memory out) = address(0x100).staticcall(abi.encodePacked(z, r, s, px, py));
        assertTrue(ok && out.length == 32 && abi.decode(out, (uint256)) == 1);
        // a tampered digest fails; the precompile answers a rejection with
        // empty return data, which the vault's `out.length == 32` check reads
        // as "not verified"
        (ok, out) = address(0x100).staticcall(abi.encodePacked(bytes32(uint256(z) + 1), r, s, px, py));
        assertTrue(ok && (out.length != 32 || abi.decode(out, (uint256)) == 0));
    }

    // ---- one owner, within the daily limit ----

    function test_SpendWithinDailyLimitByOneOwner() public {
        (bytes32 r, bytes32 s) = spendSig(recipient, 0.4 ether, 0, D0);
        vm.expectEmit(true, true, true, true);
        emit Spent(0, recipient, 0.4 ether, 0, 0.4 ether);
        vault.spend(recipient, 0.4 ether, 0, r, s);
        assertEq(recipient.balance, 0.4 ether);
        assertEq(vault.spendNonce(), 1);

        // a different owner, same day: the limit is vault-wide
        (r, s) = spendSig(recipient, 0.4 ether, 1, D1);
        vault.spend(recipient, 0.4 ether, 1, r, s);
        assertEq(recipient.balance, 0.8 ether);
        assertEq(vault.dailyAvailable(), 0.2 ether);

        // together over the limit
        (r, s) = spendSig(recipient, 0.3 ether, 2, D1);
        vm.expectRevert(abi.encodeWithSelector(EastSeaVault.OverDailyLimit.selector, 1.1 ether, 1 ether));
        vault.spend(recipient, 0.3 ether, 1, r, s);
    }

    function test_DailyLimitSlidingWindow() public {
        // 0.6 now; the same UTC day can hold only 0.4 more ...
        (bytes32 r, bytes32 s) = spendSig(recipient, 0.6 ether, 0, D0);
        vault.spend(recipient, 0.6 ether, 0, r, s);
        (r, s) = spendSig(recipient, 0.5 ether, 1, D0);
        vm.expectRevert(abi.encodeWithSelector(EastSeaVault.OverDailyLimit.selector, 1.1 ether, 1 ether));
        vault.spend(recipient, 0.5 ether, 0, r, s);

        // ... 12 h later still counts (same day)
        vm.warp(block.timestamp + 12 hours);
        (r, s) = spendSig(recipient, 0.5 ether, 1, D0);
        vm.expectRevert(abi.encodeWithSelector(EastSeaVault.OverDailyLimit.selector, 1.1 ether, 1 ether));
        vault.spend(recipient, 0.5 ether, 0, r, s);

        // next day: yesterday's 0.6 still bounds the last 24 h
        vm.warp(block.timestamp + 12 hours);
        assertEq(vault.dailyAvailable(), 0.4 ether);
        (r, s) = spendSig(recipient, 0.5 ether, 1, D0);
        vm.expectRevert(abi.encodeWithSelector(EastSeaVault.OverDailyLimit.selector, 1.1 ether, 1 ether));
        vault.spend(recipient, 0.5 ether, 0, r, s);
        (r, s) = spendSig(recipient, 0.4 ether, 1, D0);
        vault.spend(recipient, 0.4 ether, 0, r, s);

        // the day after, only 0.4 is remembered
        vm.warp(block.timestamp + 1 days);
        assertEq(vault.dailyAvailable(), 0.6 ether);
        (r, s) = spendSig(recipient, 1 ether, 2, D2);
        vm.expectRevert(abi.encodeWithSelector(EastSeaVault.OverDailyLimit.selector, 1.4 ether, 1 ether));
        vault.spend(recipient, 1 ether, 2, r, s);
        (r, s) = spendSig(recipient, 0.6 ether, 2, D2);
        vault.spend(recipient, 0.6 ether, 2, r, s);
        assertEq(recipient.balance, 1.6 ether);
    }

    // ---- above the limit: queue, approvals, delay, execute ----

    function test_WithdrawalQueueApprovalsDelayExecute() public {
        uint256 amount = 5 ether;

        (bytes32 r, bytes32 s) = withdrawSig(1, address(0), recipient, amount, D0);
        vm.expectEmit(true, true, true, true);
        emit WithdrawalProposed(1, address(0), recipient, amount);
        vault.proposeWithdrawal(address(0), recipient, amount, 0, r, s);

        EastSeaVault.Proposal memory p = vault.proposal(1);
        assertEq(uint8(p.kind), uint8(EastSeaVault.Kind.Withdraw));
        assertEq(uint256(p.approvals), 1);
        assertEq(uint256(p.readyAt), 0); // threshold (2) not reached

        // execute before the threshold is met
        vm.expectRevert(err(EastSeaVault.NotReady.selector));
        vault.execute(1);

        uint256 ts2 = block.timestamp;
        (r, s) = withdrawSig(1, address(0), recipient, amount, D1);
        vm.expectEmit(true, true, true, true);
        emit Approved(1, 1, 3, uint64(ts2) + 48 hours);
        vault.approve(1, 1, r, s);

        p = vault.proposal(1);
        assertEq(uint256(p.approvals), 3);
        uint64 readyAt = p.readyAt;
        assertEq(uint256(readyAt), ts2 + 48 hours);

        // a third owner approving later does not extend the delay
        (r, s) = withdrawSig(1, address(0), recipient, amount, D2);
        vault.approve(1, 2, r, s);
        assertEq(uint256(vault.proposal(1).readyAt), uint256(readyAt));

        // too early
        vm.warp(ts2 + 47 hours);
        vm.expectRevert(abi.encodeWithSelector(EastSeaVault.NotYet.selector, readyAt));
        vault.execute(1);
        assertEq(recipient.balance, 0);

        // after the delay
        vm.warp(readyAt);
        vm.expectEmit(true, true, true, true);
        emit WithdrawalExecuted(1, address(0), recipient, amount);
        vault.execute(1);
        assertEq(recipient.balance, amount);
        assertEq(address(vault).balance, 95 ether);

        // gone for good
        vm.expectRevert(err(EastSeaVault.UnknownProposal.selector));
        vault.execute(1);
    }

    function test_ERC20WithdrawalThroughQueue() public {
        (bytes32 r, bytes32 s) = withdrawSig(1, address(token), recipient, 400 ether, D0);
        vault.proposeWithdrawal(address(token), recipient, 400 ether, 0, r, s);
        (r, s) = withdrawSig(1, address(token), recipient, 400 ether, D1);
        vault.approve(1, 1, r, s);
        vm.warp(block.timestamp + 48 hours);
        vault.execute(1);
        assertEq(token.balanceOf(recipient), 400 ether);
        assertEq(token.balanceOf(address(vault)), 600 ether);
    }

    // ---- any one owner can cancel ----

    function test_CancelByAnyOwner() public {
        // before the threshold: a proposal can be vetoed early
        (bytes32 r, bytes32 s) = withdrawSig(1, address(0), recipient, 5 ether, D0);
        vault.proposeWithdrawal(address(0), recipient, 5 ether, 0, r, s);
        (r, s) = cancelSig(1, 0, D2); // not the proposer
        vault.cancel(1, 2, r, s);
        vm.expectRevert(err(EastSeaVault.UnknownProposal.selector));
        vault.execute(1);

        // during the delay: a ready proposal is still cancellable
        readyWithdrawal(6 ether, 2);
        uint64 readyAt = vault.proposal(2).readyAt;
        assertTrue(readyAt != 0);
        (r, s) = cancelSig(2, 1, D1);
        vault.cancel(2, 1, r, s);
        vm.warp(readyAt);
        vm.expectRevert(err(EastSeaVault.UnknownProposal.selector));
        vault.execute(2);
        assertEq(recipient.balance, 0);

        // a settings proposal cancels the same way
        EastSeaVault.Key[] memory ks = new EastSeaVault.Key[](2);
        ks[0] = key(D0);
        ks[1] = key(D3);
        (r, s) = settingsSig(3, ks, 2, 1 ether, 24 hours, D0);
        vault.proposeSettings(ks, 2, 1 ether, 24 hours, 0, r, s);
        (r, s) = cancelSig(3, 2, D2);
        vault.cancel(3, 2, r, s);
        assertEq(vault.ownerCount(), 3);
    }

    // ---- owner rotation through the same M-of-N + delay path ----

    function test_OwnerRotation() public {
        // a withdrawal that will be invalidated by the rotation
        readyWithdrawal(5 ether, 1);

        // rotate to (D3, D0, D4), still 2-of-3, new limit and delay
        EastSeaVault.Key[] memory ks = new EastSeaVault.Key[](3);
        ks[0] = key(D3);
        ks[1] = key(D0);
        ks[2] = key(D4);
        (bytes32 r, bytes32 s) = settingsSig(2, ks, 2, 2 ether, 24 hours, D1);
        vault.proposeSettings(ks, 2, 2 ether, 24 hours, 1, r, s); // proposed by old owner 1 (= D1)
        (r, s) = settingsSig(2, ks, 2, 2 ether, 24 hours, D2);
        vault.approve(2, 2, r, s);
        assertEq(uint256(vault.proposal(2).readyAt), block.timestamp + 48 hours);

        vm.warp(block.timestamp + 48 hours);
        vm.expectEmit(true, true, true, true);
        emit SettingsExecuted(2, 1);
        vault.execute(2);

        assertEq(vault.ownerCount(), 3);
        assertEq(uint256(vault.threshold()), 2);
        assertEq(uint256(vault.delay()), 24 hours);
        assertEq(uint256(vault.dailyLimit()), 2 ether);
        (bytes32 x0,) = vault.owners(0);
        assertEq(uint256(x0), uint256(ks[0].x));

        // the pending withdrawal from the old era is gone
        vm.expectRevert(err(EastSeaVault.UnknownProposal.selector));
        vault.execute(1);

        // a removed owner (old index 2 = D2) no longer signs anything
        (r, s) = spendSig(recipient, 0.1 ether, 0, D2);
        vm.expectRevert(err(EastSeaVault.BadSignature.selector));
        vault.spend(recipient, 0.1 ether, 2, r, s);

        // a new owner spends under the new limit
        (r, s) = spendSig(recipient, 1.5 ether, 0, D3);
        vault.spend(recipient, 1.5 ether, 0, r, s);
        assertEq(recipient.balance, 1.5 ether);

        // and proposes withdrawals of the new set
        (r, s) = withdrawSig(3, address(0), recipient, 5 ether, D4);
        vault.proposeWithdrawal(address(0), recipient, 5 ether, 2, r, s);
    }

    // ---- replay protection ----

    function test_ReplaySpendNonce() public {
        (bytes32 r, bytes32 s) = spendSig(recipient, 0.1 ether, 0, D0);
        vault.spend(recipient, 0.1 ether, 0, r, s);
        // same signature again: the nonce moved
        vm.expectRevert(err(EastSeaVault.BadSignature.selector));
        vault.spend(recipient, 0.1 ether, 0, r, s);
        assertEq(recipient.balance, 0.1 ether);
    }

    function test_ReplayApprovalTwice() public {
        (bytes32 r, bytes32 s) = withdrawSig(1, address(0), recipient, 5 ether, D0);
        vault.proposeWithdrawal(address(0), recipient, 5 ether, 0, r, s);
        (r, s) = withdrawSig(1, address(0), recipient, 5 ether, D1);
        vault.approve(1, 1, r, s);
        // even a fresh signature over the same digest cannot count twice
        vm.expectRevert(err(EastSeaVault.AlreadyApproved.selector));
        vault.approve(1, 1, r, s);
    }

    function test_ReplayAcrossVaults() public {
        EastSeaVault other = EastSeaVault(payable(factory.create(_owners(), 2, 1 ether, 48 hours, bytes32(uint256(0x52)))));
        (bytes32 r, bytes32 s) = withdrawSig(1, address(0), recipient, 5 ether, D0);
        vault.proposeWithdrawal(address(0), recipient, 5 ether, 0, r, s);
        // the same signature does not open a proposal on another vault
        vm.expectRevert(err(EastSeaVault.BadSignature.selector));
        other.proposeWithdrawal(address(0), recipient, 5 ether, 0, r, s);
    }

    function test_ReplayAcrossChains() public {
        (bytes32 r, bytes32 s) = spendSig(recipient, 0.1 ether, 0, D0);
        vm.chainId(7781);
        vm.expectRevert(err(EastSeaVault.BadSignature.selector));
        vault.spend(recipient, 0.1 ether, 0, r, s);
    }

    function test_ReplayCancelNonce() public {
        (bytes32 r, bytes32 s) = withdrawSig(1, address(0), recipient, 5 ether, D0);
        vault.proposeWithdrawal(address(0), recipient, 5 ether, 0, r, s);
        (r, s) = cancelSig(1, 0, D1);
        vault.cancel(1, 1, r, s);
        // re-proposing the same withdrawal: the old cancel signature is spent
        (r, s) = withdrawSig(2, address(0), recipient, 5 ether, D0);
        vault.proposeWithdrawal(address(0), recipient, 5 ether, 0, r, s);
        (bytes32 cr, bytes32 cs) = cancelSig(2, 0, D1);
        vm.expectRevert(err(EastSeaVault.BadSignature.selector)); // nonce is 1 now
        vault.cancel(2, 1, cr, cs);
        (cr, cs) = cancelSig(2, 1, D1);
        vault.cancel(2, 1, cr, cs);
    }

    // ---- reentrancy on the ERC-20 transfer ----

    function test_ReentrancyOnERC20Transfer() public {
        ReentrantERC20 evil = new ReentrantERC20();
        evil.mint(address(vault), 50 ether);

        (bytes32 r, bytes32 s) = withdrawSig(1, address(evil), recipient, 50 ether, D0);
        vault.proposeWithdrawal(address(evil), recipient, 50 ether, 0, r, s);
        (r, s) = withdrawSig(1, address(evil), recipient, 50 ether, D1);
        vault.approve(1, 1, r, s);
        uint64 readyAt = vault.proposal(1).readyAt;

        evil.arm(vault, 1);
        vm.warp(readyAt);
        vault.execute(1); // the token calls execute(1) back from transfer()

        assertTrue(!evil.reentrySucceeded());
        assertEq(evil.balanceOf(recipient), 50 ether); // moved exactly once
        assertEq(evil.balanceOf(address(vault)), 0);
        vm.expectRevert(err(EastSeaVault.UnknownProposal.selector));
        vault.execute(1);
    }

    // ---- wrong signers ----

    function test_WrongSignerRejected() public {
        // not an owner at all
        (bytes32 r, bytes32 s) = spendSig(recipient, 0.1 ether, 0, STRANGER);
        vm.expectRevert(err(EastSeaVault.BadSignature.selector));
        vault.spend(recipient, 0.1 ether, 0, r, s);

        // a valid signature from a different owner, claimed as another index
        (r, s) = spendSig(recipient, 0.1 ether, 0, D0);
        vm.expectRevert(err(EastSeaVault.BadSignature.selector));
        vault.spend(recipient, 0.1 ether, 1, r, s);

        // signed over different parameters than relayed
        (r, s) = withdrawSig(1, address(0), recipient, 5 ether, D0);
        vm.expectRevert(err(EastSeaVault.BadSignature.selector));
        vault.proposeWithdrawal(address(0), recipient, 6 ether, 0, r, s);

        // approve with the proposer's own key under another owner's index
        (r, s) = withdrawSig(1, address(0), recipient, 5 ether, D0);
        vault.proposeWithdrawal(address(0), recipient, 5 ether, 0, r, s);
        (r, s) = withdrawSig(1, address(0), recipient, 5 ether, D0);
        vm.expectRevert(err(EastSeaVault.BadSignature.selector));
        vault.approve(1, 1, r, s);

        // cancel by a stranger
        (r, s) = cancelSig(1, 0, STRANGER);
        vm.expectRevert(err(EastSeaVault.BadSignature.selector));
        vault.cancel(1, 1, r, s);

        // index out of range
        vm.expectRevert(err(EastSeaVault.BadSignature.selector));
        vault.spend(recipient, 0.1 ether, 3, bytes32(uint256(1)), bytes32(uint256(1)));
    }

    // ---- the 24 h delay minimum ----

    function test_DelayMinimumEnforced() public {
        // key derivation staticcalls the modexp precompile, so everything the
        // reverting calls need is built before vm.expectRevert (which latches
        // onto the very next call).
        EastSeaVault.Key[] memory ks = _owners();
        EastSeaVault.Key[] memory one = new EastSeaVault.Key[](1);
        one[0] = key(D0);
        (bytes32 r, bytes32 s) = settingsSig(1, one, 1, 1 ether, 12 hours, D0);

        // at creation, directly and through the factory
        vm.expectRevert(err(EastSeaVault.BadConfig.selector));
        new EastSeaVault(ks, 2, 1 ether, 23 hours);
        vm.expectRevert(err(EastSeaVault.BadConfig.selector));
        factory.create(ks, 2, 1 ether, 23 hours, bytes32(uint256(0x53)));

        // 24 h on the dot is fine
        EastSeaVault ok24 = EastSeaVault(payable(factory.create(ks, 2, 1 ether, 24 hours, bytes32(uint256(0x53)))));
        assertEq(uint256(ok24.delay()), 24 hours);

        // settings proposals cannot lower it below the minimum either
        vm.expectRevert(err(EastSeaVault.BadConfig.selector));
        vault.proposeSettings(one, 1, 1 ether, 12 hours, 0, r, s);
    }

    // ---- config validation ----

    function test_ConfigValidation() public {
        // built up front: vm.expectRevert latches onto the next call, and key
        // derivation itself staticcalls the modexp precompile.
        EastSeaVault.Key[] memory ks = _owners();
        EastSeaVault.Key[] memory none = new EastSeaVault.Key[](0);
        EastSeaVault.Key[] memory dup = new EastSeaVault.Key[](2);
        dup[0] = key(D0);
        dup[1] = key(D0);
        EastSeaVault.Key[] memory zero = new EastSeaVault.Key[](1);
        zero[0] = EastSeaVault.Key(bytes32(0), bytes32(0));

        vm.expectRevert(err(EastSeaVault.BadConfig.selector)); // threshold 0
        new EastSeaVault(ks, 0, 1 ether, 48 hours);
        vm.expectRevert(err(EastSeaVault.BadConfig.selector)); // threshold above the count
        new EastSeaVault(ks, 4, 1 ether, 48 hours);
        vm.expectRevert(err(EastSeaVault.BadConfig.selector)); // no owners
        new EastSeaVault(none, 1, 1 ether, 48 hours);
        vm.expectRevert(err(EastSeaVault.BadConfig.selector)); // duplicate keys
        new EastSeaVault(dup, 1, 1 ether, 48 hours);
        vm.expectRevert(err(EastSeaVault.BadConfig.selector)); // zero key
        new EastSeaVault(zero, 1, 1 ether, 48 hours);
    }

    // ---- CREATE2 factory ----

    function test_FactoryDeterministicAddress() public {
        bytes32 salt = bytes32(uint256(0x77));
        address predicted = factory.predict(_owners(), 2, 1 ether, 48 hours, salt);
        address deployed = factory.create(_owners(), 2, 1 ether, 48 hours, salt);
        assertEq(deployed, predicted);
        assertEq(EastSeaVault(payable(deployed)).ownerCount(), 3);

        // same owners + salt (and other params) again: CREATE2 collision
        bool collided;
        try factory.create(_owners(), 2, 1 ether, 48 hours, salt) {
            collided = true;
        } catch {}
        assertTrue(!collided);

        // a different salt gives a different address
        address other = factory.create(_owners(), 2, 1 ether, 48 hours, bytes32(uint256(0x78)));
        assertTrue(other != deployed);

        // and the parameters are part of the address
        assertTrue(factory.predict(_owners(), 3, 1 ether, 48 hours, salt) != predicted);
    }

    function test_VaultCreatedEvent() public {
        bytes32 salt = bytes32(uint256(0x79));
        address predicted = factory.predict(_owners(), 2, 1 ether, 48 hours, salt);
        vm.expectEmit(true, true, true, true);
        emit VaultCreated(predicted, salt, 2, 1 ether, 48 hours);
        factory.create(_owners(), 2, 1 ether, 48 hours, salt);
    }

    event VaultCreated(address indexed vault, bytes32 indexed salt, uint8 threshold, uint128 dailyLimit, uint64 delay);
}
