// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.19;

// Local fallback for chains without the RIP-7212 precompile. Adapted from the
// repository's LibP256 test implementation; production should use address(0x100).
library P256Fallback {
    uint256 constant p = 0xffffffff00000001000000000000000000000000ffffffffffffffffffffffff;
    uint256 constant n = 0xffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551;
    uint256 constant cb = 0x5ac635d8aa3a93e7b3ebbd55769886bc651d06b0cc53b0f63bce3c3e27d2604b;
    uint256 constant gx = 0x6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296;
    uint256 constant gy = 0x4fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5;

    /// x - y mod p. addmod is exact for any 256-bit operands, so this cannot underflow.
    function _subP(uint256 x, uint256 y) private pure returns (uint256) {
        return addmod(x, p - y, p);
    }

    /// x^-1 mod m (m prime) via the modexp precompile: sizes header, then
    /// base, exponent (m - 2, by Fermat), modulus.
    function _inv(uint256 x, uint256 m) private view returns (uint256 r) {
        (bool ok, bytes memory out) =
            address(5).staticcall(abi.encode(uint256(32), uint256(32), uint256(32), x, m - 2, m));
        require(ok && out.length == 32, "modexp");
        r = abi.decode(out, (uint256));
    }

    /// Jacobian point; (0, 0, 0) is the point at infinity. Points travel by
    /// reference so the arithmetic stays inside the old codegen's stack budget.
    struct Jac {
        uint256 x;
        uint256 y;
        uint256 z;
    }

    /// Jacobian doubling for a = -3 (EFD dbl-2001-b).
    function _dbl(Jac memory a) private pure returns (Jac memory) {
        if (a.z == 0 || a.y == 0) return Jac(0, 0, 0);
        uint256 delta = mulmod(a.z, a.z, p);
        uint256 gamma = mulmod(a.y, a.y, p);
        uint256 beta = mulmod(a.x, gamma, p);
        uint256 alpha = mulmod(3, mulmod(_subP(a.x, delta), addmod(a.x, delta, p), p), p);
        uint256 x3 = _subP(mulmod(alpha, alpha, p), mulmod(8, beta, p));
        uint256 y3 = _subP(mulmod(alpha, _subP(mulmod(4, beta, p), x3), p), mulmod(8, mulmod(gamma, gamma, p), p));
        return Jac(x3, y3, mulmod(2, mulmod(a.y, a.z, p), p));
    }

    /// Jacobian addition (EFD add-2007-bl).
    function _add(Jac memory a, Jac memory b) private pure returns (Jac memory) {
        if (a.z == 0) return b;
        if (b.z == 0) return a;
        Jac memory out = Jac(0, 0, 0);
        uint256 u1;
        uint256 s1;
        {
            uint256 z2z2 = mulmod(b.z, b.z, p);
            u1 = mulmod(a.x, z2z2, p);
            s1 = mulmod(mulmod(a.y, b.z, p), z2z2, p);
        }
        uint256 u2;
        uint256 s2;
        {
            uint256 z1z1 = mulmod(a.z, a.z, p);
            u2 = mulmod(b.x, z1z1, p);
            s2 = mulmod(mulmod(b.y, a.z, p), z1z1, p);
        }
        if (u1 == u2) {
            if (s1 != s2) return out; // P + (-P) is the point at infinity
            return _dbl(a);
        }
        uint256 h = _subP(u2, u1);
        uint256 i = mulmod(4, mulmod(h, h, p), p);
        uint256 j = mulmod(h, i, p);
        out.z = mulmod(mulmod(2, mulmod(a.z, b.z, p), p), h, p);
        uint256 r = mulmod(2, _subP(s2, s1), p);
        uint256 v = mulmod(u1, i, p);
        out.x = _subP(_subP(mulmod(r, r, p), j), mulmod(2, v, p));
        out.y = _subP(mulmod(r, _subP(v, out.x), p), mulmod(2, mulmod(s1, j, p), p));
        return out;
    }

    function _mulJ(uint256 k, uint256 x, uint256 y) private pure returns (Jac memory) {
        Jac memory acc = Jac(0, 0, 0);
        Jac memory g = Jac(x, y, 1);
        for (uint256 i = 0; i < 256; i++) {
            if (acc.z != 0) acc = _dbl(acc); // infinity stays infinity
            if (((k >> (255 - i)) & 1) == 1) acc = _add(acc, g);
        }
        return acc;
    }

    function onCurve(uint256 x, uint256 y) internal pure returns (bool) {
        if (x >= p || y >= p) return false;
        uint256 lhs = mulmod(y, y, p);
        uint256 rhs = addmod(_subP(mulmod(mulmod(x, x, p), x, p), mulmod(3, x, p)), cb, p);
        return lhs == rhs;
    }

    /// What the 0x100 precompile checks.
    function verify(bytes32 digest, bytes32 rb, bytes32 sb, bytes32 xb, bytes32 yb) internal view returns (bool) {
        uint256 r = uint256(rb);
        uint256 s = uint256(sb);
        if (r == 0 || r >= n || s == 0 || s >= n) return false;
        if (!onCurve(uint256(xb), uint256(yb))) return false;
        return _recoverX(digest, r, s, xb, yb) == r;
    }

    /// x coordinate of u1*G + u2*Q, reduced mod n (0 for the point at infinity).
    function _recoverX(bytes32 digest, uint256 r, uint256 s, bytes32 xb, bytes32 yb) private view returns (uint256) {
        uint256 w = _inv(s, n);
        Jac memory p1 = _mulJ(mulmod(uint256(digest) % n, w, n), gx, gy);
        Jac memory p2 = _mulJ(mulmod(r, w, n), uint256(xb), uint256(yb));
        Jac memory q = _add(p1, p2);
        if (q.z == 0) return 0;
        uint256 zi = _inv(q.z, p);
        return mulmod(q.x, mulmod(zi, zi, p), p) % n;
    }
}
