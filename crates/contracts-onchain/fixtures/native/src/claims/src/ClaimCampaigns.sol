// SPDX-License-Identifier: MIT
pragma solidity ^0.8.31;

import {NativeBrake} from "../../common/src/NativeBrake.sol";
import {TransientLock} from "../../common/src/TransientLock.sol";
import {ExactToken, IERC20Min} from "../../common/src/ExactToken.sol";

/// @title ClaimCampaigns: immutable funded Merkle entitlements on one asset
/// @notice One shared instance per exact-transfer ERC-20. Anyone opens a
///         finite campaign with a fixed root, leaf count, refund recipient
///         and deadline (a block height), funding it exactly. A valid leaf
///         pays its fixed account once, whoever submits it. After the
///         deadline (or once fully paid) anyone closes the campaign, which
///         returns the remainder to the fixed refund recipient; after close
///         anyone may prune its replay bitmap. Nobody can edit a root,
///         redirect a claim, sweep early, upgrade or charge a fee.
/// @dev Design: native/claims/DESIGN.md. Storage per live campaign is three
///      words (C0 root, C1 refund|deadline|leafCount, C2 remaining|funded)
///      plus one bitmap word per touched 256-index group.
///
///      Canonical leaf/proof format (also implemented by tools/claims_tree.py):
///        leaf = keccak256(abi.encode(LEAF_DOMAIN, chainid, instance,
///                                    campaignId, index, account, amount))
///        node = keccak256(left || right)   (64 bytes, never a leaf preimage)
///      The tree is positional: a leaf sits at position `index`, the proof
///      has exactly ceil(log2(leafCount)) siblings and the index bits pick
///      left/right at each level. Empty positions hold bytes32(0). One index
///      therefore has exactly one valid leaf in a given root.
contract ClaimCampaigns is NativeBrake, TransientLock {
    uint256 public constant MAX_LEAVES = 1 << 16;
    uint256 public constant MAX_DEPTH = 16;
    uint256 public constant MAX_PRUNE_WORDS = 8;
    bytes32 public constant LEAF_DOMAIN = keccak256("eastsea.native.claims.leaf.v1");

    /// @notice The single exact-transfer asset of this instance.
    address public immutable token;
    /// @notice Token code hash at deployment; a change trips the entry brake.
    bytes32 public immutable tokenCodeHash;

    struct Meta {
        uint64 nextCampaign;
        uint64 brakeSince;
        uint128 outstanding;
    }

    struct Campaign {
        bytes32 root; // C0
        address refundRecipient; // C1
        uint64 deadline; // C1, last block height at which claims are accepted
        uint32 leafCount; // C1
        uint128 remaining; // C2
        uint128 initiallyFunded; // C2, keeps C2 nonzero while live
    }

    Meta private _meta;
    mapping(uint256 => Campaign) private _campaigns;
    mapping(uint256 => mapping(uint256 => uint256)) private _used;

    event CampaignCreated(
        uint256 indexed id,
        address indexed creator,
        bytes32 root,
        uint256 funding,
        uint64 deadline,
        uint32 leafCount,
        address refundRecipient,
        bytes32 dataHash
    );
    event Claimed(uint256 indexed id, uint256 indexed index, address indexed account, uint256 amount);
    event Closed(uint256 indexed id, address indexed refundRecipient, uint256 amount);
    event Pruned(uint256 indexed id, uint256 startWord, uint256 count);

    error TokenHasNoCode();
    error UnexpectedCampaignId(uint256 expected, uint256 actual);
    error ZeroRoot();
    error BadLeafCount(uint256 leafCount);
    error BadRefundRecipient();
    error BadDeadline();
    error ZeroAmount();
    error OutstandingOverflow();
    error CampaignNotLive(uint256 id);
    error ClaimWindowClosed(uint256 id, uint64 deadline);
    error IndexOutOfRange(uint256 index, uint32 leafCount);
    error BadProofLength(uint256 got, uint256 want);
    error InvalidProof();
    error AlreadyClaimed(uint256 id, uint256 index);
    error InsufficientBacking(uint256 remaining, uint256 amount);
    error BackingShortfall(uint256 balance, uint256 outstanding);
    error NotClosable(uint256 id);
    error NotClosed(uint256 id);
    error BadPruneCount(uint256 count);

    constructor(address token_, bytes32 brakeDocSha256) NativeBrake(brakeDocSha256) {
        if (token_.code.length == 0) revert TokenHasNoCode();
        token = token_;
        tokenCodeHash = token_.codehash;
        _meta.nextCampaign = 1;
    }

    // ------------------------------------------------------------------ entry

    /// @notice Open and exactly fund a campaign.
    /// @param expectedId must equal `nextCampaign()`; the leaf commitment binds
    ///        the id, so a concurrent creator makes this call fail before any
    ///        token moves rather than silently creating an unclaimable root.
    /// @param dataHash  issuer-chosen hash of the published leaf dataset
    ///        (tools/claims_tree.py: sha256 of the canonical CSV). Event only.
    function create(
        uint256 expectedId,
        bytes32 root,
        uint32 leafCount,
        address refundRecipient,
        uint64 deadline,
        uint128 amount,
        bytes32 dataHash
    ) external lock returns (uint256 id) {
        _requireEntryOpen();
        Meta memory m = _meta;
        id = m.nextCampaign;
        if (expectedId != id) revert UnexpectedCampaignId(expectedId, id);
        if (root == bytes32(0)) revert ZeroRoot();
        if (leafCount == 0 || leafCount > MAX_LEAVES) revert BadLeafCount(leafCount);
        if (refundRecipient == address(0) || refundRecipient == address(this)) revert BadRefundRecipient();
        if (deadline <= block.number) revert BadDeadline();
        if (amount == 0) revert ZeroAmount();
        if (uint256(m.outstanding) + amount > type(uint128).max) revert OutstandingOverflow();

        _meta = Meta({nextCampaign: m.nextCampaign + 1, brakeSince: m.brakeSince, outstanding: m.outstanding + amount});
        _campaigns[id] = Campaign({
            root: root,
            refundRecipient: refundRecipient,
            deadline: deadline,
            leafCount: leafCount,
            remaining: amount,
            initiallyFunded: amount
        });

        ExactToken.pullExact(token, msg.sender, amount);
        emit CampaignCreated(id, msg.sender, root, amount, deadline, leafCount, refundRecipient, dataHash);
    }

    // ------------------------------------------------------------------ exits

    /// @notice Pay a valid leaf to its fixed account. Anyone may submit.
    function claim(uint256 id, uint256 index, address account, uint128 amount, bytes32[] calldata proof) external lock {
        Campaign storage c = _campaigns[id];
        _checkLeaf(c, id, index, account, amount, proof);
        _consumeBit(id, index);
        uint128 remaining = c.remaining;
        if (amount > remaining) revert InsufficientBacking(remaining, amount);
        c.remaining = remaining - amount;
        uint128 owed = _meta.outstanding - amount;
        _meta.outstanding = owed;

        ExactToken.pushExact(token, account, amount);
        _requireBacked(owed);
        emit Claimed(id, index, account, amount);
    }

    /// @notice Close an ended (past deadline) or fully paid campaign and send
    ///         the remainder to its fixed refund recipient. Anyone may call.
    ///         A failed refund transfer reverts and leaves the campaign intact.
    function close(uint256 id) external lock {
        Campaign storage c = _campaigns[id];
        if (c.root == bytes32(0)) revert CampaignNotLive(id);
        uint128 refund = c.remaining;
        if (block.number <= c.deadline && refund != 0) revert NotClosable(id);
        address to = c.refundRecipient;

        delete _campaigns[id];
        uint128 owed = _meta.outstanding - refund;
        _meta.outstanding = owed;

        if (refund != 0) {
            ExactToken.pushExact(token, to, refund);
            _requireBacked(owed);
        }
        emit Closed(id, to, refund);
    }

    /// @notice Clear up to eight bitmap words of a closed campaign. Never
    ///         touches a live campaign. Clearing refunds no burned state fee.
    function prune(uint256 id, uint256 startWord, uint256 count) external {
        if (count == 0 || count > MAX_PRUNE_WORDS) revert BadPruneCount(count);
        if (id == 0 || id >= _meta.nextCampaign) revert NotClosed(id);
        if (_campaigns[id].root != bytes32(0)) revert NotClosed(id);
        mapping(uint256 => uint256) storage used = _used[id];
        for (uint256 i; i < count; i++) {
            delete used[startWord + i];
        }
        emit Pruned(id, startWord, count);
    }

    // ------------------------------------------------------------------ views

    function nextCampaign() external view returns (uint256) {
        return _meta.nextCampaign;
    }

    function outstanding() external view returns (uint256) {
        return _meta.outstanding;
    }

    function campaign(uint256 id) external view returns (Campaign memory) {
        return _campaigns[id];
    }

    function isClaimed(uint256 id, uint256 index) external view returns (bool) {
        return _used[id][index >> 8] & (1 << (index & 0xff)) != 0;
    }

    function bitmapWord(uint256 id, uint256 word) external view returns (uint256) {
        return _used[id][word];
    }

    /// @notice Canonical leaf commitment (domain: chain, instance, campaign).
    function leafHash(uint256 id, uint256 index, address account, uint256 amount) public view returns (bytes32) {
        return keccak256(abi.encode(LEAF_DOMAIN, block.chainid, address(this), id, index, account, amount));
    }

    /// @notice Required proof length: ceil(log2(leafCount)), 0 for one leaf.
    function proofDepth(uint256 leafCount) public pure returns (uint256 depth) {
        while ((uint256(1) << depth) < leafCount) depth++;
    }

    /// @notice Lets a wallet check a leaf/proof locally before paying a fee.
    function verify(uint256 id, uint256 index, address account, uint256 amount, bytes32[] calldata proof)
        external
        view
        returns (bool)
    {
        Campaign storage c = _campaigns[id];
        if (c.root == bytes32(0) || index >= c.leafCount) return false;
        if (proof.length != proofDepth(c.leafCount)) return false;
        return _root(leafHash(id, index, account, amount), index, proof) == c.root;
    }

    // --------------------------------------------------------------- internal

    function _checkLeaf(
        Campaign storage c,
        uint256 id,
        uint256 index,
        address account,
        uint128 amount,
        bytes32[] calldata proof
    ) private view {
        bytes32 root = c.root;
        if (root == bytes32(0)) revert CampaignNotLive(id);
        uint64 deadline = c.deadline;
        if (block.number > deadline) revert ClaimWindowClosed(id, deadline);
        uint32 leafCount = c.leafCount;
        if (index >= leafCount) revert IndexOutOfRange(index, leafCount);
        if (amount == 0) revert ZeroAmount();
        uint256 depth = proofDepth(leafCount);
        if (proof.length != depth) revert BadProofLength(proof.length, depth);
        if (_root(leafHash(id, index, account, amount), index, proof) != root) revert InvalidProof();
    }

    function _consumeBit(uint256 id, uint256 index) private {
        uint256 word = index >> 8;
        uint256 bit = 1 << (index & 0xff);
        uint256 bits = _used[id][word];
        if (bits & bit != 0) revert AlreadyClaimed(id, index);
        _used[id][word] = bits | bit;
    }

    function _root(bytes32 h, uint256 index, bytes32[] calldata proof) private pure returns (bytes32) {
        for (uint256 i; i < proof.length; i++) {
            bytes32 s = proof[i];
            h = (index >> i) & 1 == 0 ? _node(h, s) : _node(s, h);
        }
        return h;
    }

    function _node(bytes32 a, bytes32 b) private pure returns (bytes32 r) {
        assembly ("memory-safe") {
            mstore(0x00, a)
            mstore(0x20, b)
            r := keccak256(0x00, 0x40)
        }
    }

    /// @dev Exits never consume another campaign's backing.
    function _requireBacked(uint256 outstanding_) private view {
        uint256 bal = IERC20Min(token).balanceOf(address(this));
        if (bal < outstanding_) revert BackingShortfall(bal, outstanding_);
    }

    function _brakeReasons() internal view override returns (uint8 r) {
        if (token.codehash != tokenCodeHash) r |= REASON_CODE_CHANGED;
        if (IERC20Min(token).balanceOf(address(this)) < _meta.outstanding) r |= REASON_DEFICIT;
        if (_meta.nextCampaign == type(uint64).max) r |= REASON_IDS_EXHAUSTED;
    }

    function _brakeSince() internal view override returns (uint64) {
        return _meta.brakeSince;
    }

    function _setBrakeSince(uint64 h) internal override {
        _meta.brakeSince = h;
    }

    function _brakeUri() internal pure override returns (string memory) {
        return "native/claims/SECURITY.md#brake";
    }
}
