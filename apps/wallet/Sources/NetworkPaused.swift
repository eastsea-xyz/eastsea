import SwiftUI

/// Said the same way wherever the verification state shows (Home, menu bar).
enum NetworkPausedText {
    static let help = "The network has not made a new block for a while, so there is nothing new to verify. The balance shown is the last one this device verified; a pause by itself does not move funds. It updates by itself when blocks resume."

    /// "Network paused · last block 3 min ago".
    static func line(since: Date, now: Date) -> String {
        let minutes = max(1, Int(now.timeIntervalSince(since) / 60))
        let ago = minutes < 120 ? "\(minutes) min ago" : "\(minutes / 60) h ago"
        return "Network paused · last block \(ago)"
    }
}

/// In place of "Verified" while the chain makes no blocks: neutral, not an error.
struct NetworkPausedBadge: View {
    let since: Date

    var body: some View {
        // Re-read every half minute so "3 min ago" stays true (no continuous animation).
        TimelineView(.periodic(from: .now, by: 30)) { tl in
            Label(NetworkPausedText.line(since: since, now: tl.date), systemImage: "pause.circle.fill")
        }
        .font(.aeCaption.weight(.semibold)).foregroundStyle(.orange)
        .padding(.horizontal, 10).padding(.vertical, 4)
        .background(.orange.opacity(0.12), in: Capsule())
        .help(NetworkPausedText.help)
        .accessibilityHint(NetworkPausedText.help)
    }
}
