#if canImport(SwiftUI)
import SwiftUI

@available(macOS 14.0, iOS 17.0, *)
private struct DBLNRewardShine<Event: Hashable>: ViewModifier {
    let arrival: Event?
    let cornerRadius: Double
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    func body(content: Content) -> some View {
        let policy = DesignEffectPolicy(reduceMotion: reduceMotion, reduceTransparency: reduceTransparency)
        content.modifier(EastSeaEventEffect(event: arrival, policy: policy, duration: DesignTokens.Motion.slow) { sample in
            GeometryReader { geometry in
                ZStack {
                    RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                        .strokeBorder(DesignTokens.Palette.gold.color, lineWidth: 1)
                        .opacity(sample.opacity * 0.36)
                    Rectangle()
                        .fill(LinearGradient(
                            colors: [.clear, DesignTokens.Palette.gold.color.opacity(0.34), .clear],
                            startPoint: .leading, endPoint: .trailing))
                        .frame(width: max(24, geometry.size.width * 0.22), height: geometry.size.height * 2)
                        .rotationEffect(.degrees(18))
                        .offset(x: sample.travel * (geometry.size.width + geometry.size.height) * 0.8)
                        .opacity(sample.opacity)
                }
                .frame(width: geometry.size.width, height: geometry.size.height)
                .clipShape(RoundedRectangle(cornerRadius: cornerRadius, style: .continuous))
            }
        })
    }
}

@available(macOS 14.0, iOS 17.0, *)
extension View {
    /// Pass a new reward identifier only after a verified DBLN arrival. A
    /// unchanged ID, an initial ID, or nil does not celebrate an old reward.
    func dblnRewardShine<Event: Hashable>(arrival: Event?, cornerRadius: Double = DesignTokens.Radius.plate) -> some View {
        modifier(DBLNRewardShine(arrival: arrival, cornerRadius: cornerRadius))
    }
}
#endif
