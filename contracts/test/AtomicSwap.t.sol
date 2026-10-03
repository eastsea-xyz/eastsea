// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.19;

import {AtomicSwap} from "../src/AtomicSwap.sol";
import {AtomicSwapEVM} from "../src/AtomicSwapEVM.sol";

interface SwapVm {
    function deal(address, uint256) external;
    function prank(address) external;
    function warp(uint256) external;
    function expectRevert(bytes calldata) external;
    function expectEmit(bool, bool, bool, bool) external;
}

contract SwapToken {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;
    bool public feeOnTransfer;

    function mint(address to, uint256 amount) external {
        balanceOf[to] += amount;
    }

    function setFeeOnTransfer(bool enabled) external {
        feeOnTransfer = enabled;
    }

    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
        return true;
    }

    function transfer(address to, uint256 amount) external returns (bool) {
        _move(msg.sender, to, amount);
        return true;
    }

    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        allowance[from][msg.sender] -= amount;
        _move(from, to, amount);
        return true;
    }

    function _move(address from, address to, uint256 amount) private {
        balanceOf[from] -= amount;
        balanceOf[to] += amount - (feeOnTransfer ? amount / 100 : 0);
    }
}

contract AtomicSwapTest {
    SwapVm constant vm = SwapVm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);
    address constant SENDER = address(0x1111);
    address constant RECIPIENT = address(0x2222);
    address constant STRANGER = address(0x3333);
    bytes constant SECRET = hex"00112233445566778899aabbccddeeff";
    uint256 constant AMOUNT = 1 ether;

    AtomicSwap swap;
    SwapToken token;
    uint64 deadline;
    bytes32 hashlock;

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

    function setUp() public {
        swap = new AtomicSwap();
        token = new SwapToken();
        deadline = uint64(block.timestamp + 1 days);
        hashlock = sha256(SECRET);
        vm.deal(SENDER, 10 ether);
        token.mint(SENDER, 10 ether);
        vm.prank(SENDER);
        token.approve(address(swap), type(uint256).max);
    }

    function nativeLock() internal returns (uint256) {
        vm.prank(SENDER);
        return swap.lock{value: AMOUNT}(RECIPIENT, hashlock, deadline, address(0), AMOUNT);
    }

    function tokenLock() internal returns (uint256) {
        vm.prank(SENDER);
        return swap.lock(RECIPIENT, hashlock, deadline, address(token), AMOUNT);
    }

    function test_ThirdPartyClaimPaysOnlyRecipient() public {
        vm.expectEmit(true, true, true, true);
        emit Locked(0, hashlock, RECIPIENT, SENDER, deadline, address(0), AMOUNT);
        uint256 id = nativeLock();
        vm.expectEmit(true, true, true, true);
        emit Claimed(id, hashlock, RECIPIENT, SECRET);
        vm.prank(STRANGER);
        swap.claim(id, SECRET);
        require(RECIPIENT.balance == AMOUNT && STRANGER.balance == 0, "wrong recipient");
        (,,,,,, AtomicSwap.Status status) = swap.swaps(id);
        require(status == AtomicSwap.Status.Claimed, "not claimed");
    }

    function test_WrongPreimageAndDoubleClaim() public {
        uint256 id = nativeLock();
        vm.expectRevert(abi.encodeWithSelector(AtomicSwap.WrongPreimage.selector));
        swap.claim(id, hex"00");
        swap.claim(id, SECRET);
        vm.expectRevert(abi.encodeWithSelector(AtomicSwap.AlreadySettled.selector));
        swap.claim(id, SECRET);
    }

    function test_RefundAtDeadlineAndClaimAfterRefund() public {
        uint256 id = nativeLock();
        vm.expectRevert(abi.encodeWithSelector(AtomicSwap.NotExpired.selector));
        swap.refund(id);
        vm.warp(deadline);
        vm.expectEmit(true, true, true, true);
        emit Refunded(id, hashlock, RECIPIENT);
        vm.prank(STRANGER);
        swap.refund(id);
        require(SENDER.balance == 10 ether && RECIPIENT.balance == 0, "wrong refund");
        vm.expectRevert(abi.encodeWithSelector(AtomicSwap.AlreadySettled.selector));
        swap.claim(id, SECRET);
    }

    function test_ClaimStopsAtDeadline() public {
        uint256 id = nativeLock();
        vm.warp(deadline);
        vm.expectRevert(abi.encodeWithSelector(AtomicSwap.Expired.selector));
        swap.claim(id, SECRET);
    }

    function test_ERC20ClaimAndRefund() public {
        uint256 claimed = tokenLock();
        vm.prank(STRANGER);
        swap.claim(claimed, SECRET);
        require(token.balanceOf(RECIPIENT) == AMOUNT, "token recipient");

        uint256 refunded = tokenLock();
        vm.warp(deadline);
        swap.refund(refunded);
        require(token.balanceOf(SENDER) == 9 ether, "token refund");
        require(token.balanceOf(address(swap)) == 0, "escrow balance");
    }

    function test_FeeOnTransferDepositRejected() public {
        token.setFeeOnTransfer(true);
        vm.expectRevert(abi.encodeWithSelector(AtomicSwap.UnsupportedToken.selector));
        tokenLock();
        require(swap.swapCount() == 0 && token.balanceOf(SENDER) == 10 ether, "partial deposit");
    }

    function test_FeeOnTransferPayoutRejected() public {
        uint256 id = tokenLock();
        token.setFeeOnTransfer(true);
        vm.expectRevert(abi.encodeWithSelector(AtomicSwap.UnsupportedToken.selector));
        swap.claim(id, SECRET);
        require(token.balanceOf(address(swap)) == AMOUNT && token.balanceOf(RECIPIENT) == 0, "partial payout");
        (,,,,,, AtomicSwap.Status status) = swap.swaps(id);
        require(status == AtomicSwap.Status.Open, "settled on failed payout");
    }

    function test_InvalidLockAndUnknownId() public {
        vm.prank(SENDER);
        vm.expectRevert(abi.encodeWithSelector(AtomicSwap.BadLock.selector));
        swap.lock{value: AMOUNT}(address(0), hashlock, deadline, address(0), AMOUNT);
        vm.prank(SENDER);
        vm.expectRevert(abi.encodeWithSelector(AtomicSwap.BadLock.selector));
        swap.lock{value: AMOUNT - 1}(RECIPIENT, hashlock, deadline, address(0), AMOUNT);
        vm.expectRevert(abi.encodeWithSelector(AtomicSwap.UnknownSwap.selector));
        swap.refund(0);
    }

    function test_EVMDeploymentUsesSameBehavior() public {
        AtomicSwapEVM evm = new AtomicSwapEVM();
        vm.prank(SENDER);
        uint256 id = evm.lock{value: AMOUNT}(RECIPIENT, hashlock, deadline, address(0), AMOUNT);
        vm.prank(STRANGER);
        evm.claim(id, SECRET);
        require(RECIPIENT.balance == AMOUNT, "EVM recipient");
    }
}
