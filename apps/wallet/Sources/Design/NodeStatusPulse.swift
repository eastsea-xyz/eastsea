#if canImport(SwiftUI)
import SwiftUI

/// The accompanying sentence supplies meaning. Pulse only on a real node
/// event (for example, a newly verified block); a connected node is still.
@available(macOS 14.0, iOS 17.0, *)
struct NodeStatusPulse<Event: Hashable>: View {
    let status: EastSeaNodeStatus
    let event: Event?
    let accessibilityLabel: String
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    init(status: EastSeaNodeStatus, event: Event?, accessibilityLabel: String) {
        self.status = status
        self.event = event
        self.accessibilityLabel = accessibilityLabel
    }

    private var colour: Color {
        switch status {
        case .connected: return DesignTokens.Palette.success.color
        case .checking: return DesignTokens.Palette.accent.color
        case .paused, .offline: return DesignTokens.Palette.warn.color
        }
    }

    var body: some View {
        let policy = DesignEffectPolicy(reduceMotion: reduceMotion, reduceTransparency: reduceTransparency)
        Group {
            switch status {
            case .connected:
                Circle().fill(colour).frame(width: 6, height: 6)
            case .checking:
                Circle().strokeBorder(colour, lineWidth: 1.5).frame(width: 8, height: 8)
            case .paused:
                Image(systemName: "pause.fill").font(.system(size: 10, weight: .medium)).foregroundStyle(colour)
            case .offline:
                Image(systemName: "wifi.slash").font(.system(size: 11, weight: .medium)).foregroundStyle(colour)
            }
        }
        .frame(width: 16, height: 16)
        .modifier(EastSeaEventEffect(event: event, policy: policy, enabled: status.permitsPulse,
                                    duration: DesignTokens.Motion.slow) { sample in
            Circle()
                .strokeBorder(colour, lineWidth: 1)
                .frame(width: 8, height: 8)
                .scaleEffect(1 + sample.progress * 1.6)
                .opacity(sample.opacity * 0.42)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        })
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(accessibilityLabel)
        .transaction { transaction in
            if reduceMotion {
                transaction.animation = nil
                transaction.disablesAnimations = true
            }
        }
    }
}
#endif
