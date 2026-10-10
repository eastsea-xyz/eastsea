#if os(macOS)
import Foundation
import SwiftUI

/// Created before the wallet or node can start work. Its record belongs to
/// the wallet, so recovery does not need to open or migrate the node folder.
@MainActor
final class LaunchRecovery: ObservableObject {
    static let shared = LaunchRecovery()

    @Published private(set) var isSafeMode = false
    private var tracker: LaunchTracker
    private var healthTimer: Timer?
    private var healthWindowID: UUID?
    private let clock: Clock = UptimeClock()
    private var healthStartedAt = MonotonicInstant.distantPast
    private let recording: Bool

    private init() {
        #if WALLET_SCREENS
        recording = false
        #else
        #if DEBUG
        recording = !DesignPreview.on
        #else
        recording = true
        #endif
        #endif
        tracker = LaunchTracker(recordURL: recording ? LaunchTracker.defaultRecordURL : nil)
        guard recording else { return }
        let info = Bundle.main.infoDictionary
        isSafeMode = tracker.beginLaunch(
            runningVersion: info?["CFBundleShortVersionString"] as? String ?? "",
            runningBuild: info?["CFBundleVersion"] as? String ?? "") == .safe
    }

    /// Start only once AppKit has finished launching. A stalled initializer
    /// cannot mark itself healthy just because sixty seconds passed.
    func startHealthWindow() {
        guard recording else { return }
        healthTimer?.invalidate()
        healthStartedAt = clock.now
        let id = UUID()
        healthWindowID = id
        scheduleHealthCheck(id: id, after: LaunchTracker.healthyInterval)
    }

    private func scheduleHealthCheck(id: UUID, after interval: TimeInterval) {
        healthTimer = Timer.scheduledTimer(withTimeInterval: max(interval, 0.01), repeats: false) { [weak self] _ in
            Task { @MainActor in
                guard let self, self.healthWindowID == id else { return }
                let elapsed = self.clock.now.elapsed(since: self.healthStartedAt)
                self.healthTimer = nil
                if self.tracker.markHealthy(elapsed: elapsed) {
                    self.healthWindowID = nil
                } else {
                    // Sleep and a delayed run-loop callback must not lose
                    // the remaining awake-time health check.
                    self.scheduleHealthCheck(id: id, after: LaunchTracker.healthyInterval - elapsed)
                }
            }
        }
    }

    func retryNormal() {
        guard isSafeMode else { return }
        // Persist the attempted normal launch before re-enabling anything.
        tracker.retryNormal()
        isSafeMode = false
        startHealthWindow()
    }

    func cleanShutdown() {
        healthTimer?.invalidate()
        healthTimer = nil
        healthWindowID = nil
        if recording { tracker.cleanShutdown() }
    }

    func copyDiagnosticReport() {
        var snapshot = DiagnosticReport.Snapshot()
        snapshot.appVersion = Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? ""
        snapshot.osMajor = ProcessInfo.processInfo.operatingSystemVersion.majorVersion
        snapshot.failures = ["crash_loop": tracker.consecutiveCrashes]
        #if DEBUG
        snapshot.publicBuild = false
        #endif
        Clipboard.copy(DiagnosticReport.text(snapshot))
    }
}

/// Recovery builds no dashboard, globe, or browser. The two actions work
/// without a wallet key, a responding node, or a completed data migration.
struct SafeModeBanner: View {
    let retryNormal: () -> Void
    @State private var copied = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(String(localized: "EastSea had trouble starting. It is checking for a fix."))
                .font(.aeBody)
                .fixedSize(horizontal: false, vertical: true)
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 12) { actions }
                VStack(alignment: .leading, spacing: 12) { actions }
            }
        }
        .padding(20)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(NSColor.controlBackgroundColor))
    }

    @ViewBuilder private var actions: some View {
        Button(copied ? String(localized: "Copied") : String(localized: "Copy Diagnostic Report")) {
            LaunchRecovery.shared.copyDiagnosticReport()
            copied = true
        }
        Button(String(localized: "Try Normal Mode Again"), action: retryNormal)
    }
}
#endif
