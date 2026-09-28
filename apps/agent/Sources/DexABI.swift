import Foundation

/// Unsigned 256-bit integer: just enough for ERC-20 amounts (supplies reach
/// 1e48 base units, past UInt128). Four 64-bit limbs, least significant first.
struct U256: Comparable, CustomStringConvertible {
    let limbs: [UInt64]

    static let zero = U256(limbs: [0, 0, 0, 0])

    init(limbs: [UInt64]) { self.limbs = limbs }
    init(_ v: UInt64) { limbs = [v, 0, 0, 0] }

    /// From up to 64 hex digits (one ABI word), with or without 0x.
    init?(hex: some StringProtocol) {
        var h = String(hex)
        if h.hasPrefix("0x") { h.removeFirst(2) }
        guard h.count <= 64, h.allSatisfy(\.isHexDigit) else { return nil }
        h = String(repeating: "0", count: 64 - h.count) + h
        let chars = Array(h)
        var out: [UInt64] = []
        for i in stride(from: 48, through: 0, by: -16) {
            guard let v = UInt64(String(chars[i..<(i + 16)]), radix: 16) else { return nil }
            out.append(v)
        }
        limbs = out
    }

    /// From a base-10 integer string.
    init?(decimal s: String) {
        guard !s.isEmpty, s.allSatisfy(\.isNumber) else { return nil }
        var v = U256.zero
        for c in s {
            guard let d = c.wholeNumberValue, let m = v.multiplied(by: 10), let a = m.adding(UInt64(d)) else { return nil }
            v = a
        }
        limbs = v.limbs
    }

    /// From a decimal token amount like "1.5" with `decimals` places.
    init?(units s: String, decimals: Int) {
        let parts = s.trimmingCharacters(in: .whitespaces).split(separator: ".", omittingEmptySubsequences: false)
        guard (1...2).contains(parts.count), parts.allSatisfy({ $0.allSatisfy(\.isNumber) }) else { return nil }
        let whole = String(parts[0])
        let frac = parts.count == 2 ? String(parts[1]) : ""
        guard !(whole.isEmpty && frac.isEmpty), frac.count <= decimals else { return nil }
        let digits = (whole.isEmpty ? "0" : whole) + frac + String(repeating: "0", count: decimals - frac.count)
        guard let v = U256(decimal: digits) else { return nil }
        limbs = v.limbs
    }

    var isZero: Bool { limbs.allSatisfy { $0 == 0 } }

    /// One 64-digit ABI word.
    var hexWord: String { limbs.reversed().map { String(format: "%016llx", $0) }.joined() }

    func multiplied(by m: UInt64) -> U256? {
        var carry: UInt64 = 0
        var out: [UInt64] = []
        for l in limbs {
            let (hi, lo) = l.multipliedFullWidth(by: m)
            let (sum, o) = lo.addingReportingOverflow(carry)
            out.append(sum)
            carry = hi + (o ? 1 : 0)
        }
        return carry == 0 ? U256(limbs: out) : nil
    }

    func adding(_ a: UInt64) -> U256? {
        var carry = a
        var out: [UInt64] = []
        for l in limbs {
            let (s, o) = l.addingReportingOverflow(carry)
            out.append(s)
            carry = o ? 1 : 0
        }
        return carry == 0 ? U256(limbs: out) : nil
    }

    func divided(by d: UInt64) -> (quotient: U256, remainder: UInt64) {
        var rem: UInt64 = 0
        var out = [UInt64](repeating: 0, count: 4)
        for i in stride(from: 3, through: 0, by: -1) {
            let (q, r) = d.dividingFullWidth((high: rem, low: limbs[i]))
            out[i] = q
            rem = r
        }
        return (U256(limbs: out), rem)
    }

    /// Base-10 string.
    var description: String {
        if isZero { return "0" }
        var digits: [Character] = []
        var v = self
        while !v.isZero {
            let (q, r) = v.divided(by: 10)
            digits.append(Character(String(r)))
            v = q
        }
        return String(digits.reversed())
    }

    /// Lossy, for prices and percentages only.
    var double: Double { limbs.enumerated().reduce(0.0) { $0 + Double($1.element) * pow(2.0, Double(64 * $1.offset)) } }

    /// `self` as a token amount with `decimals` places, trailing zeros trimmed (exact).
    func units(_ decimals: Int) -> String {
        let s = description
        guard decimals > 0 else { return s }
        let padded = String(repeating: "0", count: max(0, decimals + 1 - s.count)) + s
        let whole = padded.dropLast(decimals)
        var frac = String(padded.suffix(decimals))
        while frac.hasSuffix("0") { frac.removeLast() }
        return frac.isEmpty ? String(whole) : "\(whole).\(frac)"
    }

    static func < (a: U256, b: U256) -> Bool {
        for i in stride(from: 3, through: 0, by: -1) where a.limbs[i] != b.limbs[i] { return a.limbs[i] < b.limbs[i] }
        return false
    }
}

/// Solidity ABI encoding and decoding for the few read calls the DEX tools make.
enum ABI {
    static func word(address a: String) -> String {
        let h = a.lowercased().hasPrefix("0x") ? String(a.dropFirst(2)) : a
        return String(repeating: "0", count: 64 - h.count) + h.lowercased()
    }

    static func isAddress(_ s: String) -> Bool {
        s.hasPrefix("0x") && s.count == 42 && s.dropFirst(2).allSatisfy(\.isHexDigit)
    }

    /// Call data: selector (8 hex digits) followed by static words.
    static func call(_ selector: String, _ words: [String] = []) -> String { "0x" + selector + words.joined() }

    /// `getAmountsOut(uint256,address[])`.
    static func amountsOut(_ selector: String, amount: U256, path: [String]) -> String {
        call(selector, [amount.hexWord, U256(0x40).hexWord, U256(UInt64(path.count)).hexWord] + path.map { word(address: $0) })
    }

    /// Return data split into 32-byte words (hex).
    static func words(_ data: String) throws -> [String] {
        let h = data.hasPrefix("0x") ? String(data.dropFirst(2)) : data
        guard h.count % 64 == 0 else { throw AgentError.io("unexpected contract answer (\(h.count / 2) bytes)") }
        let chars = Array(h)
        return stride(from: 0, to: chars.count, by: 64).map { String(chars[$0..<($0 + 64)]) }
    }

    static func uint(_ data: String, at i: Int = 0) throws -> U256 {
        let w = try words(data)
        guard i < w.count, let v = U256(hex: w[i]) else { throw AgentError.io("unexpected contract answer (no word \(i))") }
        return v
    }

    static func address(_ data: String, at i: Int = 0) throws -> String {
        let w = try words(data)
        guard i < w.count else { throw AgentError.io("unexpected contract answer (no address)") }
        return "0x" + w[i].suffix(40)
    }

    /// A dynamic `string` return value.
    static func string(_ data: String) throws -> String {
        let w = try words(data)
        guard let off = try? uint(data, at: 0), off.limbs[0] % 32 == 0, off.limbs[1...].allSatisfy({ $0 == 0 }) else { throw AgentError.io("bad string") }
        let at = Int(off.limbs[0] / 32)
        guard at < w.count, let len = U256(hex: w[at]), len.limbs[1...].allSatisfy({ $0 == 0 }), len.limbs[0] <= 4096 else { throw AgentError.io("bad string") }
        let n = Int(len.limbs[0])
        let hex = w[(at + 1)...].joined().prefix(n * 2)
        guard hex.count == n * 2 else { throw AgentError.io("bad string") }
        var bytes: [UInt8] = []
        var i = hex.startIndex
        while i < hex.endIndex {
            let j = hex.index(i, offsetBy: 2)
            bytes.append(UInt8(hex[i..<j], radix: 16) ?? 0)
            i = j
        }
        return String(decoding: bytes, as: UTF8.self)
    }

    /// A dynamic `uint256[]` return value.
    static func uintArray(_ data: String) throws -> [U256] {
        let w = try words(data)
        let off = try uint(data, at: 0)
        let at = Int(off.limbs[0] / 32)
        guard at < w.count, let len = U256(hex: w[at]), Int(len.limbs[0]) <= w.count - at - 1 else { throw AgentError.io("bad uint256[]") }
        return try (0..<Int(len.limbs[0])).map { try uint(data, at: at + 1 + $0) }
    }
}
