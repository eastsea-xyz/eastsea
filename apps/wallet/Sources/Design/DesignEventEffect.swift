#if canImport(SwiftUI)
import SwiftUI

func eastSeaWithoutAnimation(_ updates: () -> Void) {
    var transaction = Transaction(animation: nil)
    transaction.disablesAnimations = true
    withTransaction(transaction, updates)
}

/// A changed, non-nil event runs one animation. Existing events do not replay
/// on appearance. SwiftUI cancels the finite task on removal or ID changes.
@available(macOS 14.0, iOS 17.0, *)
struct EastSeaEventEffect<Event: Hashable, Overlay: View>: ViewModifier {
    let event: Event?
    let policy: DesignEffectPolicy
    let enabled: Bool
    let duration: Double
    let overlay: (TransientLightSample) -> Overlay
    @State private var observed: Event?
    @State private var progress = 1.0
    @State private var active = false

    init(event: Event?, policy: DesignEffectPolicy, enabled: Bool = true,
         duration: Double, @ViewBuilder overlay: @escaping (TransientLightSample) -> Overlay) {
        self.event = event
        self.policy = policy
        self.enabled = enabled
        self.duration = duration
        self.overlay = overlay
        _observed = State(initialValue: event)
    }

    private struct EffectID: Hashable {
        let event: Event?
        let policy: DesignEffectPolicy
        let enabled: Bool
    }

    func body(content: Content) -> some View {
        content
            .overlay {
                Group {
                    if active && enabled && policy.showsTransientLight {
                        TransientLightOverlay(progress: progress, overlay: overlay)
                            .allowsHitTesting(false)
                            .accessibilityHidden(true)
                    }
                }
                .transaction { transaction in
                    if !policy.showsTransientLight {
                        transaction.animation = nil
                        transaction.disablesAnimations = true
                    }
                }
            }
            .onAppear {
                observed = event
                reset()
            }
            .task(id: EffectID(event: event, policy: policy, enabled: enabled)) {
                guard event != observed else {
                    reset()
                    return
                }
                observed = event
                guard event != nil, enabled, policy.showsTransientLight,
                      duration.isFinite, duration > 0,
                      duration < Double(UInt64.max) / 1_000_000_000 else {
                    reset()
                    return
                }
                eastSeaWithoutAnimation {
                    progress = 0
                    active = true
                }
                do {
                    // One frame allows the zero-opacity initial sample to be
                    // committed before animation. This task never repeats.
                    try await Task.sleep(nanoseconds: 16_000_000)
                    try Task.checkCancellation()
                    withAnimation(DesignTokens.Motion.standard.animation(duration: duration)) {
                        progress = 1
                    }
                    try await Task.sleep(nanoseconds: UInt64(duration * 1_000_000_000))
                    try Task.checkCancellation()
                    reset()
                } catch {
                    // A replacement task owns the current state. Disappearance
                    // also clears state synchronously through onDisappear.
                }
            }
            .onDisappear(perform: reset)
    }

    private func reset() {
        eastSeaWithoutAnimation {
            active = false
            progress = 1
        }
    }
}

/// Sample inside the animatable view so a zero-opacity → zero-opacity envelope
/// still exposes its intermediate peak to SwiftUI's render-time interpolation.
private struct TransientLightOverlay<Overlay: View>: View, Animatable {
    var progress: Double
    let overlay: (TransientLightSample) -> Overlay

    var animatableData: Double {
        get { progress }
        set { progress = newValue }
    }

    var body: some View { overlay(TransientLightSample(progress: progress)) }
}
#endif
