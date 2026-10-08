#if canImport(SwiftUI)
import SwiftUI
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

@available(macOS 14.0, iOS 17.0, *)
private struct EastSeaSuccessFeedback<Event: Hashable>: ViewModifier {
    let event: Event?
    let enabled: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    func body(content: Content) -> some View {
        content.onChange(of: event) { previous, current in
            let policy = DesignEffectPolicy(reduceMotion: reduceMotion, reduceTransparency: reduceTransparency)
            guard current != nil, previous != current, policy.emitsSuccessFeedback(enabled: enabled) else { return }
            #if os(iOS)
            UINotificationFeedbackGenerator().notificationOccurred(.success)
            #elseif os(macOS)
            // Supported trackpads provide a single native acknowledgement.
            NSHapticFeedbackManager.defaultPerformer.perform(.generic, performanceTime: .now)
            #endif
        }
    }
}

@available(macOS 14.0, iOS 17.0, *)
extension View {
    /// Trigger from a confirmed success ID and pass the user's haptics setting.
    /// An existing ID does not fire on appearance; nil clears without feedback.
    func eastSeaSuccessFeedback<Event: Hashable>(event: Event?, enabled: Bool = true) -> some View {
        modifier(EastSeaSuccessFeedback(event: event, enabled: enabled))
    }
}
#endif
