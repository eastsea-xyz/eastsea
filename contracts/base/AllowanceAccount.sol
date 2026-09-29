// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.19;

import {P256Fallback} from "./P256Fallback.sol";

/// Immutable-code Base account owned by a raw Secure Enclave P-256 key.
contract AllowanceAccount {
    struct Key {
        bytes32 x;
        bytes32 y;
    }

    struct PackedUserOperation {
        address sender;
        uint256 nonce;
        bytes initCode;
        bytes callData;
        bytes32 accountGasLimits;
        uint256 preVerificationGas;
        bytes32 gasFees;
        bytes paymasterAndData;
        bytes signature;
    }

    struct Session {
        Key key;
        uint128 perPayment;
        uint128 perDay;
        uint128 spent;
        uint128 previousSpent;
        uint64 day;
        uint64 expires;
        uint256 nonce;
        uint256 epoch;
        bool active;
    }

    uint256 private constant ORDER = 0xffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551;
    uint256 private constant HALF_ORDER = ORDER / 2;
    uint64 public constant RECOVERY_DELAY = 48 hours;
    bytes32 private constant OWNER_TAG = keccak256("aether.base.owner.v1");
    bytes32 private constant USER_OP_TAG = keccak256("aether.base.userop.v1");
    bytes32 private constant SESSION_TAG = keccak256("aether.base.session.v1");
    bytes32 private constant RECOVERY_TAG = keccak256("aether.base.recovery.v1");
    bytes4 private constant TRANSFER = 0xa9059cbb;

    address public immutable usdc;
    address public immutable entryPoint;
    Key public owner;
    Key public guardian;
    uint256 public ownerNonce;
    uint256 public userOpNonce;
    uint256 public recoveryNonce;
    uint256 public sessionEpoch;
    uint256 public nextSessionId;
    Key public pendingOwner;
    uint64 public recoveryReadyAt;
    mapping(uint256 => Session) public sessions;
    mapping(uint256 => mapping(address => bool)) public allowedRecipients;
    mapping(bytes32 => bool) public usedSessionKey;

    error InvalidKey();
    error Unauthorized();
    error Expired();
    error InvalidCall();
    error InvalidSession();
    error LimitExceeded();
    error RecoveryNotReady();
    error TokenTransferFailed();

    event OwnerRotated(bytes32 x, bytes32 y);
    event GuardianSet(bytes32 x, bytes32 y);
    event RecoveryProposed(uint256 nonce, uint64 readyAt);
    event RecoveryCancelled();
    event SessionCreated(uint256 indexed id, uint128 perPayment, uint128 perDay, uint64 expires);
    event SessionRevoked(uint256 indexed id);
    event SessionPaid(uint256 indexed id, address indexed recipient, uint256 amount);

    constructor(Key memory initialOwner, address token, address trustedEntryPoint) {
        if (!_validKey(initialOwner) || token == address(0) || trustedEntryPoint == address(0)) revert InvalidKey();
        owner = initialOwner;
        usdc = token;
        entryPoint = trustedEntryPoint;
    }

    modifier onlySelf() {
        if (msg.sender != address(this)) revert Unauthorized();
        _;
    }

    function _same(Key memory a, Key memory b) private pure returns (bool) {
        return a.x == b.x && a.y == b.y;
    }

    function _validKey(Key memory key) private pure returns (bool) {
        return (key.x != 0 || key.y != 0) && P256Fallback.onCurve(uint256(key.x), uint256(key.y));
    }

    function _verify(bytes32 digest, bytes calldata signature, Key memory key) private view returns (bool) {
        return _verifyMemory(digest, signature, key);
    }

    function ownerDigest(address to, uint256 value, bytes calldata data, uint256 nonce, uint64 expiry)
        public
        view
        returns (bytes32)
    {
        return sha256(abi.encode(OWNER_TAG, block.chainid, address(this), nonce, expiry, to, value, keccak256(data)));
    }

    /// Direct owner operation; the relayer pays ETH gas. Self-calls configure policy.
    function execute(address to, uint256 value, bytes calldata data, uint64 expiry, bytes calldata signature)
        external
        returns (bytes memory)
    {
        if (block.timestamp > expiry) revert Expired();
        uint256 nonce = ownerNonce;
        if (!_verify(ownerDigest(to, value, data, nonce, expiry), signature, owner)) revert Unauthorized();
        ownerNonce = nonce + 1;
        return _call(to, value, data);
    }

    function _call(address to, uint256 value, bytes calldata data) private returns (bytes memory result) {
        (bool ok, bytes memory output) = to.call{value: value}(data);
        if (!ok) {
            assembly {
                revert(add(output, 32), mload(output))
            }
        }
        return output;
    }

    function userOpDigest(bytes32 userOpHash, uint256 nonce, uint64 expiry) public view returns (bytes32) {
        return sha256(abi.encode(USER_OP_TAG, block.chainid, address(this), nonce, expiry, userOpHash));
    }

    /// ERC-4337 v0.7 validation. Signature = abi.encode(uint64 expiry, bytes32 r, bytes32 s).
    function validateUserOp(PackedUserOperation calldata op, bytes32 userOpHash, uint256 missingAccountFunds)
        external
        returns (uint256 validationData)
    {
        if (msg.sender != entryPoint) revert Unauthorized();
        if (
            op.sender != address(this) || op.nonce != userOpNonce || op.callData.length < 4
                || bytes4(op.callData[:4]) != this.executeFromEntryPoint.selector || op.signature.length != 96
        ) return 1;
        (uint64 expiry, bytes32 r, bytes32 s) = abi.decode(op.signature, (uint64, bytes32, bytes32));
        bytes memory raw = abi.encodePacked(r, s);
        if (block.timestamp > expiry || !_verifyMemory(userOpDigest(userOpHash, op.nonce, expiry), raw, owner)) {
            return 1;
        }
        userOpNonce++;
        if (missingAccountFunds != 0) {
            (bool paid,) = payable(msg.sender).call{value: missingAccountFunds}("");
            if (!paid) revert TokenTransferFailed();
        }
        return 0;
    }

    function _verifyMemory(bytes32 digest, bytes memory signature, Key memory key) private view returns (bool) {
        if (signature.length != 64) return false;
        bytes32 r;
        bytes32 s;
        assembly {
            r := mload(add(signature, 32))
            s := mload(add(signature, 64))
        }
        if (uint256(r) == 0 || uint256(r) >= ORDER || uint256(s) == 0 || uint256(s) > HALF_ORDER) return false;
        (bool ok, bytes memory result) = address(0x100).staticcall(abi.encodePacked(digest, r, s, key.x, key.y));
        if (ok && result.length == 32) return abi.decode(result, (uint256)) == 1;
        // Anvil/older EVMs have no 0x100 code. Never bypass verification.
        return P256Fallback.verify(digest, r, s, key.x, key.y);
    }

    function executeFromEntryPoint(address to, uint256 value, bytes calldata data) external returns (bytes memory) {
        if (msg.sender != entryPoint) revert Unauthorized();
        return _call(to, value, data);
    }

    function setGuardian(Key calldata replacement) external onlySelf {
        if (
            !_validKey(replacement) || _same(replacement, owner)
                || usedSessionKey[keccak256(abi.encode(replacement.x, replacement.y))]
        ) revert InvalidKey();
        guardian = replacement;
        delete pendingOwner;
        recoveryReadyAt = 0;
        emit GuardianSet(replacement.x, replacement.y);
    }

    /// The app funding flow must use this method and conceal the deposit address
    /// until a separate recovery key is configured. Direct ERC-20 transfers to
    /// the address cannot be prevented by the account contract.
    function fund(uint256 amount) external {
        if (!_validKey(guardian)) revert InvalidKey();
        if (amount == 0) revert InvalidCall();
        _tokenCall(abi.encodeWithSelector(0x23b872dd, msg.sender, address(this), amount));
    }

    function rotateOwner(Key calldata replacement) external onlySelf {
        if (!_validKey(replacement) || _same(replacement, guardian)) revert InvalidKey();
        owner = replacement;
        sessionEpoch++;
        delete pendingOwner;
        recoveryReadyAt = 0;
        emit OwnerRotated(replacement.x, replacement.y);
    }

    function recoveryDigest(Key calldata replacement, uint256 nonce, uint64 expiry) public view returns (bytes32) {
        return sha256(abi.encode(RECOVERY_TAG, block.chainid, address(this), nonce, expiry, replacement));
    }

    function proposeRecovery(Key calldata replacement, uint64 expiry, bytes calldata signature) external {
        if (!_validKey(guardian) || !_validKey(replacement) || _same(replacement, guardian)) revert InvalidKey();
        if (block.timestamp > expiry) revert Expired();
        if (recoveryReadyAt != 0) revert RecoveryNotReady();
        if (!_verify(recoveryDigest(replacement, recoveryNonce, expiry), signature, guardian)) revert Unauthorized();
        pendingOwner = replacement;
        recoveryReadyAt = uint64(block.timestamp + RECOVERY_DELAY);
        emit RecoveryProposed(recoveryNonce++, recoveryReadyAt);
    }

    function cancelRecovery() external onlySelf {
        delete pendingOwner;
        recoveryReadyAt = 0;
        emit RecoveryCancelled();
    }

    function finishRecovery() external {
        if (recoveryReadyAt == 0 || block.timestamp < recoveryReadyAt) revert RecoveryNotReady();
        Key memory replacement = pendingOwner;
        owner = replacement;
        sessionEpoch++;
        delete pendingOwner;
        recoveryReadyAt = 0;
        emit OwnerRotated(replacement.x, replacement.y);
    }

    function createSession(
        Key calldata key,
        uint128 perPayment,
        uint128 perDay,
        uint64 expires,
        address[] calldata recipients
    ) external onlySelf returns (uint256 id) {
        if (
            !_validKey(key) || _same(key, owner) || _same(key, guardian) || perPayment == 0 || perPayment > perDay
                || expires <= block.timestamp || recipients.length == 0 || recipients.length > 16
        ) revert InvalidSession();
        id = ++nextSessionId;
        Session storage session = sessions[id];
        session.key = key;
        session.perPayment = perPayment;
        session.perDay = perDay;
        session.expires = expires;
        session.epoch = sessionEpoch;
        session.active = true;
        usedSessionKey[keccak256(abi.encode(key.x, key.y))] = true;
        for (uint256 i = 0; i < recipients.length; i++) {
            if (recipients[i] == address(0)) revert InvalidSession();
            allowedRecipients[id][recipients[i]] = true;
        }
        emit SessionCreated(id, perPayment, perDay, expires);
    }

    function revokeSession(uint256 id) external onlySelf {
        if (!sessions[id].active) revert InvalidSession();
        sessions[id].active = false;
        emit SessionRevoked(id);
    }

    function sessionDigest(uint256 id, uint256 nonce, uint64 expiry, bytes calldata tokenCall)
        public
        view
        returns (bytes32)
    {
        return sha256(abi.encode(SESSION_TAG, block.chainid, address(this), id, nonce, expiry, keccak256(tokenCall)));
    }

    /// Only canonical USDC transfer(address,uint256) calldata is accepted.
    /// ERC-1271 is absent: its view check cannot record a daily cap.
    function sessionExecute(uint256 id, bytes calldata tokenCall, uint64 expiry, bytes calldata signature) external {
        Session storage session = sessions[id];
        if (!session.active || session.epoch != sessionEpoch) revert InvalidSession();
        if (block.timestamp > expiry || block.timestamp >= session.expires || expiry > session.expires) {
            revert Expired();
        }
        if (tokenCall.length != 68 || bytes4(tokenCall[:4]) != TRANSFER) revert InvalidCall();
        (address recipient, uint256 amount) = abi.decode(tokenCall[4:], (address, uint256));
        if (amount == 0 || amount > type(uint128).max || !allowedRecipients[id][recipient]) revert InvalidCall();
        if (amount > session.perPayment) revert LimitExceeded();
        uint256 nonce = session.nonce;
        if (!_verify(sessionDigest(id, nonce, expiry, tokenCall), signature, session.key)) revert Unauthorized();
        uint64 today = uint64(block.timestamp / 1 days);
        if (today != session.day) {
            session.previousSpent = today == session.day + 1 ? session.spent : 0;
            session.spent = 0;
            session.day = today;
        }
        uint256 used = uint256(session.previousSpent) + session.spent + amount;
        if (used > session.perDay) revert LimitExceeded();
        session.spent += uint128(amount);
        session.nonce = nonce + 1;
        _tokenCall(tokenCall);
        emit SessionPaid(id, recipient, amount);
    }

    function _tokenCall(bytes memory data) private {
        (bool ok, bytes memory result) = usdc.call(data);
        if (!ok || result.length != 32 || !abi.decode(result, (bool))) revert TokenTransferFailed();
    }

    receive() external payable {}
}
