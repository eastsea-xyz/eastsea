#if canImport(SwiftUI)
import SwiftUI

@available(macOS 14.0, iOS 17.0, *)
private struct EastSeaPresentation<Value: Equatable>: ViewModifier {
    let style: EastSeaPresentationStyle
    let value: Value
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    private var policy: DesignEffectPolicy {
        DesignEffectPolicy(reduceMotion: reduceMotion, reduceTransparency: reduceTransparency)
    }

    private var transition: AnyTransition {
        guard policy.animatesPresentation else { return .identity }
        let movement: AnyTransition = style == .sheet
            ? .move(edge: .bottom) : .scale(scale: 0.985, anchor: .top)
        return reduceTransparency ? movement : movement.combined(with: .opacity)
    }

    private var animation: Animation? {
        guard policy.animatesPresentation else { return nil }
        return style == .sheet ? DesignTokens.Motion.sheet.animation : DesignTokens.Motion.panel.animation
    }

    func body(content: Content) -> some View {
        content
            .transition(transition)
            .animation(animation, value: value)
            .transaction { transaction in
                if !policy.animatesPresentation {
                    transaction.animation = nil
                    transaction.disablesAnimations = true
                }
            }
    }
}

@available(macOS 14.0, iOS 17.0, *)
extension View {
    /// Apply to the conditionally presented content and animate its container
    /// with the same visibility value. Reduced transparency keeps an opaque move.
    func eastSeaPresentation<Value: Equatable>(_ style: EastSeaPresentationStyle, value: Value) -> some View {
        modifier(EastSeaPresentation(style: style, value: value))
    }
}
#endif
