import Foundation

// Which icon a token gets, as a pure value (the token-icon security rules,
// adopted 2026-10-04). An icon is trust signage, so NOTHING a token contract
// or a node supplies can influence it: no URL is ever fetched (icons carry no
// network at all, not even IPFS), and the official art belongs to the shipped
// trust list alone (apps/wallet/Sources/TokenGuard.swift `KnownTokens`, plus
// the native coin). The list is keyed by address, so a look-alike symbol on an
// unknown address can never reach it — classification never even looks at the
// symbol for that. Every other token gets a deterministic generated glyph
// derived only from its lowercase address (FNV-1a → hue) and the first letter
// of its symbol, drawn so it cannot be mistaken for official art: pastel
// fill, dashed ring and a "?" badge (shape, not just colour). Everything here
// is Foundation only, so apps/wallet/Tests/token-icon checks it without an
// app or a node.

/// The kind of mark a token carries: bundled art for the official few, a
/// deterministic generated glyph for everything else.
enum TokenIconKind: Equatable {
    /// The chain's native coin (DBLN): a gold doubloon.
    case nativeCoin
    /// A token on the shipped trust list. The symbol is the LIST's (the
    /// node's claim of it is irrelevant to the art).
    case official(symbol: String)
    /// Everything else: the uppercased first letter of the symbol on a
    /// background coloured by the address hash.
    case generated(letter: String, seed: UInt32)
}

/// The icon decision for one token, computed from the shipped list and the
/// address alone. The SwiftUI drawing of this lives in TokenIconView.swift.
struct TokenIconSpec: Equatable {
    let kind: TokenIconKind

    /// On the shipped list (or the native coin): may carry official art.
    var verified: Bool {
        if case .generated = kind { return false }
        return true
    }

    /// What VoiceOver says: verification is part of the label, never only a
    /// colour ("WAETH, verified", "Unverified token").
    var accessibilityLabel: String {
        switch kind {
        case .nativeCoin: return "\(Brand.coinTicker), verified"
        case .official(let symbol): return "\(symbol), verified"
        case .generated: return "Unverified token"
        }
    }

    /// `address` nil or empty means the native coin; otherwise the shipped
    /// list decides by address (case-insensitively, per chain), and anything
    /// unknown falls to the generated glyph. `symbol` only names the token in
    /// the generated letter — it can never promote a token to official art.
    static func of(chainId: UInt64, address: String?, symbol: String) -> TokenIconSpec {
        guard let address, !address.isEmpty else { return TokenIconSpec(kind: .nativeCoin) }
        if let known = KnownTokens.knownToken(chainId: chainId, address: address) {
            return TokenIconSpec(kind: .official(symbol: known.symbol))
        }
        return TokenIconSpec(kind: .generated(letter: glyphLetter(symbol), seed: seed(of: address)))
    }

    /// The glyph letter: the symbol's first non-space character, uppercased;
    /// "?" when there is nothing to show.
    static func glyphLetter(_ symbol: String) -> String {
        guard let first = symbol.trimmingCharacters(in: .whitespaces).first else { return "?" }
        return String(first).uppercased()
    }

    /// FNV-1a of the lowercase address: two spellings of one address get one
    /// glyph; two addresses almost surely get different hues.
    static func seed(of address: String) -> UInt32 {
        var hash: UInt32 = 0x811c_9dc5
        for byte in address.lowercased().utf8 {
            hash = (hash ^ UInt32(byte)) &* 0x0100_0193
        }
        return hash
    }

    /// The glyph background's hue, 0...1. A golden-ratio multiply spreads
    /// neighbouring addresses across the wheel.
    static func hue(seed: UInt32) -> Double {
        Double((seed &* 2_654_435_761) % 360) / 360
    }

    /// The glyph background as pastel RGB (each channel 0...1) — quieter than
    /// the official art's saturated brand colours, so the two never blur.
    static func rgb(seed: UInt32) -> (Double, Double, Double) {
        hslToRgb(h: hue(seed: seed), s: 0.45, l: 0.62)
    }

    /// HSL → RGB (Foundation has no colour type; the view turns these into a
    /// SwiftUI Color). h, s, l in 0...1.
    private static func hslToRgb(h: Double, s: Double, l: Double) -> (Double, Double, Double) {
        let c = (1 - abs(2 * l - 1)) * s
        let hp = h * 6
        let x = c * (1 - abs(hp.truncatingRemainder(dividingBy: 2) - 1))
        let rgb1: (Double, Double, Double)
        switch hp {
        case ..<1: rgb1 = (c, x, 0)
        case ..<2: rgb1 = (x, c, 0)
        case ..<3: rgb1 = (0, c, x)
        case ..<4: rgb1 = (0, x, c)
        case ..<5: rgb1 = (x, 0, c)
        default: rgb1 = (c, 0, x)
        }
        let m = l - c / 2
        return (rgb1.0 + m, rgb1.1 + m, rgb1.2 + m)
    }
}
