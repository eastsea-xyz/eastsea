// SPDX-License-Identifier: MIT
pragma solidity ^0.8.31;

import {NativeBrake} from "../../common/src/NativeBrake.sol";
import {TransientLock} from "../../common/src/TransientLock.sol";
import {ExactToken, IERC20Min} from "../../common/src/ExactToken.sol";

/// @title EscrowBook: bounded two-party escrow with explicit silence policy
/// @notice A buyer funds a deal for a named seller with an acceptance
///         deadline, a delivery deadline, a "what happens on silence" policy
///         and a terms hash. The seller accepts (or the offer lapses and the
///         buyer gets everything back). Once accepted: the buyer can release,
///         the seller can refund, either can propose a split that only the
///         other can accept, and at the deadline anyone applies the agreed
///         silence policy. In an instance with a named arbiter, either party
///         may dispute before the delivery deadline; the arbiter then has a
///         fixed window to split the funds between the two parties only, and
///         after it anyone applies the silence policy. Each party's award is
///         pulled separately to its fixed address by anyone.
/// @dev Design: native/escrow/DESIGN.md. Immutable per instance: asset
///      (address(0) = native DBLN), arbiter (address(0) = none), ruling
///      window, public specification hash. No owner, fee, proxy or rescue.
///      Storage per live deal: four words (S0-S3) and, only after the first
///      proposal, a fifth (S4). Everything is deleted after both payouts.
contract EscrowBook is NativeBrake, TransientLock {
    uint8 internal constant NONE = 0;
    uint8 internal constant OFFERED = 1;
    uint8 internal constant ACCEPTED = 2;
    uint8 internal constant DISPUTED = 3;
    uint8 internal constant RESOLVED = 4;

    uint8 internal constant PAID_SELLER = 1;
    uint8 internal constant PAID_BUYER = 2;

    /// @notice Silence policy: what anyone may apply at the deadline.
    uint8 public constant BUYER_REFUND = 0;
    uint8 public constant SELLER_PAYMENT = 1;

    uint8 internal constant ROLE_BUYER = 1;
    uint8 internal constant ROLE_SELLER = 2;

    address public immutable asset;
    address public immutable arbiter;
    uint64 public immutable rulingDuration;
    bytes32 public immutable specHash;
    bytes32 public immutable assetCodeHash;

    struct Global {
        uint128 totalLiability;
        uint64 nextId;
        uint64 brakeSince;
    }

    struct Deal {
        address buyer; // S0
        uint64 clock; // S0: acceptBy while offered, rulingBy once disputed
        uint8 phase; // S0
        uint8 paid; // S0: consumed-award flags
        address seller; // S1
        uint64 deliverBy; // S1
        uint8 policy; // S1
        uint128 amount; // S2
        uint128 sellerAward; // S2
        bytes32 termsHash; // S3
    }

    struct Proposal {
        uint128 sellerAward; // S4
        uint64 round;
        uint8 proposer;
    }

    Global private _g;
    mapping(uint256 => Deal) private _deals;
    mapping(uint256 => Proposal) private _proposals;

    event Funded(
        uint256 indexed id,
        address indexed buyer,
        address indexed seller,
        uint256 amount,
        uint64 acceptBy,
        uint64 deliverBy,
        uint8 policy,
        bytes32 termsHash
    );
    event Accepted(uint256 indexed id, address indexed seller);
    event Proposed(uint256 indexed id, address indexed proposer, uint256 sellerAward, uint64 round);
    event Disputed(uint256 indexed id, address indexed by, uint64 rulingBy);
    event Resolved(uint256 indexed id, uint256 sellerAward);
    event Paid(uint256 indexed id, address indexed to, uint256 amount);

    error AssetHasNoCode();
    error BadArbiterConfig();
    error BadParty();
    error ZeroAmount();
    error LiabilityOverflow();
    error BadDeadlines();
    error BadPolicy();
    error ZeroTerms();
    error BadFunding();
    error UnknownDeal(uint256 id);
    error WrongPhase(uint256 id, uint8 phase);
    error NotAuthorized();
    error TooLate();
    error TooEarly();
    error AwardTooLarge(uint256 award, uint256 amount);
    error StaleProposal();
    error NoArbiter();
    error AlreadyPaid();
    error BackingShortfall(uint256 custody, uint256 liability);

    constructor(address asset_, address arbiter_, uint64 rulingDuration_, bytes32 specHash_, bytes32 brakeDocSha256)
        NativeBrake(brakeDocSha256)
    {
        if (asset_ != address(0) && asset_.code.length == 0) revert AssetHasNoCode();
        if ((arbiter_ == address(0)) != (rulingDuration_ == 0)) revert BadArbiterConfig();
        asset = asset_;
        arbiter = arbiter_;
        rulingDuration = rulingDuration_;
        specHash = specHash_;
        assetCodeHash = asset_ == address(0) ? bytes32(0) : asset_.codehash;
        _g.nextId = 1;
    }

    // ------------------------------------------------------------------ entry

    /// @notice Buyer funds a deal. Native asset: send exactly `amount` as value.
    function create(address seller, uint128 amount, uint64 acceptBy, uint64 deliverBy, uint8 policy, bytes32 termsHash)
        external
        payable
        lock
        returns (uint256 id)
    {
        _requireEntryOpen();
        address buyer = msg.sender;
        if (seller == address(0) || seller == buyer || seller == address(this)) revert BadParty();
        if (arbiter != address(0) && (buyer == arbiter || seller == arbiter)) revert BadParty();
        if (amount == 0) revert ZeroAmount();
        if (!(block.timestamp < acceptBy && acceptBy < deliverBy)) revert BadDeadlines();
        if (policy > SELLER_PAYMENT) revert BadPolicy();
        if (termsHash == bytes32(0)) revert ZeroTerms();
        Global memory g = _g;
        if (uint256(g.totalLiability) + amount > type(uint128).max) revert LiabilityOverflow();

        id = g.nextId;
        _g = Global({totalLiability: g.totalLiability + amount, nextId: g.nextId + 1, brakeSince: g.brakeSince});
        _deals[id] = Deal({
            buyer: buyer,
            clock: acceptBy,
            phase: OFFERED,
            paid: 0,
            seller: seller,
            deliverBy: deliverBy,
            policy: policy,
            amount: amount,
            sellerAward: 0,
            termsHash: termsHash
        });

        if (asset == address(0)) {
            if (msg.value != amount) revert BadFunding();
        } else {
            if (msg.value != 0) revert BadFunding();
            ExactToken.pullExact(asset, buyer, amount);
        }
        emit Funded(id, buyer, seller, amount, acceptBy, deliverBy, policy, termsHash);
    }

    // ---------------------------------------------------- agreement and exits

    /// @notice Seller accepts before the acceptance deadline.
    function accept(uint256 id) external {
        Deal storage d = _live(id);
        if (d.phase != OFFERED) revert WrongPhase(id, d.phase);
        if (msg.sender != d.seller) revert NotAuthorized();
        if (block.timestamp >= d.clock) revert TooLate();
        d.phase = ACCEPTED;
        emit Accepted(id, msg.sender);
    }

    /// @notice An unaccepted offer returns everything to the buyer: the buyer
    ///         may withdraw it at any time, anyone may lapse it at `acceptBy`.
    function cancel(uint256 id) external {
        Deal storage d = _live(id);
        if (d.phase != OFFERED) revert WrongPhase(id, d.phase);
        if (msg.sender != d.buyer && block.timestamp < d.clock) revert NotAuthorized();
        _resolve(id, d, 0);
    }

    /// @notice Buyer releases everything to the seller (also ends a dispute).
    function release(uint256 id) external {
        Deal storage d = _open(id);
        if (msg.sender != d.buyer) revert NotAuthorized();
        _resolve(id, d, d.amount);
    }

    /// @notice Seller refunds everything to the buyer (also ends a dispute).
    function refund(uint256 id) external {
        Deal storage d = _open(id);
        if (msg.sender != d.seller) revert NotAuthorized();
        _resolve(id, d, 0);
    }

    /// @notice Either party proposes a split; a new proposal replaces the old.
    function propose(uint256 id, uint128 sellerAward) external {
        Deal storage d = _open(id);
        uint8 role = _role(d);
        if (sellerAward > d.amount) revert AwardTooLarge(sellerAward, d.amount);
        uint64 round = _proposals[id].round + 1;
        _proposals[id] = Proposal({sellerAward: sellerAward, round: round, proposer: role});
        emit Proposed(id, msg.sender, sellerAward, round);
    }

    /// @notice The other party accepts the exact current proposal.
    function acceptProposal(uint256 id, uint64 round, uint128 sellerAward) external {
        Deal storage d = _open(id);
        uint8 role = _role(d);
        Proposal memory p = _proposals[id];
        if (p.round == 0 || p.round != round || p.sellerAward != sellerAward) revert StaleProposal();
        if (p.proposer == role) revert NotAuthorized();
        _resolve(id, d, sellerAward);
    }

    /// @notice Either party opens a dispute before the delivery deadline
    ///         (arbiter instances only). The ruling window is fixed.
    function dispute(uint256 id) external {
        if (arbiter == address(0)) revert NoArbiter();
        Deal storage d = _live(id);
        if (d.phase != ACCEPTED) revert WrongPhase(id, d.phase);
        _role(d);
        if (block.timestamp >= d.deliverBy) revert TooLate();
        uint64 rulingBy = uint64(block.timestamp) + rulingDuration;
        d.clock = rulingBy;
        d.phase = DISPUTED;
        emit Disputed(id, msg.sender, rulingBy);
    }

    /// @notice The instance's arbiter splits a disputed deal before `rulingBy`.
    ///         Funds can only go to the two fixed parties.
    function rule(uint256 id, uint128 sellerAward) external {
        if (msg.sender != arbiter || arbiter == address(0)) revert NotAuthorized();
        Deal storage d = _live(id);
        if (d.phase != DISPUTED) revert WrongPhase(id, d.phase);
        if (block.timestamp >= d.clock) revert TooLate();
        if (sellerAward > d.amount) revert AwardTooLarge(sellerAward, d.amount);
        _resolve(id, d, sellerAward);
    }

    /// @notice Anyone applies the agreed silence policy: at `deliverBy` for an
    ///         accepted deal, at `rulingBy` for a disputed one.
    function resolveTimeout(uint256 id) external {
        Deal storage d = _live(id);
        uint8 phase = d.phase;
        if (phase == ACCEPTED) {
            if (block.timestamp < d.deliverBy) revert TooEarly();
        } else if (phase == DISPUTED) {
            if (block.timestamp < d.clock) revert TooEarly();
        } else {
            revert WrongPhase(id, phase);
        }
        _resolve(id, d, d.policy == SELLER_PAYMENT ? d.amount : 0);
    }

    /// @notice Pay one party's award to its fixed address. Anyone may call.
    ///         A failed transfer reverts and keeps the right; the other
    ///         party's payout is independent.
    function pay(uint256 id, bool toSeller) external lock {
        Deal storage d = _live(id);
        if (d.phase != RESOLVED) revert WrongPhase(id, d.phase);
        uint8 flag = toSeller ? PAID_SELLER : PAID_BUYER;
        uint8 paid = d.paid;
        if (paid & flag != 0) revert AlreadyPaid();
        uint256 award = toSeller ? d.sellerAward : d.amount - d.sellerAward;
        address to = toSeller ? d.seller : d.buyer;

        paid |= flag;
        if (paid == PAID_SELLER | PAID_BUYER) {
            delete _deals[id];
            delete _proposals[id];
        } else {
            d.paid = paid;
        }
        // award <= amount < 2^128
        // forge-lint: disable-next-line(unsafe-typecast)
        uint128 owed = _g.totalLiability - uint128(award);
        _g.totalLiability = owed;

        if (asset == address(0)) ExactToken.pushNative(to, award);
        else ExactToken.pushExact(asset, to, award);
        uint256 custody = _custody();
        if (custody < owed) revert BackingShortfall(custody, owed);
        emit Paid(id, to, award);
    }

    // ------------------------------------------------------------------ views

    function nextId() external view returns (uint256) {
        return _g.nextId;
    }

    function totalLiability() external view returns (uint256) {
        return _g.totalLiability;
    }

    function deal(uint256 id) external view returns (Deal memory) {
        return _deals[id];
    }

    function proposal(uint256 id) external view returns (Proposal memory) {
        return _proposals[id];
    }

    // --------------------------------------------------------------- internal

    function _live(uint256 id) private view returns (Deal storage d) {
        d = _deals[id];
        if (d.phase == NONE) revert UnknownDeal(id);
    }

    function _open(uint256 id) private view returns (Deal storage d) {
        d = _live(id);
        if (d.phase != ACCEPTED && d.phase != DISPUTED) revert WrongPhase(id, d.phase);
    }

    function _role(Deal storage d) private view returns (uint8) {
        if (msg.sender == d.buyer) return ROLE_BUYER;
        if (msg.sender == d.seller) return ROLE_SELLER;
        revert NotAuthorized();
    }

    /// @dev Records the split once. Zero awards are consumed immediately.
    function _resolve(uint256 id, Deal storage d, uint128 sellerAward) private {
        uint8 paid;
        if (sellerAward == 0) paid |= PAID_SELLER;
        if (sellerAward == d.amount) paid |= PAID_BUYER;
        d.phase = RESOLVED;
        d.sellerAward = sellerAward;
        d.paid = paid;
        if (_proposals[id].round != 0) delete _proposals[id];
        emit Resolved(id, sellerAward);
    }

    function _custody() private view returns (uint256) {
        return asset == address(0) ? address(this).balance : IERC20Min(asset).balanceOf(address(this));
    }

    function _brakeReasons() internal view override returns (uint8 r) {
        if (asset != address(0) && asset.codehash != assetCodeHash) r |= REASON_CODE_CHANGED;
        if (_custody() < _g.totalLiability) r |= REASON_DEFICIT;
        if (_g.nextId == type(uint64).max) r |= REASON_IDS_EXHAUSTED;
    }

    function _brakeSince() internal view override returns (uint64) {
        return _g.brakeSince;
    }

    function _setBrakeSince(uint64 h) internal override {
        _g.brakeSince = h;
    }

    function _brakeUri() internal pure override returns (string memory) {
        return "native/escrow/SECURITY.md#brake";
    }
}
