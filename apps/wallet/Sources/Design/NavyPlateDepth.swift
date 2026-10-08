#if canImport(SwiftUI)
import SwiftUI

/// Fixed light and engraved arcs create depth without a timer, pointer tracker
/// or animated shader. Reduced transparency substitutes an opaque plate edge.
@available(macOS 14.0, iOS 17.0, *)
struct NavyPlateDepth: View {
    let cornerRadius: Double
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    init(cornerRadius: Double = DesignTokens.Radius.plate) {
        self.cornerRadius = cornerRadius
    }

    var body: some View {
        let policy = DesignEffectPolicy(reduceMotion: reduceMotion, reduceTransparency: reduceTransparency)
        let shape = RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
        let shadows = colorScheme == .dark ? DesignTokens.Shadows.plate.dark : DesignTokens.Shadows.plate.light
        let shadow = shadows.first
        ZStack {
            if policy.usesCompositedDepth {
                shape.fill(LinearGradient(colors: [DesignTokens.Palette.plate.color, DesignTokens.Palette.plate2.color],
                                          startPoint: .topLeading, endPoint: .bottomTrailing))
                GeometryReader { geometry in
                    PlateEngraving()
                        .stroke(DesignTokens.Palette.seaLine.color.opacity(0.15), lineWidth: 0.75)
                        .frame(width: geometry.size.width, height: geometry.size.height)
                }
                shape.strokeBorder(DesignTokens.Palette.plateSoft.color.opacity(0.22), lineWidth: 0.75)
            } else {
                shape.fill(DesignTokens.Palette.plate.color)
                shape.strokeBorder(DesignTokens.Palette.plate2.color, lineWidth: 1)
            }
        }
        .clipShape(shape)
        .shadow(color: policy.usesCompositedDepth ? (shadow?.color ?? .clear) : .clear,
                radius: policy.usesCompositedDepth ? (shadow?.radius ?? 0) : 0,
                x: shadow?.x ?? 0, y: shadow?.y ?? 0)
        .allowsHitTesting(false)
        .accessibilityHidden(true)
        .transaction { transaction in
            transaction.animation = nil
            transaction.disablesAnimations = true
        }
    }
}

private struct PlateEngraving: Shape {
    func path(in rect: CGRect) -> Path {
        var path = Path()
        for index in 0..<4 {
            let shift = Double(index) * 14
            path.move(to: CGPoint(x: rect.width * 0.42, y: rect.height + 12 + shift))
            path.addCurve(to: CGPoint(x: rect.width + 20, y: rect.height * 0.24 + shift),
                          control1: CGPoint(x: rect.width * 0.58, y: rect.height * 0.44 + shift),
                          control2: CGPoint(x: rect.width * 0.84, y: rect.height * 0.82 + shift))
        }
        return path
    }
}

@available(macOS 14.0, iOS 17.0, *)
extension View {
    func eastSeaNavyPlate(cornerRadius: Double = DesignTokens.Radius.plate) -> some View {
        background(NavyPlateDepth(cornerRadius: cornerRadius))
    }
}
#endif
