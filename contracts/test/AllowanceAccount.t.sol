// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.19;

import {AllowanceAccount} from "../base/AllowanceAccount.sol";
import {LibP256} from "./EastSeaVault.t.sol";
import {P256Fallback} from "../base/P256Fallback.sol";

interface VmAllowance {
    function warp(uint256) external;
    function etch(address, bytes calldata) external;
    function prank(address) external;
    function expectRevert(bytes calldata) external;
    function chainId(uint256) external;
}

contract MockAllowanceUSDC {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;
    mapping(address => bool) public blacklisted;

    function mint(address to, uint256 amount) external {
        balanceOf[to] += amount;
    }

    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
        return true;
    }

    function blacklist(address who, bool blocked) external {
        blacklisted[who] = blocked;
    }

    function transfer(address to, uint256 amount) external returns (bool) {
        require(!blacklisted[msg.sender] && !blacklisted[to], "blacklisted");
        require(balanceOf[msg.sender] >= amount, "balance");
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
        return true;
    }

    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        require(!blacklisted[from] && !blacklisted[to], "blacklisted");
        require(allowance[from][msg.sender] >= amount, "allowance");
        require(balanceOf[from] >= amount, "balance");
        allowance[from][msg.sender] -= amount;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
        return true;
    }
}

contract AllowanceAccountTest {
    VmAllowance constant vm = VmAllowance(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);
    uint256 constant ORDER = 0xffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551;
    uint256 constant OWNER = 0x1001;
    uint256 constant GUARDIAN = 0x1002;
    uint256 constant AGENT = 0x1003;
    uint256 constant REPLACEMENT = 0x1004;
    address constant PAYEE = address(0xBEEF);
    address constant OTHER = address(0xBAD);
    address constant ENTRY_POINT = address(0x4337);

    AllowanceAccount account;
    MockAllowanceUSDC token;

    function setUp() public {
        token = new MockAllowanceUSDC();
        account = new AllowanceAccount(key(OWNER), address(token), ENTRY_POINT);
        token.mint(address(this), 100_000_000);
        token.approve(address(account), type(uint256).max);
        vm.warp(10 days);
    }

    function key(uint256 d) internal view returns (AllowanceAccount.Key memory) {
        (bytes32 x, bytes32 y) = LibP256.derivePub(d);
        return AllowanceAccount.Key(x, y);
    }

    function sign(uint256 d, bytes32 digest) internal view returns (bytes memory) {
        (bytes32 r, bytes32 s) = LibP256.sign(d, digest, 0x2001);
        if (uint256(s) > ORDER / 2) s = bytes32(ORDER - uint256(s));
        return abi.encodePacked(r, s);
    }

    function ownerCall(bytes memory data) internal {
        uint64 expiry = uint64(block.timestamp + 1 hours);
        bytes32 digest = account.ownerDigest(address(account), 0, data, account.ownerNonce(), expiry);
        account.execute(address(account), 0, data, expiry, sign(OWNER, digest));
    }

    function ownerCallExternal(bytes memory data) external {
        ownerCall(data);
    }

    function setGuardian() internal {
        ownerCall(abi.encodeCall(account.setGuardian, (key(GUARDIAN))));
    }

    function setSession() internal {
        address[] memory payees = new address[](1);
        payees[0] = PAYEE;
        ownerCall(
            abi.encodeCall(
                account.createSession,
                (key(AGENT), uint128(3_000_000), uint128(5_000_000), uint64(block.timestamp + 4 days), payees)
            )
        );
    }

    function pay(uint256 amount, address recipient) internal {
        bytes memory callData = abi.encodeWithSelector(0xa9059cbb, recipient, amount);
        uint64 expiry = uint64(block.timestamp + 1 hours);
        bytes32 digest = account.sessionDigest(1, sessionNonce(), expiry, callData);
        account.sessionExecute(1, callData, expiry, sign(AGENT, digest));
    }

    function payExternal(uint256 amount, address recipient) external {
        pay(amount, recipient);
    }

    function sessionNonce() internal view returns (uint256 nonce) {
        (,,,,,,, nonce,,) = account.sessions(1);
    }

    function testP256FallbackAcceptReject() public {
        // This forge runs osaka, so 0x100 is the real precompile: it accepts a
        // valid signature, and a rejection (empty output) drops to the
        // Solidity fallback, which must reject too. The fallback's accept path
        // (Anvil/older EVMs without 0x100) is checked on the library directly.
        setGuardian();
        uint64 expiry = uint64(block.timestamp + 1 hours);
        bytes memory data = abi.encodeCall(account.cancelRecovery, ());
        bytes32 digest = account.ownerDigest(address(account), 0, data, 1, expiry);
        (bytes32 r, bytes32 s) = LibP256.sign(OWNER, digest, 0x2002);
        (bytes32 x, bytes32 y) = LibP256.derivePub(OWNER);
        require(P256Fallback.verify(digest, r, s, x, y), "fallback accepts a valid signature");
        require(!P256Fallback.verify(bytes32(uint256(digest) ^ 1), r, s, x, y), "fallback rejects another digest");
        bytes memory wrongSignature = sign(AGENT, digest);
        vm.expectRevert(abi.encodeWithSelector(AllowanceAccount.Unauthorized.selector));
        account.execute(address(account), 0, data, expiry, wrongSignature);
        setGuardian();
    }

    function testPrecompilePathAndHighSRejected() public {
        setGuardian();
        bytes memory data = abi.encodeCall(account.setGuardian, (key(GUARDIAN)));
        uint64 expiry = uint64(block.timestamp + 1 hours);
        bytes32 digest = account.ownerDigest(address(account), 0, data, 1, expiry);
        (bytes32 r, bytes32 s) = LibP256.sign(OWNER, digest, 0x2001);
        if (uint256(s) <= ORDER / 2) s = bytes32(ORDER - uint256(s));
        vm.expectRevert(abi.encodeWithSelector(AllowanceAccount.Unauthorized.selector));
        account.execute(address(account), 0, data, expiry, abi.encodePacked(r, s));
        ownerCall(data);
    }

    function testOwnerReplayExpiryAndChainDomain() public {
        setGuardian();
        bytes memory data = abi.encodeCall(account.setGuardian, (key(GUARDIAN)));
        uint64 expiry = uint64(block.timestamp + 1);
        bytes memory sig = sign(OWNER, account.ownerDigest(address(account), 0, data, 1, expiry));
        account.execute(address(account), 0, data, expiry, sig);
        vm.expectRevert(abi.encodeWithSelector(AllowanceAccount.Unauthorized.selector));
        account.execute(address(account), 0, data, expiry, sig);
        vm.warp(block.timestamp + 2);
        vm.expectRevert(abi.encodeWithSelector(AllowanceAccount.Expired.selector));
        account.execute(address(account), 0, data, expiry, sig);
        uint64 later = uint64(block.timestamp + 1 hours);
        bytes memory wrongChain = sign(OWNER, account.ownerDigest(address(account), 0, data, 2, later));
        vm.chainId(block.chainid + 1);
        vm.expectRevert(abi.encodeWithSelector(AllowanceAccount.Unauthorized.selector));
        account.execute(address(account), 0, data, later, wrongChain);
    }

    function testGuardianRequiredBeforeFundAndRecovery() public {
        vm.expectRevert(abi.encodeWithSelector(AllowanceAccount.InvalidKey.selector));
        account.fund(10_000_000);
        setGuardian();
        account.fund(10_000_000);
        setSession();
        AllowanceAccount.Key memory replacement = key(REPLACEMENT);
        uint64 expiry = uint64(block.timestamp + 1 hours);
        bytes32 digest = account.recoveryDigest(replacement, 0, expiry);
        account.proposeRecovery(replacement, expiry, sign(GUARDIAN, digest));
        vm.expectRevert(abi.encodeWithSelector(AllowanceAccount.RecoveryNotReady.selector));
        account.finishRecovery();
        vm.warp(block.timestamp + account.RECOVERY_DELAY());
        account.finishRecovery();
        (bytes32 x,) = account.owner();
        assert(x == replacement.x);
        vm.expectRevert(abi.encodeWithSelector(AllowanceAccount.InvalidSession.selector));
        this.payExternal(1_000_000, PAYEE);
    }

    function testRecoveryCancellationAndOwnerRotation() public {
        setGuardian();
        setSession();
        AllowanceAccount.Key memory replacement = key(REPLACEMENT);
        uint64 expiry = uint64(block.timestamp + 1 hours);
        account.proposeRecovery(replacement, expiry, sign(GUARDIAN, account.recoveryDigest(replacement, 0, expiry)));
        ownerCall(abi.encodeCall(account.cancelRecovery, ()));
        vm.warp(block.timestamp + account.RECOVERY_DELAY());
        vm.expectRevert(abi.encodeWithSelector(AllowanceAccount.RecoveryNotReady.selector));
        account.finishRecovery();
        ownerCall(abi.encodeCall(account.rotateOwner, (replacement)));
        vm.expectRevert(abi.encodeWithSelector(AllowanceAccount.InvalidSession.selector));
        this.payExternal(1_000_000, PAYEE);
    }

    function testRecoveryKeyCannotBeAgentKey() public {
        setGuardian();
        address[] memory payees = new address[](1);
        payees[0] = PAYEE;
        bytes memory sessionCall = abi.encodeCall(
            account.createSession,
            (key(GUARDIAN), uint128(1_000_000), uint128(2_000_000), uint64(block.timestamp + 1 days), payees)
        );
        vm.expectRevert(abi.encodeWithSelector(AllowanceAccount.InvalidSession.selector));
        this.ownerCallExternal(sessionCall);
        setSession();
        bytes memory guardianCall = abi.encodeCall(account.setGuardian, (key(AGENT)));
        vm.expectRevert(abi.encodeWithSelector(AllowanceAccount.InvalidKey.selector));
        this.ownerCallExternal(guardianCall);
    }

    function testSessionLimitsAcrossDaysReplayAndRevocation() public {
        setGuardian();
        account.fund(20_000_000);
        setSession();
        pay(3_000_000, PAYEE);
        bytes memory callData = abi.encodeWithSelector(0xa9059cbb, PAYEE, 3_000_000);
        uint64 expiry = uint64(block.timestamp + 1 hours);
        bytes memory oldSig = sign(AGENT, account.sessionDigest(1, 0, expiry, callData));
        vm.expectRevert(abi.encodeWithSelector(AllowanceAccount.Unauthorized.selector));
        account.sessionExecute(1, callData, expiry, oldSig);
        vm.expectRevert(abi.encodeWithSelector(AllowanceAccount.LimitExceeded.selector));
        this.payExternal(3_000_000, PAYEE);
        vm.warp(block.timestamp + 1 days);
        vm.expectRevert(abi.encodeWithSelector(AllowanceAccount.LimitExceeded.selector));
        this.payExternal(3_000_000, PAYEE); // Previous UTC day's spend still counts.
        vm.warp(block.timestamp + 1 days);
        pay(3_000_000, PAYEE);
        ownerCall(abi.encodeCall(account.revokeSession, (1)));
        vm.expectRevert(abi.encodeWithSelector(AllowanceAccount.InvalidSession.selector));
        this.payExternal(1_000_000, PAYEE);
    }

    function testPerPaymentAndSessionExpiry() public {
        setGuardian();
        account.fund(10_000_000);
        setSession();
        vm.expectRevert(abi.encodeWithSelector(AllowanceAccount.LimitExceeded.selector));
        this.payExternal(3_000_001, PAYEE);
        vm.warp(block.timestamp + 4 days);
        vm.expectRevert(abi.encodeWithSelector(AllowanceAccount.Expired.selector));
        this.payExternal(1_000_000, PAYEE);
    }

    function testPayeeAndMaliciousCalldataRejected() public {
        setGuardian();
        account.fund(10_000_000);
        setSession();
        vm.expectRevert(abi.encodeWithSelector(AllowanceAccount.InvalidCall.selector));
        this.payExternal(1_000_000, OTHER);
        bytes memory malicious = abi.encodeWithSelector(0x23b872dd, address(account), OTHER, 1_000_000);
        vm.expectRevert(abi.encodeWithSelector(AllowanceAccount.InvalidCall.selector));
        account.sessionExecute(1, malicious, uint64(block.timestamp + 1 hours), new bytes(64));
        malicious = abi.encodePacked(abi.encodeWithSelector(0xa9059cbb, PAYEE, 1_000_000), hex"00");
        vm.expectRevert(abi.encodeWithSelector(AllowanceAccount.InvalidCall.selector));
        account.sessionExecute(1, malicious, uint64(block.timestamp + 1 hours), new bytes(64));
    }

    function testBlacklistedAccountCannotSpendAndNoCapConsumed() public {
        setGuardian();
        account.fund(10_000_000);
        setSession();
        token.blacklist(address(account), true);
        vm.expectRevert(abi.encodeWithSelector(AllowanceAccount.TokenTransferFailed.selector));
        this.payExternal(1_000_000, PAYEE);
        assert(sessionNonce() == 0);
        token.blacklist(address(account), false);
        pay(1_000_000, PAYEE);
        assert(token.balanceOf(PAYEE) == 1_000_000);
    }

    function testUserOpValidationOnlyEntryPointAndNoReplay() public {
        AllowanceAccount.PackedUserOperation memory op;
        op.sender = address(account);
        op.callData = abi.encodeCall(account.executeFromEntryPoint, (address(token), 0, bytes("")));
        bytes32 hash = keccak256("local user operation");
        uint64 expiry = uint64(block.timestamp + 1 hours);
        bytes memory sig = sign(OWNER, account.userOpDigest(hash, 0, expiry));
        bytes32 r;
        bytes32 s;
        assembly {
            r := mload(add(sig, 32))
            s := mload(add(sig, 64))
        }
        op.signature = abi.encode(expiry, r, s);
        vm.expectRevert(abi.encodeWithSelector(AllowanceAccount.Unauthorized.selector));
        account.validateUserOp(op, hash, 0);
        vm.prank(ENTRY_POINT);
        assert(account.validateUserOp(op, hash, 0) == 0);
        vm.prank(ENTRY_POINT);
        assert(account.validateUserOp(op, hash, 0) == 1);
    }
}
