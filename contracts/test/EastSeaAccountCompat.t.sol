// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.19;

import {LibP256} from "./EastSeaVault.t.sol";
import {EvmCompatToken, EvmCompatNft} from "./fixtures/EvmCompat.sol";

interface VmCompat {
    function prank(address) external;
    function chainId(uint256) external;
    function etch(address, bytes calldata) external;
    function readFile(string calldata) external view returns (string memory);
    function parseBytes(string calldata) external pure returns (bytes memory);
    function expectRevert() external;
    function addr(uint256) external returns (address);
    function sign(uint256, bytes32) external returns (uint8, bytes32, bytes32);
}

interface IEvmCompatAccount {
    function isValidSignature(bytes32 hash, bytes calldata signature) external view returns (bytes4);
    function onERC721Received(address, address, uint256, bytes calldata) external returns (bytes4);
    function onERC1155Received(address, address, uint256, uint256, bytes calldata) external returns (bytes4);
    function onERC1155BatchReceived(address, address, uint256[] calldata, uint256[] calldata, bytes calldata)
        external
        returns (bytes4);
    function supportsInterface(bytes4 interfaceId) external view returns (bool);
}

interface IEvmCompatPermit2 {
    struct TokenPermissions {
        address token;
        uint256 amount;
    }

    struct PermitTransferFrom {
        TokenPermissions permitted;
        uint256 nonce;
        uint256 deadline;
    }

    struct SignatureTransferDetails {
        address to;
        uint256 requestedAmount;
    }

    function permitTransferFrom(
        PermitTransferFrom calldata permit,
        SignatureTransferDetails calldata transferDetails,
        address owner,
        bytes calldata signature
    ) external;
    function DOMAIN_SEPARATOR() external view returns (bytes32);
    function nonceBitmap(address owner, uint256 wordPos) external view returns (uint256);
}

/// Integration coverage over the frozen on-chain bytes, rather than compiling
/// a fresh account/Permit2 implementation. The account's actual code is the
/// EIP-7702 designator, and Osaka resolves it to the pinned code at 0x7702.
/// Signatures use real P256VERIFY at 0x100; the account wrapper and both token
/// permit domains are reconstructed independently of their implementation.
contract EastSeaAccountCompatTest {
    VmCompat constant vm = VmCompat(address(uint160(uint256(keccak256("hevm cheat code")))));
    address constant ACCOUNT_IMPL = address(0x7702);
    address constant PERMIT2 = 0x000000000022D473030F116dDEE9F6B43aC78BA3;
    address constant RECIPIENT = address(0xBEEF);
    uint256 constant OWNER_KEY = 0xa11ce;
    uint256 constant WRONG_KEY = 0xb0b;
    uint256 constant P256_N = 0xffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551;
    bytes32 constant ACCOUNT_CODE_HASH = 0xdeaca4e6cc9787c233aeec5034a8884cf5ced4c6899299b3e76027a03f85288a;
    bytes32 constant PERMIT2_CODE_HASH = 0xc67d1657868aa5146eaf24fb879fb1fdec3d2d493b3683a61c9c2f4fb2851131;
    bytes32 constant MESSAGE = keccak256("a standard contract's message");

    EvmCompatToken token;
    EvmCompatNft nft;
    address account;
    uint256 signingNonce = 0x1000;

    function setUp() public {
        vm.chainId(7781);
        bytes memory runtime = _runtime("../crates/execution/src/aether_account_v2.bin.hex");
        require(keccak256(runtime) == ACCOUNT_CODE_HASH, "frozen v2 bytes");
        vm.etch(ACCOUNT_IMPL, runtime);
        account = _addressOf(OWNER_KEY);
        vm.etch(account, abi.encodePacked(hex"ef0100", ACCOUNT_IMPL));
        require(keccak256(account.code) == keccak256(abi.encodePacked(hex"ef0100", ACCOUNT_IMPL)), "delegation");

        runtime = _runtime("../crates/execution/src/permit2.bin.hex");
        require(keccak256(runtime) == PERMIT2_CODE_HASH, "canonical Permit2 bytes");
        vm.etch(PERMIT2, runtime);
        token = new EvmCompatToken();
        nft = new EvmCompatNft();
        token.mint(account, 100);
        vm.prank(account);
        token.approve(PERMIT2, type(uint256).max);
    }

    function testPinnedDelegatedAccountAcceptsP256AndRejectsWrongSignatures() public {
        IEvmCompatAccount owner = IEvmCompatAccount(account);
        bytes memory signature = _signFor(OWNER_KEY, MESSAGE);
        require(owner.isValidSignature(MESSAGE, signature) == 0x1626ba7e, "P256 owner signature");
        require(owner.isValidSignature(keccak256("wrong hash"), signature) == 0xffffffff, "wrong hash");
        require(owner.isValidSignature(MESSAGE, _signFor(WRONG_KEY, MESSAGE)) == 0xffffffff, "wrong signer");
        require(owner.isValidSignature(MESSAGE, "") == 0xffffffff, "malformed signature");
    }

    function testSafeMintAndSafeTransferToDelegatedAccount() public {
        require(account.code.length == 23, "actual 7702 designator");
        nft.safeMint(account, 1);
        require(nft.ownerOf(1) == account, "safe mint");
        address sender = address(0xCAFE);
        nft.safeMint(sender, 2);
        vm.prank(sender);
        nft.safeTransferFrom(sender, account, 2);
        require(nft.ownerOf(2) == account, "safe transfer");
    }

    function testDelegatedReceiverHooksAndErc165() public {
        IEvmCompatAccount owner = IEvmCompatAccount(account);
        require(owner.onERC721Received(address(this), address(0), 1, "") == 0x150b7a02, "721 hook");
        require(owner.onERC1155Received(address(this), address(0), 1, 2, "") == 0xf23a6e61, "1155 hook");
        uint256[] memory ids = new uint256[](2);
        uint256[] memory amounts = new uint256[](2);
        ids[0] = 1;
        ids[1] = 2;
        amounts[0] = 3;
        amounts[1] = 4;
        require(
            owner.onERC1155BatchReceived(address(this), address(0), ids, amounts, "") == 0xbc197c81, "1155 batch hook"
        );
        require(owner.supportsInterface(0x01ffc9a7), "ERC165");
        require(owner.supportsInterface(0x1626ba7e), "ERC1271");
        require(owner.supportsInterface(0x150b7a02), "ERC721 receiver");
        require(owner.supportsInterface(0x4e2312e0), "ERC1155 receiver");
        require(!owner.supportsInterface(0xffffffff), "reserved interface");
        require(!owner.supportsInterface(0x12345678), "unknown interface");
    }

    function testLegacyPinnedRuntimeCannotVerifyOrReceive() public {
        bytes memory signature = _signFor(OWNER_KEY, MESSAGE);
        vm.etch(ACCOUNT_IMPL, _runtime("../crates/execution/src/aether_account.bin.hex"));
        (bool ok, bytes memory out) = account.staticcall(abi.encodeWithSelector(bytes4(0x1626ba7e), MESSAGE, signature));
        require(!ok || out.length < 32 || bytes4(out) != 0x1626ba7e, "legacy cannot validate");
        vm.expectRevert();
        nft.safeMint(account, 1);
        require(nft.ownerOf(1) == address(0), "failed safe mint rolls back");
        address sender = address(0xCAFE);
        nft.safeMint(sender, 2);
        vm.prank(sender);
        vm.expectRevert();
        nft.safeTransferFrom(sender, account, 2);
        require(nft.ownerOf(2) == sender, "failed safe transfer rolls back");
    }

    function testPermitWith1271FallbackConsumesOnlyValidNonceAndRejectsReplay() public {
        uint256 deadline = block.timestamp + 1 hours;
        bytes32 digest = _tokenPermitDigest(account, address(this), 40, 0, deadline);
        require(token.DOMAIN_SEPARATOR() == _tokenDomain(), "independent token domain");
        bytes memory signature = _signFor(OWNER_KEY, digest);
        token.permit(account, address(this), 40, deadline, signature);
        require(token.nonces(account) == 1, "nonce consumed");
        require(token.allowance(account, address(this)) == 40, "approval");
        token.transferFrom(account, RECIPIENT, 25);
        require(token.balanceOf(RECIPIENT) == 25, "relayed transfer");
        require(token.allowance(account, address(this)) == 15, "allowance spent");
        vm.expectRevert();
        token.permit(account, address(this), 40, deadline, signature);
        require(token.nonces(account) == 1, "replay cannot consume nonce");
        require(token.allowance(account, address(this)) == 15, "replay cannot reset approval");
    }

    function testPermitWith1271FallbackRejectsWrongSignerHashAndSpenderBeforeEffects() public {
        uint256 deadline = block.timestamp + 1 hours;
        bytes32 digest = _tokenPermitDigest(account, address(this), 40, 0, deadline);
        bytes memory signature = _signFor(WRONG_KEY, digest);
        vm.expectRevert();
        token.permit(account, address(this), 40, deadline, signature);
        signature = _signFor(OWNER_KEY, _tokenPermitDigest(account, address(this), 41, 0, deadline));
        vm.expectRevert();
        token.permit(account, address(this), 40, deadline, signature);
        signature = _signFor(OWNER_KEY, digest);
        vm.expectRevert();
        token.permit(account, address(0xBAD), 40, deadline, signature);
        require(token.nonces(account) == 0, "invalid signatures cannot consume nonce");
        require(token.allowance(account, address(this)) == 0, "invalid signatures cannot approve");
        require(token.allowance(account, address(0xBAD)) == 0, "invalid spender cannot approve");
    }

    function testLegacyAccountCannotUse1271PermitFallback() public {
        uint256 deadline = block.timestamp + 1 hours;
        bytes memory signature = _signFor(OWNER_KEY, _tokenPermitDigest(account, address(this), 40, 0, deadline));
        vm.etch(ACCOUNT_IMPL, _runtime("../crates/execution/src/aether_account.bin.hex"));
        vm.expectRevert();
        token.permit(account, address(this), 40, deadline, signature);
        require(token.nonces(account) == 0, "legacy failed permit cannot consume nonce");
        require(token.allowance(account, address(this)) == 0, "legacy failed permit cannot approve");
    }

    function testStandardErc2612Secp256k1PermitRemainsUsable() public {
        uint256 key = 0x1234;
        address owner = vm.addr(key);
        uint256 deadline = block.timestamp + 1 hours;
        bytes32 digest = _tokenPermitDigest(owner, address(this), 40, 0, deadline);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(key, digest);
        token.permit(owner, address(this), 40, deadline, v, r, s);
        require(token.nonces(owner) == 1, "standard nonce");
        require(token.allowance(owner, address(this)) == 40, "standard approval");
        vm.expectRevert();
        token.permit(owner, address(this), 40, deadline, v, r, s);
        require(token.nonces(owner) == 1, "standard replay refused");
    }

    function testCanonicalPermit2CodeAndAlternateChainDomain() public {
        require(PERMIT2.codehash == PERMIT2_CODE_HASH, "canonical runtime hash");
        require(IEvmCompatPermit2(PERMIT2).DOMAIN_SEPARATOR() == _permit2Domain(), "chain7781 domain");
        bytes32 otherChainDomain = _permit2Domain();
        vm.chainId(1);
        require(IEvmCompatPermit2(PERMIT2).DOMAIN_SEPARATOR() == _permit2Domain(), "cached chain1 domain");
        require(_permit2Domain() != otherChainDomain, "domain changes with chain");
    }

    function testCanonicalPermit2TransferWith1271SignerAndReplayProtection() public {
        IEvmCompatPermit2.PermitTransferFrom memory permit = _permit(513);
        bytes memory signature = _signFor(OWNER_KEY, _permit2Digest(permit, address(this)));
        _permit2Transfer(permit, signature);
        require(token.balanceOf(RECIPIENT) == 25 && token.balanceOf(account) == 75, "Permit2 token transfer");
        require(IEvmCompatPermit2(PERMIT2).nonceBitmap(account, 2) == 2, "unordered nonce consumed");
        vm.expectRevert();
        _permit2Transfer(permit, signature);
        require(token.balanceOf(RECIPIENT) == 25, "Permit2 replay cannot transfer");
        require(IEvmCompatPermit2(PERMIT2).nonceBitmap(account, 2) == 2, "replay keeps nonce consumed");
    }

    function testCanonicalPermit2RejectsWrongSignerHashAndSpenderBeforeEffects() public {
        IEvmCompatPermit2.PermitTransferFrom memory permit = _permit(513);
        bytes memory signature = _signFor(WRONG_KEY, _permit2Digest(permit, address(this)));
        vm.expectRevert();
        _permit2Transfer(permit, signature);
        permit.permitted.amount = 41;
        signature = _signFor(OWNER_KEY, _permit2Digest(permit, address(this)));
        permit.permitted.amount = 40;
        vm.expectRevert();
        _permit2Transfer(permit, signature);
        signature = _signFor(OWNER_KEY, _permit2Digest(permit, address(this)));
        vm.prank(address(0xBAD));
        vm.expectRevert();
        _permit2Transfer(permit, signature);
        require(token.balanceOf(RECIPIENT) == 0 && token.balanceOf(account) == 100, "invalid permit cannot transfer");
        require(IEvmCompatPermit2(PERMIT2).nonceBitmap(account, 2) == 0, "invalid permit cannot consume nonce");
        _permit2Transfer(permit, signature);
        require(token.balanceOf(RECIPIENT) == 25, "valid permit works after invalid attempts");
    }

    function testCanonicalPermit2RejectsSignatureFromAnotherChain() public {
        IEvmCompatPermit2.PermitTransferFrom memory permit = _permit(513);
        vm.chainId(1);
        bytes memory signature = _signFor(OWNER_KEY, _permit2Digest(permit, address(this)));
        vm.chainId(7781);
        vm.expectRevert();
        _permit2Transfer(permit, signature);
        require(IEvmCompatPermit2(PERMIT2).nonceBitmap(account, 2) == 0, "foreign chain cannot consume nonce");
        require(token.balanceOf(RECIPIENT) == 0, "foreign chain cannot transfer");
    }

    function testLegacyAccountCannotUseCanonicalPermit2() public {
        IEvmCompatPermit2.PermitTransferFrom memory permit = _permit(513);
        bytes memory signature = _signFor(OWNER_KEY, _permit2Digest(permit, address(this)));
        vm.etch(ACCOUNT_IMPL, _runtime("../crates/execution/src/aether_account.bin.hex"));
        vm.expectRevert();
        _permit2Transfer(permit, signature);
        require(token.balanceOf(RECIPIENT) == 0, "legacy failed permit cannot transfer");
        require(IEvmCompatPermit2(PERMIT2).nonceBitmap(account, 2) == 0, "legacy failed permit cannot consume nonce");
    }

    function testMissingPermit2CannotPerformTransfer() public {
        IEvmCompatPermit2.PermitTransferFrom memory permit = _permit(513);
        bytes memory signature = _signFor(OWNER_KEY, _permit2Digest(permit, address(this)));
        vm.etch(PERMIT2, "");
        // An empty-code EVM call succeeds but cannot perform a transfer.
        // Use a low-level call: Solidity's high-level code-existence guard
        // would revert in this test's own frame before calling Permit2.
        (bool ok, bytes memory out) = PERMIT2.call(
            abi.encodeCall(
                IEvmCompatPermit2.permitTransferFrom,
                (permit, IEvmCompatPermit2.SignatureTransferDetails(RECIPIENT, 25), account, signature)
            )
        );
        require(ok && out.length == 0, "empty-code EVM call");
        require(
            token.balanceOf(RECIPIENT) == 0 && token.balanceOf(account) == 100, "absent predeploy transfers nothing"
        );
    }

    function _permit(uint256 nonce) private view returns (IEvmCompatPermit2.PermitTransferFrom memory) {
        return IEvmCompatPermit2.PermitTransferFrom(
            IEvmCompatPermit2.TokenPermissions(address(token), 40), nonce, block.timestamp + 1 hours
        );
    }

    function _permit2Transfer(IEvmCompatPermit2.PermitTransferFrom memory permit, bytes memory signature) private {
        IEvmCompatPermit2(PERMIT2)
            .permitTransferFrom(permit, IEvmCompatPermit2.SignatureTransferDetails(RECIPIENT, 25), account, signature);
    }

    function _tokenDomain() private view returns (bytes32) {
        return keccak256(
            abi.encode(
                keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
                keccak256("EvmCompatToken"),
                keccak256("1"),
                block.chainid,
                address(token)
            )
        );
    }

    function _tokenPermitDigest(address owner, address spender, uint256 amount, uint256 nonce, uint256 deadline)
        private
        view
        returns (bytes32)
    {
        bytes32 contents = keccak256(
            abi.encode(
                keccak256("Permit(address owner,address spender,uint256 value,uint256 nonce,uint256 deadline)"),
                owner,
                spender,
                amount,
                nonce,
                deadline
            )
        );
        return keccak256(abi.encodePacked("\x19\x01", _tokenDomain(), contents));
    }

    function _permit2Domain() private view returns (bytes32) {
        return keccak256(
            abi.encode(
                keccak256("EIP712Domain(string name,uint256 chainId,address verifyingContract)"),
                keccak256("Permit2"),
                block.chainid,
                PERMIT2
            )
        );
    }

    function _permit2Digest(IEvmCompatPermit2.PermitTransferFrom memory permit, address spender)
        private
        view
        returns (bytes32)
    {
        bytes32 permissions = keccak256(
            abi.encode(
                keccak256("TokenPermissions(address token,uint256 amount)"),
                permit.permitted.token,
                permit.permitted.amount
            )
        );
        bytes32 contents = keccak256(
            abi.encode(
                keccak256(
                    "PermitTransferFrom(TokenPermissions permitted,address spender,uint256 nonce,uint256 deadline)TokenPermissions(address token,uint256 amount)"
                ),
                permissions,
                spender,
                permit.nonce,
                permit.deadline
            )
        );
        return keccak256(abi.encodePacked("\x19\x01", _permit2Domain(), contents));
    }

    function _signFor(uint256 key, bytes32 hash) private returns (bytes memory) {
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
        bytes32 digest = sha256(abi.encodePacked("\x19\x01", domain, contents));
        (bytes32 r, bytes32 s) = LibP256.sign(key, digest, signingNonce++);
        if (uint256(s) > P256_N / 2) s = bytes32(P256_N - uint256(s));
        (bytes32 x, bytes32 y) = LibP256.derivePub(key);
        return abi.encodePacked(r, s, x, y);
    }

    function _addressOf(uint256 key) private view returns (address) {
        (bytes32 x, bytes32 y) = LibP256.derivePub(key);
        bytes1 prefix = uint256(y) & 1 == 0 ? bytes1(0x02) : bytes1(0x03);
        return address(uint160(uint256(keccak256(abi.encodePacked(uint8(1), prefix, x)))));
    }

    function _runtime(string memory path) private view returns (bytes memory) {
        bytes memory raw = bytes(vm.readFile(path));
        uint256 length = raw.length;
        while (length != 0 && uint8(raw[length - 1]) <= 0x20) length--;
        bytes memory prefixed = new bytes(length + 2);
        prefixed[0] = "0";
        prefixed[1] = "x";
        for (uint256 i = 0; i < length; i++) {
            prefixed[i + 2] = raw[i];
        }
        return vm.parseBytes(string(prefixed));
    }
}
