import CryptoKit
import Foundation

/// Archipelago v1: an address-derived recognition aid, never proof of identity.
/// Only decoded address bytes enter the hash. Invalid input has no icon spec.
struct AccountIconSpec: Equatable, Hashable, Sendable {
    static let domain = "eastsea-account-icon-v1"
    static let palettes = ["#4e8dad", "#368f8b", "#73864a", "#a77a45",
                           "#b36c5c", "#96749e", "#758694", "#958130"]
    static let ink = "#101820"

    let version: UInt8
    let palette: UInt8
    let layout: UInt16
    let shape: UInt8
    let rotation: UInt8

    private init(palette: UInt8, layout: UInt16, shape: UInt8, rotation: UInt8) {
        version = 1
        self.palette = palette
        self.layout = layout
        self.shape = shape
        self.rotation = rotation
    }

    /// The default is explicitly v1. Future versions must use a new domain
    /// and published vectors; they cannot silently reinterpret this spec.
    static func of(address: String?, version: UInt8 = 1) -> AccountIconSpec? {
        guard version == 1, let address, let bytes = addressBytes(address) else { return nil }
        let seed = Array(SHA256.hash(data: Data(domain.utf8) + Data(bytes)))
        return AccountIconSpec(palette: seed[0] & 7,
                               layout: ((UInt16(seed[1]) << 8) | UInt16(seed[2])) & 0x3fff,
                               shape: (seed[0] >> 3) & 3, rotation: (seed[0] >> 5) & 3)
    }

    /// Exactly 40 ASCII hex digits, optionally prefixed by 0x or 0X.
    /// No trimming, Unicode digit folding, names or shortened addresses.
    private static func addressBytes(_ address: String) -> [UInt8]? {
        let text = Array(address.utf8)
        let start: Int
        if text.count == 42, text[0] == 48, text[1] == 120 || text[1] == 88 {
            start = 2
        } else if text.count == 40 {
            start = 0
        } else {
            return nil
        }
        var bytes: [UInt8] = []
        bytes.reserveCapacity(20)
        for i in stride(from: start, to: text.count, by: 2) {
            guard let high = hexNibble(text[i]), let low = hexNibble(text[i + 1]) else { return nil }
            bytes.append((high << 4) | low)
        }
        return bytes
    }

    private static func hexNibble(_ byte: UInt8) -> UInt8? {
        switch byte {
        case 48...57: byte - 48
        case 65...70: byte - 55
        case 97...102: byte - 87
        default: nil
        }
    }

    var paletteHex: String { Self.palettes[Int(palette)] }

    /// Row-major cells before rotation: one fixed island and one fixed sea.
    func isOccupied(_ cell: Int) -> Bool {
        guard (0..<16).contains(cell) else { return false }
        if cell == 0 { return true }
        if cell == 15 { return false }
        return layout & (UInt16(1) << (cell - 1)) != 0
    }

    var occupiedCells: [Int] { (0..<16).filter(isOccupied) }

    /// Coarse mask after the actual clockwise rotation. Different tuples can
    /// alias this mask, so measurements must not count tuples as unique art.
    var rotatedMask: UInt16 {
        occupiedCells.reduce(0) { mask, cell in
            var x = cell % 4
            var y = cell / 4
            for _ in 0..<rotation { (x, y) = (3 - y, x) }
            return mask | (UInt16(1) << (y * 4 + x))
        }
    }

    /// Canonical SVG for cross-language drawing goldens and static review.
    /// Production SwiftUI paints this same 64-unit geometry at every size.
    var svg64: String {
        var svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"64\" height=\"64\" viewBox=\"0 0 64 64\" aria-hidden=\"true\">"
        svg += "<rect width=\"64\" height=\"64\" rx=\"12\" fill=\"\(paletteHex)\"/>"
        svg += "<g fill=\"\(Self.ink)\" transform=\"rotate(\(Int(rotation) * 90) 32 32)\">"
        for cell in occupiedCells {
            let x = 9 + 12 * (cell % 4)
            let y = 9 + 12 * (cell / 4)
            switch shape {
            case 0: svg += "<rect x=\"\(x)\" y=\"\(y)\" width=\"10\" height=\"10\"/>"
            case 1: svg += "<circle cx=\"\(x + 5)\" cy=\"\(y + 5)\" r=\"5\"/>"
            case 2: svg += "<path d=\"M\(x + 5) \(y)L\(x + 10) \(y + 10)L\(x) \(y + 10)Z\"/>"
            default: svg += "<path d=\"M\(x) \(y)L\(x + 10) \(y)A10 10 0 0 1 \(x) \(y + 10)Z\"/>"
            }
        }
        return svg + "</g></svg>"
    }
}
