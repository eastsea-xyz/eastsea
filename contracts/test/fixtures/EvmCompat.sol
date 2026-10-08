// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.19;

interface IEvmCompat721Receiver {
    function onERC721Received(address operator, address from, uint256 id, bytes calldata data) external returns (bytes4);
}

/// Test-only ERC-20 fixture. The v/r/s permit keeps ERC-2612's secp256k1
/// semantics; the separate bytes overload represents an ERC-1271-aware caller.
/// ERC-2612 itself does not require a contract-signature fallback.
contract EvmCompatToken {
    string public constant name = "EvmCompatToken";
    string public constant symbol = "COMPAT";
    uint8 public constant decimals = 18;

    bytes32 private constant DOMAIN_TYPEHASH =
        keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)");
    bytes32 private constant PERMIT_TYPEHASH =
        keccak256("Permit(address owner,address spender,uint256 value,uint256 nonce,uint256 deadline)");

    uint256 public totalSupply;
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;
    mapping(address => uint256) public nonces;

    event Transfer(address indexed from, address indexed to, uint256 amount);
    event Approval(address indexed owner, address indexed spender, uint256 amount);

    function mint(address to, uint256 amount) external {
        require(to != address(0), "zero recipient");
        totalSupply += amount;
        balanceOf[to] += amount;
        emit Transfer(address(0), to, amount);
    }

    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
        emit Approval(msg.sender, spender, amount);
        return true;
    }

    function transfer(address to, uint256 amount) external returns (bool) {
        _transfer(msg.sender, to, amount);
        return true;
    }

    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        uint256 allowed = allowance[from][msg.sender];
        if (allowed != type(uint256).max) allowance[from][msg.sender] = allowed - amount;
        _transfer(from, to, amount);
        return true;
    }

    function DOMAIN_SEPARATOR() public view returns (bytes32) {
        return
            keccak256(abi.encode(DOMAIN_TYPEHASH, keccak256(bytes(name)), keccak256("1"), block.chainid, address(this)));
    }

    function permit(address owner, address spender, uint256 value, uint256 deadline, uint8 v, bytes32 r, bytes32 s)
        external
    {
        bytes32 digest = _permitDigest(owner, spender, value, deadline);
        require(ecrecover(digest, v, r, s) == owner, "signature");
        _approvePermit(owner, spender, value);
    }

    function permit(address owner, address spender, uint256 value, uint256 deadline, bytes calldata signature)
        external
    {
        bytes32 digest = _permitDigest(owner, spender, value, deadline);
        bool valid;
        if (owner.code.length == 0) {
            if (signature.length == 65) {
                bytes32 r = bytes32(signature[0:32]);
                bytes32 s = bytes32(signature[32:64]);
                uint8 v = uint8(signature[64]);
                valid = ecrecover(digest, v, r, s) == owner;
            }
        } else {
            (bool ok, bytes memory out) =
                owner.staticcall(abi.encodeWithSelector(bytes4(0x1626ba7e), digest, signature));
            valid = ok && out.length >= 32 && bytes4(out) == 0x1626ba7e;
        }
        require(valid, "signature");
        _approvePermit(owner, spender, value);
    }

    function _permitDigest(address owner, address spender, uint256 value, uint256 deadline)
        private
        view
        returns (bytes32)
    {
        require(owner != address(0), "zero owner");
        require(block.timestamp <= deadline, "expired");
        bytes32 contents = keccak256(abi.encode(PERMIT_TYPEHASH, owner, spender, value, nonces[owner], deadline));
        return keccak256(abi.encodePacked("\x19\x01", DOMAIN_SEPARATOR(), contents));
    }

    function _approvePermit(address owner, address spender, uint256 value) private {
        nonces[owner]++;
        allowance[owner][spender] = value;
        emit Approval(owner, spender, value);
    }

    function _transfer(address from, address to, uint256 amount) private {
        require(to != address(0), "zero recipient");
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
        emit Transfer(from, to, amount);
    }
}

/// Test-only safe-mint/transfer fixture. An account's 23-byte delegation
/// designator is code, so the same receiver callback as any contract is needed.
contract EvmCompatNft {
    mapping(uint256 => address) public ownerOf;

    event Transfer(address indexed from, address indexed to, uint256 indexed id);

    function safeMint(address to, uint256 id) external {
        require(to != address(0) && ownerOf[id] == address(0), "mint");
        ownerOf[id] = to;
        emit Transfer(address(0), to, id);
        _checkReceiver(address(0), to, id, "");
    }

    function safeTransferFrom(address from, address to, uint256 id) external {
        _safeTransfer(from, to, id, "");
    }

    function safeTransferFrom(address from, address to, uint256 id, bytes calldata data) external {
        _safeTransfer(from, to, id, data);
    }

    function _safeTransfer(address from, address to, uint256 id, bytes memory data) private {
        require(msg.sender == from && ownerOf[id] == from && to != address(0), "transfer");
        ownerOf[id] = to;
        emit Transfer(from, to, id);
        _checkReceiver(from, to, id, data);
    }

    function _checkReceiver(address from, address to, uint256 id, bytes memory data) private {
        if (to.code.length != 0) {
            require(IEvmCompat721Receiver(to).onERC721Received(msg.sender, from, id, data) == 0x150b7a02, "receiver");
        }
    }
}
