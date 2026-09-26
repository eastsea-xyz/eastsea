import Foundation

/// Exact AETH amounts (18 decimals) as a 128-bit wei count.
struct Wei: Comparable, CustomStringConvertible {
    let value: UInt128

    static let zero = Wei(value: 0)
    private static let unit: UInt128 = 1_000_000_000_000_000_000

    init(value: UInt128) { self.value = value }

    /// From a base-10 wei string.
    init?(decimal s: String) {
        guard let v = UInt128(s) else { return nil }
        value = v
    }

    /// From an AETH amount like "1.5" (at most 18 decimals).
    init?(aeth s: String) {
        let parts = s.trimmingCharacters(in: .whitespaces).split(separator: ".", omittingEmptySubsequences: false)
        guard (1...2).contains(parts.count), parts.allSatisfy({ $0.allSatisfy(\.isNumber) }) else { return nil }
        let whole = parts[0].isEmpty ? "0" : String(parts[0])
        let frac = parts.count == 2 ? String(parts[1]) : ""
        guard !(whole == "0" && frac.isEmpty && parts[0].isEmpty), frac.count <= 18,
              let w = UInt128(whole), let f = UInt128(frac.isEmpty ? "0" : frac) else { return nil }
        var scale: UInt128 = 1
        for _ in 0..<(18 - frac.count) { scale *= 10 }
        let (a, o1) = w.multipliedReportingOverflow(by: Self.unit)
        let (b, o2) = a.addingReportingOverflow(f * scale)
        guard !o1, !o2 else { return nil }
        value = b
    }

    /// Decimal AETH, trailing zeros trimmed.
    var aeth: String {
        let whole = value / Self.unit
        var frac = String(value % Self.unit)
        frac = String(repeating: "0", count: 18 - frac.count) + frac
        while frac.hasSuffix("0") { frac.removeLast() }
        return frac.isEmpty ? "\(whole)" : "\(whole).\(frac)"
    }

    var description: String { String(value) }

    static func + (a: Wei, b: Wei) -> Wei { Wei(value: a.value + b.value) }
    static func * (a: Wei, n: Int) -> Wei { Wei(value: a.value * UInt128(n)) }
    static func < (a: Wei, b: Wei) -> Bool { a.value < b.value }
}
