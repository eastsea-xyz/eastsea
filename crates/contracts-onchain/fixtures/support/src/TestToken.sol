// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @dev Harness-only token. Callback failures are recorded, not propagated, so
/// the outer transfer can demonstrate that guarded contracts reject reentry.
contract TestToken {
    string public constant name = "Harness Token";
    string public constant symbol = "HT";
    uint8 public constant decimals = 18;
    uint256 public totalSupply;
    uint16 public feeBps;
    bool public failTransfers;
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;
    address public callbackTarget;
    bytes public callbackData;
    bool public callbackAttempted;
    bool public innerSuccess;
    bytes public innerOutput;
    bool private inCallback;
    event Transfer(address indexed from, address indexed to, uint256 amount);
    event Approval(address indexed owner, address indexed spender, uint256 amount);

    constructor(uint16 feeBps_) { setFeeBps(feeBps_); }
    function setFeeBps(uint16 bps) public { require(bps <= 10_000, "fee"); feeBps = bps; }
    function setFailTransfers(bool fail) external { failTransfers = fail; }
    function setCallback(address target, bytes calldata data) external {
        callbackTarget = target; callbackData = data;
        callbackAttempted = false; innerSuccess = false; delete innerOutput;
    }
    function configureCallback(address target, bytes calldata data, bool enabled) external {
        callbackTarget = enabled ? target : address(0); callbackData = data;
        callbackAttempted = false; innerSuccess = false; delete innerOutput;
    }
    function mint(address to, uint256 amount) external {
        require(to != address(0), "zero");
        totalSupply += amount; balanceOf[to] += amount;
        emit Transfer(address(0), to, amount);
    }
    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
        emit Approval(msg.sender, spender, amount); return true;
    }
    function transfer(address to, uint256 amount) external returns (bool) {
        if (failTransfers) return false;
        _transfer(msg.sender, to, amount); return true;
    }
    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        if (failTransfers) return false;
        uint256 approved = allowance[from][msg.sender];
        if (approved != type(uint256).max) {
            require(approved >= amount, "allowance");
            allowance[from][msg.sender] = approved - amount;
        }
        _transfer(from, to, amount); return true;
    }
    function _transfer(address from, address to, uint256 amount) private {
        require(to != address(0), "zero");
        require(balanceOf[from] >= amount, "balance");
        uint256 fee = amount * feeBps / 10_000;
        balanceOf[from] -= amount; balanceOf[to] += amount - fee;
        totalSupply -= fee;
        emit Transfer(from, to, amount - fee);
        if (fee != 0) emit Transfer(from, address(0), fee);
        if (callbackTarget != address(0) && !inCallback) {
            inCallback = true; callbackAttempted = true;
            (innerSuccess, innerOutput) = callbackTarget.call(callbackData);
            inCallback = false;
        }
    }
}
