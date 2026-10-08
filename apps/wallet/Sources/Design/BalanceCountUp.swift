#if canImport(SwiftUI)
import SwiftUI

/// Inherits its font and colour. Supply the same formatter used for settled
/// balances and an exact, localized accessibility label (including the ticker).
@available(macOS 14.0, iOS 17.0, *)
struct BalanceCountUp: View {
    let amount: Decimal
    let accessibilityText: String
    let format: (Decimal) -> String
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @State private var presentation: Double

    init(amount: Decimal, accessibilityText: String, format: @escaping (Decimal) -> String) {
        self.amount = amount
        self.accessibilityText = accessibilityText
        self.format = format
        _presentation = State(initialValue: BalanceCountUpValue.presentationValue(amount))
    }

    private var policy: DesignEffectPolicy {
        DesignEffectPolicy(reduceMotion: reduceMotion, reduceTransparency: reduceTransparency)
    }

    var body: some View {
        AnimatedAmount(presentation: presentation, target: amount, format: format)
            .monospacedDigit()
            .accessibilityLabel(accessibilityText)
            .onAppear(perform: settle)
            .onChange(of: amount) { old, new in
                let next = BalanceCountUpValue.presentationValue(new)
                if policy.animatesBalance && BalanceCountUpValue.canAnimate(from: old, to: new) {
                    withAnimation(DesignTokens.Motion.standard.animation(duration: DesignTokens.Motion.base)) {
                        presentation = next
                    }
                } else {
                    eastSeaWithoutAnimation { presentation = next }
                }
            }
            .onChange(of: reduceMotion) { _, reduced in
                if reduced { settle() }
            }
            .onDisappear(perform: settle)
            .transaction { transaction in
                if reduceMotion {
                    transaction.animation = nil
                    transaction.disablesAnimations = true
                }
            }
    }

    private func settle() {
        eastSeaWithoutAnimation { presentation = BalanceCountUpValue.presentationValue(amount) }
    }
}

private struct AnimatedAmount: View, Animatable {
    var presentation: Double
    let target: Decimal
    let format: (Decimal) -> String

    var animatableData: Double {
        get { presentation }
        set { presentation = newValue }
    }

    var body: some View {
        let sample = BalanceCountUpValue.sample(presentation: presentation, target: target)
        Text(sample.isNaN ? "—" : format(sample))
    }
}
#endif
