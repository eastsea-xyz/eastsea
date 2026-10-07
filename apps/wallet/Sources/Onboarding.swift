import SwiftUI

/// The terms a user accepts before first use. Bump `version` when the text
/// changes materially, and everyone is asked again.
enum Terms {
    static let isTestnet: Bool = Brand.networkChainId == Brand.legacyTestnetChainId
    // Bump only when users' rights or risks change; a wording fix is not a bump.
    // (scripts/release-identity-gate.sh refuses a release that changes this
    // line without TERMS_BUMP_REASON.)
    static let version = isTestnet ? 5 : 6
    static let disclaimerURL = URL(string: "https://github.com/eastsea-xyz/eastsea/blob/main/DISCLAIMER.md")!
}

/// The chain's voting-set rules, as shown to the user (registry params and
/// MIN_OPEN_COMMITTEE in the node; a committee-signed upgrade can change them).
enum VotingRules {
    /// Epochs (hours) of unbroken liveness before a Mac can be drawn.
    static let minStreakEpochs: UInt64 = 24
    /// Registered Macs needed before the network draws a voting set from them.
    static let minCandidates: UInt32 = 4
    /// Mainnet: one operator gets at most 1/this of each hour's block rewards.
    static let mainnetIssuanceOperators = 16
    /// Said the same way everywhere an early participant looks.
    /// README.md "Planned mainnet rules" quotes this word for word.
    static var mainnetRewardsRule: String {
        let timing = Terms.isTestnet
            ? "Planned for the future mainnet, which is not live: the rules may change before launch, and after it only by a committee-signed upgrade. "
            : "These rules run from mainnet genesis and can change only by a committee-signed upgrade. "
        let testnet = Terms.isTestnet ? "Testnet \(Brand.networkCoinTicker) does not carry over. " : ""
        return timing + "No token sale, no premine and no founder allocation; the founder's Macs follow the same rules as everyone's. Half of each block's reward goes to registered Macs that stay online, shared every hour, and half to registered Macs that prove blocks. One operator gets at most 1/\(mainnetIssuanceOperators) of each half, and the rest is never issued; once \(mainnetIssuanceOperators) operators are online, all of it is shared. The reward starts at 1 \(Brand.networkCoinTicker) a block and shrinks 15% a year, down to a floor of 0.1 \(Brand.networkCoinTicker) a block. " + testnet + "Nothing here promises a price, a return or a way to cash out."
    }
    /// The founder's one exception (docs/design/12-launch-plan.md "창업자 Mac 안전망").
    /// README.md "Planned mainnet rules" quotes this word for word.
    static let founderReserveRule = "The founder's only special permission: one Mac may run up to 3 reserve validator keys, and only while the network needs them. Hours they serve count as the founder's participation, under the same 1/16 cap as everyone; they add no extra share."
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
    @AppStorage("useDevelopmentNetwork") private var useDevelopmentNetwork = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                if useDevelopmentNetwork {
                    Text("Dev network · 127.0.0.1")
                        .font(.caption.bold()).foregroundStyle(.orange)
                }
                Image(systemName: "exclamationmark.shield.fill").font(.system(size: 34)).foregroundStyle(Color.warn)
                Text("Before you use \(Brand.project)").font(.title2.bold())
                Bullet(icon: "hammer", text: Terms.isTestnet
                       ? "\(Brand.project) is built for production. Mainnet has not launched yet; the network running today is the public testnet, and its \(Brand.networkCoinTicker) does not carry over. It is provided as is, without warranty, and has not had an independent security audit yet."
                       : "\(Brand.project) is on mainnet. It is provided as is, without warranty, and has not had an independent security audit yet.")
                Bullet(icon: "chart.line.uptrend.xyaxis", text: "There is no token sale. The value of \(Brand.networkCoinTicker) is set by the market; nothing here promises a price, a return, a listing or a way to cash out.")
                Bullet(icon: "person.fill.checkmark", text: "You use \(Brand.project), and run its node, at your own risk and responsibility, including power and hardware costs, taxes, and following the laws where you live.")
                Bullet(icon: "person.3.fill", text: VotingRules.mainnetRewardsRule)
                Bullet(icon: "network", text: "Running \(Brand.project) shows your IP address to other nodes and the public DHT. Joining as a voting node sends an Apple DeviceCheck token to the registration service, currently run by Pipln, which checks it with Apple. Addresses and transactions are public on chain.")
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
            Bullet(icon: "bolt", text: "Keep \(Brand.project) running. A signing Mac that goes offline hands its seat to the next one. It uses some network, CPU and power.")
            Bullet(icon: "iphone.and.arrow.forward", text: "Registration sends an Apple DeviceCheck token to the registration service, currently run by Pipln, which checks with Apple that this is a real Mac: one Mac, one voting node. Touch ID signs it.")
            Bullet(icon: "person.3.fill", text: VotingRules.mainnetRewardsRule)
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
