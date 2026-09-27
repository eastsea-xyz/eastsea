import SwiftUI

/// The terms a user accepts before first use. Bump `version` when the text
/// changes materially, and everyone is asked again.
enum Terms {
    static let version = 1
    static let disclaimerURL = URL(string: "https://github.com/kjaylee/aether-node/blob/main/DISCLAIMER.md")!
}

/// The chain's voting-set rules, as shown to the user (registry params and
/// MIN_OPEN_COMMITTEE in the node; a committee-signed upgrade can change them).
enum VotingRules {
    /// Epochs (hours) of unbroken liveness before a Mac can be drawn.
    static let minStreakEpochs: UInt64 = 24
    /// Registered Macs needed before the network draws a voting set from them.
    static let minCandidates: UInt32 = 4
}

private struct Bullet: View {
    let icon: String
    let text: String

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: icon).foregroundStyle(Color.aether).frame(width: 20)
            Text(text).fixedSize(horizontal: false, vertical: true)
        }
        .font(.callout)
    }
}

/// First launch: what Aether is, and that using it is the user's own risk.
struct TermsSheet: View {
    let accept: () -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Image(systemName: "exclamationmark.shield.fill").font(.system(size: 34)).foregroundStyle(.orange)
                Text("Before you use Aether").font(.title2.bold())
                Bullet(icon: "flask", text: "Aether is experimental research software running on a test network. It is provided as is, without any warranty, and has not been audited.")
                Bullet(icon: "drop", text: "AETH on this testnet comes free from the faucet and does not carry over to any future network. Nothing here promises a price, a return or a way to cash out.")
                Bullet(icon: "person.fill.checkmark", text: "You use Aether, and run its node, at your own risk and responsibility, including following the laws where you live.")
                Bullet(icon: "key.fill", text: "Your key stays on this device. If you lose the device and have not set up a recovery key, nobody can restore the account.")
                Link("Read the full terms and disclaimer", destination: Terms.disclaimerURL).font(.callout)
                HStack {
                    #if os(macOS)
                    Button("Quit") { NSApp.terminate(nil) }
                    #endif
                    Spacer()
                    Button("I understand and agree", action: accept).buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction)
                }
                .padding(.top, 4)
            }
            .padding(24)
        }
        .macMinSize(width: 440, height: 460)
        .interactiveDismissDisabled()
    }
}

#if os(macOS)
/// Once the node has caught up: invite this Mac to become a voting node.
struct VotingNodeInvite: View {
    let join: () -> Void
    let later: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Image(systemName: "person.badge.plus").font(.system(size: 30)).foregroundStyle(Color.aether)
            Text("Join the network as a voting node?").font(.title3.bold())
            Bullet(icon: "clock", text: "Your Mac proves it is online every hour. After \(VotingRules.minStreakEpochs) hours in a row it can be drawn to sign blocks.")
            Bullet(icon: "bolt", text: "Keep Aether running. A signing Mac that goes offline hands its seat to the next one. It uses some network, CPU and power.")
            Bullet(icon: "iphone.and.arrow.forward", text: "Registration uses Apple DeviceCheck: one Mac, one voting node. Touch ID signs it.")
            Bullet(icon: "person.fill.checkmark", text: "Running a voting node is your choice and your responsibility. You can turn the node off anytime.")
            HStack {
                Button("Not now", action: later)
                Spacer()
                Button(action: join) { Label("Join", systemImage: "touchid") }.buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction)
            }
            .padding(.top, 4)
        }
        .padding(24)
        .frame(width: 440)
    }
}
#endif
