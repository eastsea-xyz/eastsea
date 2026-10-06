#if os(macOS)
import SwiftUI

/// The Aether → EastSea data move, as the window shows it (release-070
/// review, M1). The slow path — a copy across volumes, or resuming one, which
/// hashes every byte of an 8.5 GB chain database — runs on a background queue
/// (`DataMigration.Runner`), so the app reaches its window at once and shows
/// this instead of hanging before any window appears (and tempting a
/// force-quit).
///
/// Not main-actor isolated on purpose: it must be wired before the first
/// `DataMigration.ensure()`, which runs during `AppDelegate`'s own property
/// initialisation. Every published change hops to the main queue.
final class MigrationStatus: ObservableObject {
    static let shared = MigrationStatus()

    @Published private(set) var moving = false
    @Published private(set) var fraction: Double = 0
    /// A finished run that did not complete (deferred or failed): its
    /// sentence, until dismissed. The gates keep the node and new keys off.
    @Published var problem: String?

    /// Called on the main queue after every background run.
    var onFinish: ((DataMigration.Outcome) -> Void)?

    private init() {
        let runner = DataMigration.Runner.shared
        runner.onStart = { [weak self] in
            DispatchQueue.main.async {
                self?.fraction = 0
                self?.problem = nil
                self?.moving = true
            }
        }
        runner.onProgress = { [weak self] f in DispatchQueue.main.async { self?.fraction = f } }
        runner.onFinish = { [weak self] outcome in
            DispatchQueue.main.async {
                guard let self else { return }
                self.moving = false
                switch outcome {
                case .deferred(let why), .failed(let why): self.problem = why
                case .done, .noOldData, .running: self.problem = nil
                }
                self.onFinish?(outcome)
            }
        }
    }
}

/// Over the main window while the data moves, or after a move that could
/// not finish. Plain words, no jargon: what is happening and what to do.
struct MigrationOverlay: View {
    @ObservedObject var status: MigrationStatus

    var body: some View {
        if status.moving {
            card {
                Text("Moving your data from Aether to EastSea").font(.headline)
                ProgressView(value: status.fraction)
                    .frame(width: 280)
                Text("Your wallet and node data are copied and checked, byte by byte. This can take a few minutes. "
                     + "Keep EastSea open; if it is closed, the move picks up where it left off next time.")
                    .font(.caption).foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .frame(width: 320)
            }
        } else if let problem = status.problem {
            card {
                Text("Your data has not moved yet").font(.headline)
                Text(problem)
                    .font(.caption).foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .frame(width: 320)
                Button("OK") { status.problem = nil }
            }
        }
    }

    private func card<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        ZStack {
            Rectangle().fill(.black.opacity(0.25)).ignoresSafeArea()
            VStack(spacing: 12, content: content)
                .padding(24)
                .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 14))
        }
    }
}
#endif
