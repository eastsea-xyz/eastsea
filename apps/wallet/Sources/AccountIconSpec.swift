import CryptoKit
import Foundation

/// Archipelago v2: an address-derived recognition aid, never proof of identity.
/// Only decoded address bytes enter the hash. Invalid input has no icon spec.
struct AccountIconSpec: Equatable, Hashable, Sendable {
    struct Palette: Decodable, Equatable, Hashable, Sendable {
        let name: String
        let start: String
        let end: String
        let ink: String
    }

    struct Silhouette: Decodable, Equatable, Hashable, Sendable {
        let name: String
        let path: String
    }

    static let domain = "eastsea-account-icon-v2"
    static let palettes = [
        Palette(name: "tidal", start: "#209792", end: "#1d8781", ink: "#0d2135"),
        Palette(name: "coral", start: "#ca2b2b", end: "#b92727", ink: "#eed7a0"),
        Palette(name: "cove", start: "#2f62da", end: "#2558d0", ink: "#eed7a0"),
        Palette(name: "seagrass", start: "#429c1c", end: "#3b8b18", ink: "#0d2135"),
        Palette(name: "anemone", start: "#c7237e", end: "#b62073", ink: "#eed7a0"),
        Palette(name: "gold", start: "#aa8518", end: "#987716", ink: "#0d2135"),
        Palette(name: "azure", start: "#298ee0", end: "#1f84d6", ink: "#0d2135"),
        Palette(name: "reef", start: "#257e52", end: "#206f47", ink: "#eed7a0"),
        Palette(name: "rose", start: "#d36979", end: "#cf596b", ink: "#0d2135"),
        Palette(name: "kelp", start: "#6e7722", end: "#5f671e", ink: "#eed7a0"),
        Palette(name: "orchid", start: "#e444d4", end: "#e232d0", ink: "#0d2135"),
        Palette(name: "sea", start: "#257793", end: "#216a83", ink: "#eed7a0"),
        Palette(name: "dawn", start: "#df6320", end: "#cd5b1d", ink: "#0d2135"),
        Palette(name: "iris", start: "#a029e0", end: "#961fd6", ink: "#eed7a0"),
        Palette(name: "copper", start: "#96612c", end: "#865727", ink: "#eed7a0"),
        Palette(name: "lilac", start: "#9579d8", end: "#8969d3", ink: "#0d2135"),
    ]
    static let silhouettes = [
        Silhouette(name: "cove", path: "M 52 12 C 36 4 13 10 10 28 C 7 45 25 55 43 48 L 47 38 C 33 44 21 40 22 29 C 23 19 35 15 48 23 Z"),
        Silhouette(name: "headland", path: "M 12 47 L 12 34 C 22 32 19 18 29 11 C 37 5 51 12 53 24 C 55 35 44 41 35 38 C 29 36 29 48 22 50 Z"),
        Silhouette(name: "sandbar", path: "M 10 40 C 13 29 21 28 29 26 C 35 24 37 10 48 10 L 55 21 C 44 22 46 35 35 38 C 27 41 21 38 17 51 Z"),
        Silhouette(name: "twin peaks", path: "M 9 43 L 19 15 C 21 9 25 9 28 17 L 33 29 L 42 12 C 45 7 48 9 50 17 L 56 43 C 42 51 24 51 9 43 Z"),
        Silhouette(name: "reef", path: "M 8 32 L 25 9 C 28 6 32 8 32 13 L 29 24 L 50 16 C 56 14 58 20 53 25 L 35 48 C 31 54 26 51 28 45 L 32 34 L 13 42 C 7 45 5 39 8 32 Z"),
        Silhouette(name: "breaker", path: "M 8 43 C 16 39 17 18 31 11 C 44 4 56 14 54 27 C 48 18 36 17 34 28 C 40 26 51 32 56 43 C 40 52 22 51 8 43 Z"),
        Silhouette(name: "inlet", path: "M 10 48 L 10 26 C 10 6 52 6 52 26 L 52 48 L 40 48 L 40 29 C 40 21 22 21 22 29 L 22 48 Z"),
        Silhouette(name: "delta", path: "M 27 50 L 25 32 L 9 20 L 14 9 L 31 22 L 48 9 L 56 18 L 39 34 L 40 50 Z"),
        Silhouette(name: "spit", path: "M 11 48 C 9 26 23 9 51 10 C 49 33 33 49 11 48 Z"),
        Silhouette(name: "shelf", path: "M 10 15 L 32 10 L 33 25 L 52 20 L 55 40 L 39 50 L 12 45 C 8 34 8 25 10 15 Z"),
        Silhouette(name: "hook", path: "M 11 10 L 25 10 L 25 32 C 25 45 43 44 43 32 L 43 22 L 55 22 L 55 35 C 55 59 11 58 11 35 Z"),
        Silhouette(name: "crescent", path: "M 50 8 C 23 4 8 19 10 36 C 12 52 33 58 52 45 C 30 43 29 23 50 8 Z"),
        Silhouette(name: "ridge", path: "M 8 42 L 14 27 L 25 30 L 31 9 L 43 24 L 51 18 L 57 42 C 39 51 24 50 8 42 Z"),
        Silhouette(name: "estuary", path: "M 10 13 L 24 11 L 32 28 L 41 10 L 55 15 L 42 33 L 51 47 L 35 51 L 28 39 L 13 48 L 8 34 L 23 29 Z"),
        Silhouette(name: "arch", path: "M 8 44 C 9 27 18 8 32 8 C 46 8 55 27 56 44 L 42 47 C 42 33 37 24 32 24 C 27 24 22 33 22 47 Z"),
        Silhouette(name: "tidal pool", path: "M 54 27 C 54 47 38 55 21 48 C 6 41 8 18 23 11 C 39 3 51 13 46 27 C 42 37 30 39 25 29 C 29 32 36 29 35 23 C 34 16 21 21 21 31 C 21 43 43 42 43 29 Z"),
    ]

    let version: UInt8
    let palette: UInt8
    let layout: UInt16
    let shape: UInt8
    let rotation: UInt8

    private init(palette: UInt8, layout: UInt16, shape: UInt8, rotation: UInt8) {
        version = 2
        self.palette = palette
        self.layout = layout
        self.shape = shape
        self.rotation = rotation
    }

    /// The default is explicitly v2. Future versions must use a new domain
    /// and published vectors; they cannot silently reinterpret this spec.
    static func of(address: String?, version: UInt8 = 2) -> AccountIconSpec? {
        guard version == 2, let address, let bytes = addressBytes(address) else { return nil }
        let seed = Array(SHA256.hash(data: Data(domain.utf8) + Data(bytes)))
        return AccountIconSpec(palette: seed[0] & 15,
                               layout: ((UInt16(seed[1]) << 8) | UInt16(seed[2])) & 0x3fff,
                               shape: (seed[0] >> 4) & 3, rotation: (seed[0] >> 6) & 3)
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

    var colors: Palette { Self.palettes[Int(palette)] }

    /// One of 16 broad coastlines, readable without satellite detail at 16 px.
    var silhouetteClass: UInt8 { shape * 4 + UInt8(layout & 3) }
    var mainPath: String { Self.silhouettes[Int(silhouetteClass)].path }

    /// At 32 px and above, the remaining layout bits vary two large islands.
    var satellitePaths: [String] {
        (0..<2).map { index in
            let bits = Int((layout >> (2 + index * 6)) & 63)
            let x = 9 + index * 25 + (bits & 3)
            let y = 46 + ((bits >> 2) & 3)
            let width = 14 + ((bits >> 4) & 3)
            return "M \(x) \(y + 4) C \(x + 2) \(y - 3) \(x + width - 5) \(y + 1) \(x + width - 2) \(y - 2) L \(x + width) \(y + 6) C \(x + width - 3) \(y + 11) \(x + 3) \(y + 13) \(x) \(y + 4) Z"
        }
    }

    /// Canonical UTF-8 SVG with the same size threshold and geometry as Canvas.
    /// No trailing newline; nonpositive sizes have no SVG.
    func svg(size: Int = 64) -> String? {
        guard size > 0 else { return nil }
        let colors = colors
        let id = "eastsea-island-v2-\(palette)"
        var svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"\(size)\" height=\"\(size)\" viewBox=\"0 0 64 64\" aria-hidden=\"true\">"
        svg += "<defs><linearGradient id=\"\(id)\" x1=\"0\" y1=\"0\" x2=\"64\" y2=\"64\" gradientUnits=\"userSpaceOnUse\" color-interpolation=\"sRGB\"><stop offset=\"0\" stop-color=\"\(colors.start)\"/><stop offset=\"1\" stop-color=\"\(colors.end)\"/></linearGradient></defs>"
        svg += "<rect width=\"64\" height=\"64\" rx=\"12\" fill=\"url(#\(id))\"/>"
        svg += "<g fill=\"\(colors.ink)\" transform=\"rotate(\(Int(rotation) * 90) 32 32)\">"
        svg += "<path d=\"\(mainPath)\" transform=\"translate(0 \(size < 32 ? 5 : 0)) scale(1 0.8)\"/>"
        if size >= 32 {
            for path in satellitePaths { svg += "<path d=\"\(path)\"/>" }
        }
        return svg + "</g></svg>"
    }

    var svg64: String { svg()! }
}
