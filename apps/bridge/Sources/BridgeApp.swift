import SwiftUI
import Sparkle

/// Aether 0.6.7: the bridge from Aether to EastSea (BridgePlan explains why).
/// It opens straight into the move, shows each step, and afterwards stays a
/// plain "Aether is now EastSea" screen. It runs no node, owns no data, and
/// never deletes anything.
@main
struct BridgeApp: App {
    @NSApplicationDelegateAdaptor(BridgeDelegate.self) private var delegate
    @StateObject private var mover = Mover()

    var body: some Scene {
        WindowGroup("Aether") {
            BridgeView()
                .environmentObject(mover)
                .onAppear { if !mover.finished { mover.run() } }
        }
        .windowResizability(.contentSize)
        .commands {
            CommandGroup(after: .appInfo) {
                Button("Check for Updates…") { delegate.updater.checkForUpdates(nil) }
            }
        }
    }
}

/// Sparkle stays, on the legacy feed: if this bridge ever needs a fix, a
/// 0.6.8 reaches it the same way 0.6.7 reached 0.6.6.
final class BridgeDelegate: NSObject, NSApplicationDelegate, SPUUpdaterDelegate {
    lazy var updater = SPUStandardUpdaterController(startingUpdater: true, updaterDelegate: self, userDriverDelegate: nil)

    func applicationDidFinishLaunching(_ notification: Notification) { _ = updater }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }

    func updater(_ updater: SPUUpdater, willInstallUpdateOnQuit item: SUAppcastItem,
                 immediateInstallationBlock: @escaping () -> Void) -> Bool {
        immediateInstallationBlock()
        return true
    }
}

struct BridgeView: View {
    @EnvironmentObject var mover: Mover
    private let ko = Locale.preferredLanguages.first?.hasPrefix("ko") ?? false

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(ko ? "Aether는 이제 EastSea(동해)입니다" : "Aether is now EastSea")
                .font(.title2.bold())
            Text(mover.finished
                 ? (ko ? "EastSea가 설치되어 열렸습니다. 지갑과 노드 데이터는 EastSea가 처음 열릴 때 확인하며 옮깁니다. 이 Aether 앱은 이제 쓰지 않으니 휴지통으로 옮겨도 됩니다."
                       : "EastSea is installed and open. It moves your wallet and node data over, checking every byte, the first time it opens. You no longer need this Aether app; you can move it to the Trash.")
                 : (ko ? "같은 지갑, 같은 노드, 새 이름입니다. 서명과 Apple 공증을 확인한 EastSea를 설치합니다. 데이터는 지우지 않습니다."
                       : "Same wallet, same node, new name. This installs EastSea after checking its signature and Apple's notarization. No data is deleted."))
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            VStack(alignment: .leading, spacing: 8) {
                ForEach(mover.steps) { step in
                    HStack(spacing: 8) {
                        icon(step.state).frame(width: 18)
                        Text(step.title).foregroundStyle(step.state == .waiting ? .secondary : .primary)
                    }
                }
            }
            if let problem = mover.problem {
                Text(problem).font(.callout).foregroundStyle(.orange)
                    .fixedSize(horizontal: false, vertical: true)
                    .textSelection(.enabled)
            }
            HStack {
                if mover.finished {
                    Button(ko ? "EastSea 열기" : "Open EastSea") { mover.openEastSea() }
                        .keyboardShortcut(.defaultAction)
                } else if mover.problem != nil {
                    Button(ko ? "다시 시도" : "Try Again") { mover.run() }
                        .keyboardShortcut(.defaultAction)
                    Button(ko ? "직접 내려받기" : "Download EastSea Yourself") {
                        NSWorkspace.shared.open(BridgePlan.releasesPage)
                    }
                }
                Spacer()
                Button(ko ? "종료" : "Quit") { NSApp.terminate(nil) }.disabled(mover.busy)
            }
        }
        .padding(24)
        .frame(width: 460)
    }

    @ViewBuilder private func icon(_ state: Mover.StepState) -> some View {
        switch state {
        case .waiting: Image(systemName: "circle").foregroundStyle(.secondary)
        case .working: ProgressView().controlSize(.small)
        case .done: Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
        case .failed: Image(systemName: "xmark.circle.fill").foregroundStyle(.orange)
        }
    }
}
