// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.19;

/// Immutable, fee-free HTLC for native coin and ordinary ERC-20 tokens.
/// A zero token address denotes native coin. The caller chooses the deadline;
/// the cross-chain wallet must check the other chain's finality and refund gap.
contract AtomicSwap {
    enum Status {
        Open,
        Claimed,
        Refunded
    }

    struct Swap {
        address sender;
        address recipient;
        bytes32 hashlock;
        uint64 timelock;
        address token;
        uint256 amount;
        Status status;
    }

    error BadLock();
    error UnknownSwap();
    error AlreadySettled();
    error Expired();
    error NotExpired();
    error WrongPreimage();
    error TransferFailed();
    error UnsupportedToken();

    event Locked(
        uint256 indexed id,
        bytes32 indexed hashlock,
        address indexed recipient,
        address sender,
        uint64 timelock,
        address token,
        uint256 amount
    );
    event Claimed(uint256 indexed id, bytes32 indexed hashlock, address indexed recipient, bytes preimage);
    event Refunded(uint256 indexed id, bytes32 indexed hashlock, address indexed recipient);

    Swap[] public swaps;

    function lock(address recipient, bytes32 hashlock, uint64 timelock, address token, uint256 amount)
        external
        payable
        returns (uint256 id)
    {
        if (
            recipient == address(0) || recipient == address(this) || hashlock == bytes32(0) || amount == 0
                || timelock <= block.timestamp
        ) {
            revert BadLock();
        }
        if (token == address(0)) {
            if (msg.value != amount) revert BadLock();
        } else {
            if (msg.value != 0) revert BadLock();
            uint256 beforeBalance = _balanceOf(token, address(this));
            _transferFrom(token, msg.sender, amount);
            uint256 afterBalance = _balanceOf(token, address(this));
            if (afterBalance < beforeBalance || afterBalance - beforeBalance != amount) {
                revert UnsupportedToken();
            }
        }
        id = swaps.length;
        swaps.push(Swap(msg.sender, recipient, hashlock, timelock, token, amount, Status.Open));
        emit Locked(id, hashlock, recipient, msg.sender, timelock, token, amount);
    }

    /// Anyone can reveal the preimage, but only the stored recipient is paid.
    function claim(uint256 id, bytes calldata preimage) external {
        Swap storage swap = _open(id);
        if (block.timestamp >= swap.timelock) revert Expired();
        if (sha256(preimage) != swap.hashlock) revert WrongPreimage();
        swap.status = Status.Claimed;
        _pay(swap.token, swap.recipient, swap.amount);
        emit Claimed(id, swap.hashlock, swap.recipient, preimage);
    }

    /// Anyone can trigger expiry; the stored sender always receives the funds.
    function refund(uint256 id) external {
        Swap storage swap = _open(id);
        if (block.timestamp < swap.timelock) revert NotExpired();
        swap.status = Status.Refunded;
        _pay(swap.token, swap.sender, swap.amount);
        emit Refunded(id, swap.hashlock, swap.recipient);
    }

    function swapCount() external view returns (uint256) {
        return swaps.length;
    }

    function _open(uint256 id) private view returns (Swap storage swap) {
        if (id >= swaps.length) revert UnknownSwap();
        swap = swaps[id];
        if (swap.status != Status.Open) revert AlreadySettled();
    }

    function _pay(address token, address recipient, uint256 amount) private {
        if (token == address(0)) {
            (bool ok,) = payable(recipient).call{value: amount}("");
            if (!ok) revert TransferFailed();
        } else {
            uint256 beforeEscrow = _balanceOf(token, address(this));
            uint256 beforeRecipient = _balanceOf(token, recipient);
            _transfer(token, recipient, amount);
            uint256 afterEscrow = _balanceOf(token, address(this));
            uint256 afterRecipient = _balanceOf(token, recipient);
            if (
                afterEscrow > beforeEscrow || beforeEscrow - afterEscrow != amount || afterRecipient < beforeRecipient
                    || afterRecipient - beforeRecipient != amount
            ) {
                revert UnsupportedToken();
            }
        }
    }

    function _balanceOf(address token, address owner) private view returns (uint256 balance) {
        if (token.code.length == 0) revert UnsupportedToken();
        (bool ok, bytes memory out) = token.staticcall(abi.encodeWithSelector(bytes4(0x70a08231), owner));
        if (!ok || out.length != 32) revert UnsupportedToken();
        balance = abi.decode(out, (uint256));
    }

    function _transferFrom(address token, address sender, uint256 amount) private {
        (bool ok, bytes memory out) =
            token.call(abi.encodeWithSelector(bytes4(0x23b872dd), sender, address(this), amount));
        if (!ok || (out.length != 0 && (out.length != 32 || !abi.decode(out, (bool))))) revert TransferFailed();
    }

    function _transfer(address token, address recipient, uint256 amount) private {
        (bool ok, bytes memory out) = token.call(abi.encodeWithSelector(bytes4(0xa9059cbb), recipient, amount));
        if (!ok || (out.length != 0 && (out.length != 32 || !abi.decode(out, (bool))))) revert TransferFailed();
    }
}
