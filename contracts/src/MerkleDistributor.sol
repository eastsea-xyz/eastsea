// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.19;

/// Token distribution tools (docs/design/17-token-tools.md): a Merkle claim
/// campaign per airdrop, and a Disperse-style ERC-20 batch send. Everything
/// here is immutable and fee-free: nobody but the tree's listed recipients can
/// move a campaign's tokens, and no contract in this file takes a cut.

/// The ERC-20 pieces the tools need.
interface IERC20 {
    function transfer(address to, uint256 amount) external returns (bool);
    function transferFrom(address from, address to, uint256 amount) external returns (bool);
    function balanceOf(address account) external view returns (uint256);
}

error TransferFailed();

/// `transfer`/`transferFrom` through a raw call, so tokens that return nothing
/// (USDT-style) work too; a failing call bubbles the token's own reason, and a
/// `false` answer (a clean refusal) becomes TransferFailed.
function _erc20Move(address token, bytes memory data) {
    (bool ok, bytes memory out) = token.call(data);
    if (!ok) {
        assembly {
            revert(add(out, 32), mload(out))
        }
    }
    if (out.length != 0 && !abi.decode(out, (bool))) revert TransferFailed();
}

/// One airdrop campaign, fully funded with an ERC-20 at birth. The tree's leaf
/// for entry `index` is keccak256(index, account, amount), and every inner node
/// hashes its pair sorted (smaller hash first), exactly as
/// scripts/merkle-build.mjs builds it. After the constructor nothing changes:
/// `claim` pays the account in the tree -- never the caller -- so anyone may
/// submit a claim on someone's behalf (gas sponsorship), and each index pays
/// once. A campaign with an end time stops paying at it and the creator may
/// then sweep what is left; that sweep is the only right the creator keeps.
/// Without an end time the campaign runs forever and can never be swept.
contract MerkleDistributor {
    error AlreadyClaimed(uint256 index);
    error BadProof();
    error Ended(uint64 at);
    error NoEnd();
    error NotYet(uint64 at);
    error OnlyCreator();

    event Claimed(uint256 index, address account, uint256 amount);
    event Swept(address to, uint256 amount);

    IERC20 public immutable token;
    /// The account that funded the campaign; the only one that may sweep.
    address public immutable creator;
    bytes32 public immutable merkleRoot;
    /// Unix time claims stop at; 0 = the campaign never ends.
    uint64 public immutable ends;

    mapping(uint256 => bool) public claimed;

    constructor(IERC20 token_, address creator_, bytes32 merkleRoot_, uint64 ends_) {
        token = token_;
        creator = creator_;
        merkleRoot = merkleRoot_;
        ends = ends_;
    }

    function isClaimed(uint256 index) external view returns (bool) {
        return claimed[index];
    }

    /// Pay entry `index` to its `account`. The proof verifies against the root
    /// regardless of sibling order (pairs are hashed smaller-hash-first), so a
    /// proof is just the sibling list from leaf to root.
    function claim(uint256 index, address account, uint256 amount, bytes32[] calldata proof) external {
        if (ends != 0 && block.timestamp >= ends) revert Ended(ends);
        if (claimed[index]) revert AlreadyClaimed(index);
        bytes32 node = keccak256(abi.encodePacked(index, account, amount));
        for (uint256 i = 0; i < proof.length; i++) {
            bytes32 sibling = proof[i];
            node = node <= sibling ? keccak256(abi.encodePacked(node, sibling)) : keccak256(abi.encodePacked(sibling, node));
        }
        if (node != merkleRoot) revert BadProof();
        claimed[index] = true;
        _erc20Move(address(token), abi.encodeWithSelector(IERC20.transfer.selector, account, amount));
        emit Claimed(index, account, amount);
    }

    /// After the end time, the creator takes back what nobody claimed. This is
    /// the creator's only power; before the end (or in a campaign without one)
    /// it reverts.
    function sweep() external {
        if (msg.sender != creator) revert OnlyCreator();
        if (ends == 0) revert NoEnd();
        if (block.timestamp < ends) revert NotYet(ends);
        uint256 left = token.balanceOf(address(this));
        _erc20Move(address(token), abi.encodeWithSelector(IERC20.transfer.selector, creator, left));
        emit Swept(creator, left);
    }
}

/// Deploys one MerkleDistributor per campaign and funds it in the same
/// transaction: `create` pulls `total` tokens from the caller (approve this
/// factory first), so a campaign always starts fully funded. Campaigns get a
/// deterministic address (CREATE2, one salt per creator per campaign number).
contract MerkleDistributorFactory {
    error BadEnd(uint64 ends);
    error BadTotal();

    event Campaign(address distributor, address creator, address token, bytes32 merkleRoot, uint64 ends, uint256 total);

    MerkleDistributor[] public campaigns;

    function count() external view returns (uint256) {
        return campaigns.length;
    }

    /// `ends` must be in the future (or 0 = no end); `total` is the whole
    /// campaign, moved from the caller to the new distributor here.
    function create(IERC20 token, bytes32 merkleRoot, uint64 ends, uint256 total) external returns (MerkleDistributor d) {
        if (ends != 0 && ends <= block.timestamp) revert BadEnd(ends);
        if (total == 0) revert BadTotal();
        bytes32 salt = keccak256(abi.encode(msg.sender, campaigns.length));
        d = new MerkleDistributor{salt: salt}(token, msg.sender, merkleRoot, ends);
        campaigns.push(d);
        _erc20Move(address(token), abi.encodeWithSelector(IERC20.transferFrom.selector, msg.sender, address(d), total));
        emit Campaign(address(d), msg.sender, address(token), merkleRoot, ends, total);
    }
}

/// ERC-20 batch sends, Disperse-style: approve this contract once, then `send`
/// moves tokens straight from the caller to each recipient -- it never holds
/// them. All or nothing: one failing transfer (e.g. the allowance ran out
/// mid-batch) reverts the whole batch. The native-AETH counterpart is the
/// account's own `execute` (EastSeaAccount), which batches plain transfers.
contract TokenBatch {
    error LengthMismatch();

    event Sent(address indexed token, address indexed from, uint256 recipients, uint256 total);

    function send(IERC20 token, address[] calldata to, uint256[] calldata amounts) external {
        if (to.length != amounts.length) revert LengthMismatch();
        uint256 total = 0;
        for (uint256 i = 0; i < to.length; i++) {
            _erc20Move(address(token), abi.encodeWithSelector(IERC20.transferFrom.selector, msg.sender, to[i], amounts[i]));
            total += amounts[i];
        }
        emit Sent(address(token), msg.sender, to.length, total);
    }
}
