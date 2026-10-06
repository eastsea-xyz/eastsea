// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.19;

import {EastSeaAccount} from "../src/EastSeaAccount.sol";
import {LibP256} from "./EastSeaVault.t.sol";

interface Vm1271 {
    function prank(address) external;
    function chainId(uint256) external;
    function etch(address, bytes calldata) external;
}

/// ERC-1271 and the token receiver hooks, with real P-256 signatures checked by
/// the P256VERIFY precompile (0x100) — no mocking. Each account sits at its
/// key's own chain address, keccak256(0x01 ‖ compressed key)[12:] (the Rust
/// executor suite checks this derivation against `aether_crypto::address_of`),
/// so `vm.etch` of the account code there models a 7702-delegated account.
contract EastSeaAccount1271Test {
    Vm1271 constant vm = Vm1271(address(uint160(uint256(keccak256("hevm cheat code")))));

    uint256 constant N = 0xffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551;
    uint256 constant A_KEY = 0xa11ce;
    uint256 constant B_KEY = 0xb0b;
    uint256 constant OWNER_KEY = 0x0b0b0b;
    uint256 constant SESSION_KEY = 0x5e55;
    uint256 constant GUARDIAN_KEY = 0x9a4d;

    bytes4 constant MAGIC = 0x1626ba7e;
    bytes4 constant LEGACY_MAGIC = 0x20c13b0b;
    bytes4 constant INVALID = 0xffffffff;
    bytes32 constant MSG = keccak256("a Permit2 permit, say");
    bytes32 constant OTHER_MSG = keccak256("something else");

    uint256 nonceSeed = 0x1000;

    function addressOf(uint256 d) internal view returns (address) {
        (bytes32 x, bytes32 y) = LibP256.derivePub(d);
        bytes1 prefix = uint256(y) & 1 == 0 ? bytes1(0x02) : bytes1(0x03);
        return address(uint160(uint256(keccak256(abi.encodePacked(uint8(1), prefix, x)))));
    }

    function accountOf(uint256 d) internal returns (EastSeaAccount) {
        address a = addressOf(d);
        vm.etch(a, type(EastSeaAccount).runtimeCode);
        return EastSeaAccount(payable(a));
    }

    /// The digest an owner key must sign, rebuilt here from EIP-712 by hand.
    function expectedDigest(address account, bytes32 hash) internal view returns (bytes32) {
        bytes32 domain = keccak256(
            abi.encode(
                keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
                keccak256("EastSeaAccount"),
                keccak256("2"),
                block.chainid,
                account
            )
        );
        bytes32 contents = keccak256(abi.encode(keccak256("Contents(bytes32 contents)"), hash));
        return sha256(abi.encodePacked("\x19\x01", domain, contents));
    }

    /// r ‖ s ‖ x ‖ y over `digest` by key `d`, low-s.
    function sig(uint256 d, bytes32 digest) internal returns (bytes memory) {
        (bytes32 r, bytes32 s) = LibP256.sign(d, digest, nonceSeed++);
        if (uint256(s) > N / 2) s = bytes32(N - uint256(s));
        (bytes32 x, bytes32 y) = LibP256.derivePub(d);
        return abi.encodePacked(r, s, x, y);
    }

    /// `d` signing `hash` for `account` on the current chain.
    function signFor(uint256 d, address account, bytes32 hash) internal returns (bytes memory) {
        return sig(d, expectedDigest(account, hash));
    }

    function testDigestIsTheEip712MessageBoundToAccountAndChain() public {
        EastSeaAccount a = accountOf(A_KEY);
        require(a.signatureDigest(MSG) == expectedDigest(address(a), MSG), "digest");
        require(a.signatureDigest(MSG) == sha256(a.signatureMessage(MSG)), "sha256 of the message");
        require(a.signatureMessage(MSG).length == 66, "\\x19\\x01 || domain || struct");
        uint256 here = block.chainid;
        vm.chainId(137);
        require(a.signatureDigest(MSG) != expectedDigestOn(here, address(a), MSG), "chain bound");
        EastSeaAccount b = accountOf(B_KEY);
        require(b.signatureDigest(MSG) != a.signatureDigest(MSG), "account bound");
    }

    function expectedDigestOn(uint256 chain, address account, bytes32 hash) internal returns (bytes32 d) {
        uint256 here = block.chainid;
        vm.chainId(chain);
        d = expectedDigest(account, hash);
        vm.chainId(here);
    }

    function testTheOriginalKeyIsValidAndWrongInputsAreNot() public {
        EastSeaAccount a = accountOf(A_KEY);
        bytes memory good = signFor(A_KEY, address(a), MSG);
        require(a.isValidSignature(MSG, good) == MAGIC, "original key");
        require(a.isValidSignature(OTHER_MSG, good) == INVALID, "hash the signature was not made for");
        require(a.isValidSignature(MSG, signFor(A_KEY, address(a), OTHER_MSG)) == INVALID, "signature over another hash");
        require(a.isValidSignature(MSG, truncated(good, 127)) == INVALID, "short blob");
        require(a.isValidSignature(MSG, abi.encodePacked(good, bytes1(0))) == INVALID, "long blob");
        require(a.isValidSignature(MSG, "") == INVALID, "empty blob");
        // The bare hash (no account/chain binding) is not what an owner signs.
        require(a.isValidSignature(MSG, sig(A_KEY, MSG)) == INVALID, "unbound bare hash");
        require(a.isValidSignature(MSG, sig(A_KEY, sha256(abi.encode(MSG)))) == INVALID, "unbound sha256");
    }

    function testTheHighSTwinIsRefused() public {
        EastSeaAccount a = accountOf(A_KEY);
        bytes memory good = signFor(A_KEY, address(a), MSG);
        (bytes32 r, bytes32 s, bytes32 x, bytes32 y) = abi.decode(good, (bytes32, bytes32, bytes32, bytes32));
        bytes memory twin = abi.encodePacked(r, bytes32(N - uint256(s)), x, y);
        require(a.isValidSignature(MSG, good) == MAGIC, "low-s accepted");
        require(a.isValidSignature(MSG, twin) == INVALID, "high-s twin refused");
    }

    function testARecoveryAddedOwnerKeyIsValid() public {
        EastSeaAccount a = accountOf(A_KEY);
        bytes memory byOwner = signFor(OWNER_KEY, address(a), MSG);
        require(a.isValidSignature(MSG, byOwner) == INVALID, "not an owner yet");
        (bytes32 x, bytes32 y) = LibP256.derivePub(OWNER_KEY);
        vm.prank(address(a));
        a.addOwner(EastSeaAccount.Key(x, y));
        require(a.isValidSignature(MSG, byOwner) == MAGIC, "owner key");
        vm.prank(address(a));
        a.removeOwner(0);
        require(a.isValidSignature(MSG, byOwner) == INVALID, "removed owner");
    }

    function testASignatureCannotReplayAcrossAccounts() public {
        EastSeaAccount a = accountOf(A_KEY);
        EastSeaAccount b = accountOf(B_KEY);
        // A's original key does not own B.
        require(b.isValidSignature(MSG, signFor(A_KEY, address(a), MSG)) == INVALID, "A's key for B");
        require(b.isValidSignature(MSG, signFor(A_KEY, address(b), MSG)) == INVALID, "A's key, B's domain");
        // One owner key on both accounts: a signature made for A is not B's.
        (bytes32 x, bytes32 y) = LibP256.derivePub(OWNER_KEY);
        vm.prank(address(a));
        a.addOwner(EastSeaAccount.Key(x, y));
        vm.prank(address(b));
        b.addOwner(EastSeaAccount.Key(x, y));
        bytes memory forA = signFor(OWNER_KEY, address(a), MSG);
        require(a.isValidSignature(MSG, forA) == MAGIC, "valid for A");
        require(b.isValidSignature(MSG, forA) == INVALID, "replayed on B");
        require(b.isValidSignature(MSG, signFor(OWNER_KEY, address(b), MSG)) == MAGIC, "B's own");
    }

    function testASignatureCannotReplayAcrossChains() public {
        vm.chainId(1);
        EastSeaAccount a = accountOf(A_KEY);
        bytes memory onChain1 = signFor(A_KEY, address(a), MSG);
        require(a.isValidSignature(MSG, onChain1) == MAGIC, "own chain");
        vm.chainId(137);
        require(a.isValidSignature(MSG, onChain1) == INVALID, "chain-1 signature on chain 137");
        require(a.isValidSignature(MSG, signFor(A_KEY, address(a), MSG)) == MAGIC, "chain-137 signature");
    }

    function testSessionAndGuardianKeysAreRefused() public {
        EastSeaAccount a = accountOf(A_KEY);
        (bytes32 sx, bytes32 sy) = LibP256.derivePub(SESSION_KEY);
        address[] memory anyone = new address[](1);
        anyone[0] = address(0xB0B);
        vm.prank(address(a));
        a.addSession(EastSeaAccount.Key(sx, sy), type(uint128).max, type(uint128).max, 0, anyone);
        (bytes32 gx, bytes32 gy) = LibP256.derivePub(GUARDIAN_KEY);
        EastSeaAccount.Key[] memory guardians = new EastSeaAccount.Key[](1);
        guardians[0] = EastSeaAccount.Key(gx, gy);
        vm.prank(address(a));
        a.setGuardians(guardians, 1, 10 minutes);
        require(
            a.isValidSignature(MSG, signFor(SESSION_KEY, address(a), MSG)) == INVALID,
            "even an unlimited session key never speaks for the account"
        );
        require(
            a.isValidSignature(MSG, signFor(GUARDIAN_KEY, address(a), MSG)) == INVALID,
            "a guardian proposes recovery, not messages"
        );
    }

    function testTheLegacySelectorStillWorks() public {
        EastSeaAccount a = accountOf(A_KEY);
        bytes memory good = signFor(A_KEY, address(a), MSG);
        (bool ok, bytes memory out) =
            address(a).staticcall(abi.encodeWithSelector(LEGACY_MAGIC, abi.encodePacked(MSG), good));
        require(ok && bytes4(out) == LEGACY_MAGIC, "legacy bytes-encoded hash");
        (ok, out) = address(a).staticcall(abi.encodeWithSelector(LEGACY_MAGIC, "", good));
        require(ok && bytes4(out) == INVALID, "legacy with a non-32-byte hash");
        (ok, out) = address(a).staticcall(abi.encodeWithSelector(LEGACY_MAGIC, abi.encodePacked(MSG, MSG), good));
        require(ok && bytes4(out) == INVALID, "legacy with 64 bytes of data");
        (ok, out) = address(a).staticcall(
            abi.encodeWithSelector(LEGACY_MAGIC, abi.encodePacked(MSG), signFor(SESSION_KEY, address(a), MSG))
        );
        require(ok && bytes4(out) == INVALID, "legacy still refuses a non-owner key");
    }

    // ---- B2: receiver hooks and ERC-165 ----

    function testReceiverHooksReturnTheirSelectors() public {
        EastSeaAccount a = accountOf(A_KEY);
        require(a.onERC721Received(address(1), address(2), 3, "") == 0x150b7a02, "721");
        require(a.onERC1155Received(address(1), address(2), 3, 4, "") == 0xf23a6e61, "1155");
        uint256[] memory ids = new uint256[](1);
        uint256[] memory amounts = new uint256[](1);
        require(a.onERC1155BatchReceived(address(1), address(2), ids, amounts, "") == 0xbc197c81, "1155 batch");
    }

    function testSupportsInterface() public {
        EastSeaAccount a = accountOf(A_KEY);
        require(a.supportsInterface(0x01ffc9a7), "ERC-165");
        require(a.supportsInterface(0x1626ba7e), "ERC-1271");
        require(a.supportsInterface(0x150b7a02), "ERC-721 receiver");
        require(a.supportsInterface(0x4e2312e0), "ERC-1155 receiver");
        require(!a.supportsInterface(0xffffffff), "ERC-165 reserved");
        require(!a.supportsInterface(0x12345678), "unknown");
    }

    function truncated(bytes memory b, uint256 n) private pure returns (bytes memory out) {
        out = new bytes(n);
        for (uint256 i = 0; i < n; i++) out[i] = b[i];
    }
}
