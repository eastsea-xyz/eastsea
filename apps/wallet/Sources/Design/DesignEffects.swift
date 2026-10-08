import Foundation

/// Presentation policy only. No balance, reward or node state is derived from an effect.
struct DesignEffectPolicy: Hashable {
    let reduceMotion: Bool
    let reduceTransparency: Bool

    var animatesBalance: Bool { !reduceMotion }
    var animatesPresentation: Bool { !reduceMotion }
    var showsTransientLight: Bool { !reduceMotion && !reduceTransparency }
    var usesCompositedDepth: Bool { !reduceTransparency }

    // Haptics do not composite pixels. Reduce Transparency leaves them unchanged.
    func emitsSuccessFeedback(enabled: Bool) -> Bool { enabled && !reduceMotion }
}

enum EastSeaPresentationStyle: Hashable {
    case panel
    case sheet
}

enum EastSeaNodeStatus: Hashable {
    case connected
    case checking
    case paused
    case offline

    var permitsPulse: Bool { self == .connected || self == .checking }
}

/// Intermediate numbers are decorative. The final number always returns the
/// original Decimal, including digits beyond Double's precision.
enum BalanceCountUpValue {
    private static let largestExactInteger = Decimal(string: "9007199254740992")!

    static func presentationValue(_ amount: Decimal) -> Double {
        let value = NSDecimalNumber(decimal: amount).doubleValue
        return value.isFinite ? value : 0
    }

    static func canAnimate(from: Decimal, to: Decimal) -> Bool {
        guard !from.isNaN, !to.isNaN, from != to else { return false }
        let start = NSDecimalNumber(decimal: from).doubleValue
        let end = NSDecimalNumber(decimal: to).doubleValue
        return start.isFinite && end.isFinite && start != end
            && from >= -largestExactInteger && from <= largestExactInteger
            && to >= -largestExactInteger && to <= largestExactInteger
    }

    static func sample(presentation: Double, target: Decimal) -> Decimal {
        guard !target.isNaN, presentation.isFinite else { return target }
        let finalPresentation = NSDecimalNumber(decimal: target).doubleValue
        if presentation == finalPresentation { return target }
        return Decimal(presentation)
    }
}

/// Bounded samples for a single event. Both endpoints are invisible, allowing
/// the overlay to be removed completely once the finite animation completes.
struct TransientLightSample: Equatable {
    let progress: Double
    let opacity: Double
    let travel: Double

    init(progress: Double) {
        let bounded = progress.isFinite ? min(1, max(0, progress)) : 1
        self.progress = bounded
        opacity = 4 * bounded * (1 - bounded)
        travel = 2 * bounded - 1
    }
}
