import SwiftUI

/// One token's icon, drawn entirely in SwiftUI shapes — no raster assets and
/// no network of any kind (the security rules in TokenIconSpec.swift: an icon
/// is trust signage, so it comes from the shipped list or from the address
/// hash, never from a URL or a contract's metadata). Official art is
/// saturated brand colour with solid geometry; an unverified token's glyph is
/// a quiet pastel disc with a DASHED ring and a "?" corner badge, so the two
/// differ in shape, not only in colour, and read in light and dark alike
/// (official art is drawn white-on-brand, the glyph dark-ink-on-pastel, both
/// with their own background in every scheme). The native coin swaps its
/// vector art for the shipped `DoubloonCoin` render at 48 pt and above, where
/// the stamped detail reads; small rows keep the vector.
struct TokenIcon: View {
    let spec: TokenIconSpec
    var size: CGFloat = 28

    init(spec: TokenIconSpec, size: CGFloat = 28) {
        self.spec = spec
        self.size = size
    }

    /// The usual entry point: the same classification the pure tests check.
    init(chainId: UInt64, address: String?, symbol: String, size: CGFloat = 28) {
        self.init(spec: TokenIconSpec.of(chainId: chainId, address: address, symbol: symbol), size: size)
    }

    var body: some View {
        ZStack {
            switch spec.kind {
            case .nativeCoin:
                if size >= 48 {
                    Image("DoubloonCoin")
                        .resizable()
                        .interpolation(.high)
                        .aspectRatio(contentMode: .fit)
                } else {
                    DoubloonArt(size: size)
                }
            case .official(let symbol): OfficialTokenArt(symbol: symbol, size: size)
            case .generated(let letter, let seed): GeneratedGlyphArt(letter: letter, seed: seed, size: size)
            }
        }
        .frame(width: size, height: size)
        .accessibilityLabel(Text(spec.accessibilityLabel))
    }
}

// MARK: - The native coin: a gold doubloon

/// DBLN — a stamped gold coin: layered gold gradient, an inner rim and a
/// diamond punch in the middle.
private struct DoubloonArt: View {
    let size: CGFloat

    var body: some View {
        ZStack {
            Circle().fill(LinearGradient(colors: [Color(red: 0.98, green: 0.87, blue: 0.52),
                                                  Color(red: 0.90, green: 0.70, blue: 0.24),
                                                  Color(red: 0.72, green: 0.51, blue: 0.07)],
                                         startPoint: .topLeading, endPoint: .bottomTrailing))
            Circle().strokeBorder(Color.white.opacity(0.55), lineWidth: size * 0.045)
                .padding(size * 0.09)
            RoundedRectangle(cornerRadius: size * 0.03)
                .strokeBorder(Color(red: 0.55, green: 0.38, blue: 0.05), lineWidth: size * 0.05)
                .frame(width: size * 0.30, height: size * 0.30)
                .rotationEffect(.degrees(45))
        }
    }
}

// MARK: - Official tokens

/// The bundled art of the shipped list's tokens. Distinct at every size:
/// WAETH a wrapped diamond in deep blue, NEB a violet nebula of stars, ORB a
/// mint orb with an orbit ring, CMT a comet across a night disc. A symbol the
/// art table does not know yet (a list entry added without art) still reads
/// official: solid brand disc, solid ring, white letter — never a pastel
/// dashed glyph.
private struct OfficialTokenArt: View {
    let symbol: String
    let size: CGFloat

    var body: some View {
        switch symbol {
        case "WAETH": waeth
        case "NEB": neb
        case "ORB": orb
        case "CMT": cmt
        default: fallback
        }
    }

    /// Deep sea blue; the wrapped asset is a white diamond punched by its
    /// own chain's colour.
    private var waeth: some View {
        ZStack {
            Circle().fill(Color(red: 0.05, green: 0.35, blue: 0.65))
            Circle().strokeBorder(Color.white.opacity(0.85), lineWidth: size * 0.045)
                .padding(size * 0.13)
            RoundedRectangle(cornerRadius: size * 0.025)
                .fill(Color.white)
                .frame(width: size * 0.28, height: size * 0.28)
                .rotationEffect(.degrees(45))
            Circle().fill(Color(red: 0.05, green: 0.35, blue: 0.65))
                .frame(width: size * 0.10, height: size * 0.10)
        }
    }

    /// Violet night with a scatter of stars — a nebula, not a letter.
    private var neb: some View {
        ZStack {
            Circle().fill(Color(red: 0.42, green: 0.24, blue: 0.72))
            star(0.30, 0.28, 0.075)
            star(0.64, 0.34, 0.050)
            star(0.44, 0.60, 0.065)
            star(0.70, 0.66, 0.040)
            Circle().fill(Color.white)
                .frame(width: size * 0.10, height: size * 0.10)
                .offset(x: size * 0.06, y: -size * 0.05)
        }
    }

    /// A mint sea; the orb glows at the centre with an orbit running through.
    private var orb: some View {
        ZStack {
            Circle().fill(Color(red: 0.00, green: 0.52, blue: 0.58))
            Circle().fill(RadialGradient(colors: [Color.white, Color(red: 0.72, green: 0.97, blue: 0.95)],
                                         center: .center, startRadius: 0, endRadius: size * 0.22))
                .frame(width: size * 0.34, height: size * 0.34)
            Ellipse()
                .strokeBorder(Color.white.opacity(0.85), lineWidth: size * 0.045)
                .frame(width: size * 0.62, height: size * 0.24)
                .rotationEffect(.degrees(-24))
        }
    }

    /// A night-blue disc with a comet: bright head at the top right, its tail
    /// fading down-left.
    private var cmt: some View {
        ZStack {
            Circle().fill(Color(red: 0.10, green: 0.16, blue: 0.42))
            Capsule()
                .fill(LinearGradient(colors: [Color.white.opacity(0.9), Color.white.opacity(0)],
                                     startPoint: .trailing, endPoint: .leading))
                .frame(width: size * 0.38, height: size * 0.07)
                .rotationEffect(.degrees(-45))
                .offset(x: -size * 0.10, y: size * 0.10)
            Circle().fill(Color(red: 1.00, green: 0.62, blue: 0.25))
                .frame(width: size * 0.22, height: size * 0.22)
                .offset(x: size * 0.14, y: -size * 0.14)
            Circle().fill(Color.white)
                .frame(width: size * 0.13, height: size * 0.13)
                .offset(x: size * 0.14, y: -size * 0.14)
        }
    }

    /// Official, but not yet in the art table: brand disc, solid ring, the
    /// symbol's initial in white. Solid geometry marks it official; only the
    /// generated glyph ever dashes.
    private var fallback: some View {
        ZStack {
            Circle().fill(Color(red: 0.05, green: 0.35, blue: 0.65))
            Circle().strokeBorder(Color.white.opacity(0.85), lineWidth: size * 0.045)
                .padding(size * 0.10)
            Text(TokenIconSpec.glyphLetter(symbol))
                .font(.system(size: size * 0.44, weight: .bold, design: .rounded))
                .foregroundStyle(Color.white)
                .minimumScaleFactor(0.5)
        }
    }

    private func star(_ x: Double, _ y: Double, _ radius: Double) -> some View {
        Circle().fill(Color.white)
            .frame(width: size * radius * 2, height: size * radius * 2)
            .position(x: size * x, y: size * y)
    }
}

// MARK: - Every other token: a deterministic generated glyph

/// The unverified glyph: a pastel disc coloured by the address hash, the
/// symbol's initial in dark ink, a DASHED ring and a "?" corner badge — shape
/// signals a colour-blind eye can rely on, and that no official icon shares.
private struct GeneratedGlyphArt: View {
    let letter: String
    let seed: UInt32
    let size: CGFloat

    private var ink: Color { Color(red: 0.13, green: 0.13, blue: 0.16) }

    var body: some View {
        let rgb = TokenIconSpec.rgb(seed: seed)
        return ZStack {
            Circle().fill(Color(red: rgb.0, green: rgb.1, blue: rgb.2))
            Circle()
                .strokeBorder(ink.opacity(0.55),
                              style: StrokeStyle(lineWidth: size * 0.05, dash: [size * 0.075, size * 0.055]))
                .padding(size * 0.045)
            Text(letter)
                .font(.system(size: size * 0.42, weight: .bold, design: .rounded))
                .foregroundStyle(ink)
                .minimumScaleFactor(0.5)
            // The "?" badge: unverified, said in shape as well as in colour.
            Circle()
                .fill(Color.white)
                .frame(width: size * 0.40, height: size * 0.40)
                .overlay(Circle().strokeBorder(ink.opacity(0.55), lineWidth: max(0.75, size * 0.035)))
                .overlay(Text("?").font(.system(size: size * 0.26, weight: .bold, design: .rounded))
                    .foregroundStyle(ink).minimumScaleFactor(0.4))
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomTrailing)
        }
    }
}

// MARK: - Design preview

#if DEBUG
/// `-designPreview icons`: every icon kind side by side, for screenshots.
struct TokenIconPreviewRow: View {
    var body: some View {
        HStack(spacing: 14) {
            TokenIcon(chainId: 7780, address: nil, symbol: Brand.coinTicker, size: 40)
            TokenIcon(chainId: 7780, address: "0xa2521982a17474cb2f8741c85de653b5282d72b0", symbol: "WAETH", size: 40)
            TokenIcon(chainId: 7780, address: "0x6bc5ded76ccbdc8df35e7cd28b68fed245a74416", symbol: "NEB", size: 40)
            TokenIcon(chainId: 7780, address: "0x961f8add5ae93ff0700be8abd5f9f8ec69ba4347", symbol: "ORB", size: 40)
            TokenIcon(chainId: 7780, address: "0xc91367bac92c6de822de8afd0f34ff19fd8f7670", symbol: "CMT", size: 40)
            TokenIcon(chainId: 7780, address: "0x00000000000000000000000000000000000000c1", symbol: "USDX", size: 40)
            TokenIcon(chainId: 7780, address: "0x00000000000000000000000000000000000000d4", symbol: "VVDBLN", size: 40)
        }
        .padding(24)
    }
}
#endif
