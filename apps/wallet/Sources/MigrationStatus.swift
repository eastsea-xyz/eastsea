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
    /// The move waits for the Mac to be unlocked (the wallet handle has
    /// complete file protection): the unlock card, no OK button, gone by
    /// itself once the move finishes.
    @Published private(set) var waitingForUnlock = false

    #if DEBUG
    /// Design preview: the overlay mid-move, or after a move that did not finish.
    func loadPreview(moving: Bool, problem: String?) {
        self.moving = moving
        fraction = moving ? 0.42 : 0
        self.problem = problem
    }
    #endif

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
        // The screen unlocking is when a protected wallet handle becomes
        // readable again: retry the move then (never a silent stall).
        DistributedNotificationCenter.default().addObserver(forName: Notification.Name("com.apple.screenIsUnlocked"),
                                                            object: nil, queue: .main) { _ in
            // Unlock is an explicit retry trigger; routine data-dir reads
            // keep waiting outcomes cached until this notification.
            DataMigration.Runner.shared.start()
        }
        runner.onFinish = { [weak self] outcome in
            DispatchQueue.main.async {
                guard let self else { return }
                self.moving = false
                switch outcome {
                case .deferred(let why), .failed(let why):
                    self.problem = why
                    self.waitingForUnlock = false
                case .waitingForUnlock:
                    self.problem = nil
                    self.waitingForUnlock = true
                case .done, .noOldData, .running:
                    self.problem = nil
                    self.waitingForUnlock = false
                }
                Self.log(outcome)
                self.onFinish?(outcome)
            }
        }
    }
}

extension MigrationStatus {
    /// One line per migration outcome: in the node's node-status.log (when
    /// the node folder exists) and in EastSea/migration.log — the next stall
    /// leaves a file to read.
    static func log(_ outcome: DataMigration.Outcome) {
        let text: String
        switch outcome {
        case .noOldData: text = "no old data"
        case .done: text = "done"
        case .deferred(let w): text = "deferred: \(w)"
        case .failed(let w): text = "failed: \(w)"
        case .running: text = "running"
        case .waitingForUnlock: text = "waiting for unlock (the wallet handle is protected while the Mac is locked)"
        }
        let line = NodeStatusLog.line(at: Date(), event: "migration", detail: text, facts: nil)
        let support = DataMigration.supportURL
        NodeStatusLog.append(line, in: support.appending(path: "EastSea/node"))
        NodeStatusLog.append(line, in: support.appending(path: "EastSea"), fileName: "migration.log")
    }
}

/// Over the main window while the data moves, or after a move that could
/// not finish. Plain words, no jargon: what is happening and what to do.
struct MigrationOverlay: View {
    @ObservedObject var status: MigrationStatus

    var body: some View {
        if status.waitingForUnlock {
            card {
                Text(String(localized: "Unlock this Mac to finish moving your wallet")).font(.aeTitle)
                Text(String(localized: "Your node data has already moved to EastSea. Your wallet key file can only be read while this Mac is unlocked, so it has not moved yet. Unlock the Mac and EastSea finishes by itself within a few seconds. Nothing was deleted; your wallet is safe."))
                    .font(.aeFootnote).foregroundStyle(DesignTokens.Palette.textMuted.color)
                    .multilineTextAlignment(.center)
                    .frame(width: 320)
            }
        } else if status.moving {
            card {
                Text("Moving your data from Aether to \(Brand.name)").font(.aeTitle)
                ProgressView(value: status.fraction)
                    .tint(DesignTokens.Palette.accent.color)
                    .frame(width: 280)
                Text("Your wallet and node data are copied and checked, byte by byte. This can take a few minutes. Keep \(Brand.name) open; if it is closed, the move picks up where it left off next time.")
                    .font(.aeFootnote).foregroundStyle(DesignTokens.Palette.textMuted.color)
                    .multilineTextAlignment(.center)
                    .frame(width: 320)
            }
        } else if let problem = status.problem {
            card {
                Text(String(localized: "Your data has not finished moving")).font(.aeTitle)
                Text(problem + (String(localized: " EastSea retries by itself; nothing was deleted.")))
                    .font(.aeFootnote).foregroundStyle(DesignTokens.Palette.textMuted.color)
                    .multilineTextAlignment(.center)
                    .frame(width: 320)
                Button("OK") { status.problem = nil }.buttonStyle(EastSeaPrimaryButtonStyle())
            }
        }
    }

    private func card<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        ZStack {
            Rectangle().fill(DesignTokens.Palette.sea.color.opacity(0.55)).ignoresSafeArea()
            VStack(spacing: DesignTokens.Space.s4) {
                EastSeaDawnMark().frame(width: 40, height: 40)
                content()
            }
            .frame(width: 320)
            .multilineTextAlignment(.center)
            .padding(DesignTokens.Space.s6)
            .foregroundStyle(DesignTokens.Palette.text.color)
            .background(DesignTokens.Palette.surface.color,
                        in: RoundedRectangle(cornerRadius: DesignTokens.Radius.lg))
            .overlay {
                RoundedRectangle(cornerRadius: DesignTokens.Radius.lg)
                    .stroke(DesignTokens.Palette.line.color, lineWidth: 1)
            }
        }
    }
}
#endif
