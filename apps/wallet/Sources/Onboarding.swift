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

    // The legal body has only reviewed English and Korean versions.
    static var legalLanguage: String { ["ko": "ko"][AppLanguage.identifier] ?? "en" }
    static var legalBundle: Bundle { AppLanguage.bundle(for: legalLanguage) }
    static var legalLocale: Locale { Locale(identifier: legalLanguage) }
    static var referenceNotice: String? {
        let notice = String(localized: "This translation is for reference; the English text governs.")
        return ["ja": notice, "zh-Hans": notice, "zh-Hant": notice][AppLanguage.identifier]
    }

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
    static var mainnetRewardsRule: String { mainnetRewardsRule() }

    static func mainnetRewardsRule(locale: Locale = .current, bundle: Bundle = .main) -> String {
        let timing = Terms.isTestnet
            ? String(localized: "Planned for the future mainnet, which is not live: the rules may change before launch, and after it only by a committee-signed upgrade.", bundle: bundle, locale: locale)
            : String(localized: "These rules run from mainnet genesis and can change only by a committee-signed upgrade.", bundle: bundle, locale: locale)
        let testnet = Terms.isTestnet ? String(localized: "Testnet \(Brand.networkCoinTicker) does not carry over.", bundle: bundle, locale: locale) + " " : ""
        return timing + " " + String(localized: "No token sale, no premine and no founder allocation; the founder's Macs follow the same rules as everyone's. Half of each block's reward goes to registered Macs that stay online, shared every hour, and half to registered Macs that prove blocks. One operator gets at most 1/\(mainnetIssuanceOperators) of each half, and the rest is never issued; once \(mainnetIssuanceOperators) operators are online, all of it is shared. The reward starts at 1 \(Brand.networkCoinTicker) a block and shrinks 15% a year, down to a floor of 0.1 \(Brand.networkCoinTicker) a block.", bundle: bundle, locale: locale) + " " + testnet + String(localized: "Nothing here promises a price, a return or a way to cash out.", bundle: bundle, locale: locale)
    }
    /// The founder's one exception (docs/design/12-launch-plan.md "창업자 Mac 안전망").
    /// README.md "Planned mainnet rules" quotes this word for word.
    static var founderReserveRule: String { String(localized: "The founder's only special permission: one Mac may run up to 3 reserve validator keys, and only while the network needs them. Hours they serve count as the founder's participation, under the same 1/16 cap as everyone; they add no extra share.") }
}

private struct Bullet: View {
    let icon: String
    let text: String

    var body: some View {
        HStack(alignment: .top, spacing: DesignTokens.Space.s3) {
            Image(systemName: icon).foregroundStyle(Color.aether).frame(width: 20)
            Text(text).fixedSize(horizontal: false, vertical: true).lineSpacing(DesignTokens.Space.s1)
        }
        .font(.aeBody)
    }
}

/// First launch: what EastSea is, and that using it is the user's own risk.
struct TermsSheet: View {
    let accept: () -> Void
    @AppStorage("useDevelopmentNetwork") private var useDevelopmentNetwork = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: DesignTokens.Space.s4) {
                if useDevelopmentNetwork {
                    Text("Dev network · 127.0.0.1")
                        .font(.aeCaption.weight(.semibold)).foregroundStyle(Color.warn)
                }
                HStack(alignment: .top, spacing: DesignTokens.Space.s3) {
                    EastSeaDawnMark().frame(width: 40, height: 40)
                    Text("Before you use \(Brand.name)").font(DesignTokens.TypeScale.title2.font)
                }
                if let notice = Terms.referenceNotice {
                    Text(notice).font(.aeCaption).foregroundStyle(DesignTokens.Palette.textMuted.color)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(DesignTokens.Space.s3)
                        .background(DesignTokens.Palette.surfaceSunken.color,
                                    in: RoundedRectangle(cornerRadius: DesignTokens.Radius.md))
                }
                Bullet(icon: "hammer", text: Terms.isTestnet
                       ? String(localized: "\(Brand.name) is built for production. Mainnet has not launched yet; the network running today is the public testnet, and its \(Brand.networkCoinTicker) does not carry over. It is provided as is, without warranty, and has not had an independent security audit yet.", bundle: Terms.legalBundle, locale: Terms.legalLocale)
                       : String(localized: "\(Brand.name) is on mainnet. It is provided as is, without warranty, and has not had an independent security audit yet.", bundle: Terms.legalBundle, locale: Terms.legalLocale))
                Bullet(icon: "chart.line.uptrend.xyaxis", text: String(localized: "There is no token sale. The value of \(Brand.networkCoinTicker) is set by the market; nothing here promises a price, a return, a listing or a way to cash out.", bundle: Terms.legalBundle, locale: Terms.legalLocale))
                Bullet(icon: "person.fill.checkmark", text: String(localized: "You use \(Brand.name), and run its node, at your own risk and responsibility, including power and hardware costs, taxes, and following the laws where you live.", bundle: Terms.legalBundle, locale: Terms.legalLocale))
                Bullet(icon: "person.3.fill", text: VotingRules.mainnetRewardsRule(locale: Terms.legalLocale, bundle: Terms.legalBundle))
                Bullet(icon: "network", text: String(localized: "Running \(Brand.name) shows your IP address to other nodes and the public DHT. Joining as a voting node sends an Apple DeviceCheck token to the registration service, currently run by Pipln, which checks it with Apple. Addresses and transactions are public on chain.", bundle: Terms.legalBundle, locale: Terms.legalLocale))
                Bullet(icon: "key.fill", text: String(localized: "Your key stays on this device. If you lose the device and have not set up a recovery key, nobody can restore the account.", bundle: Terms.legalBundle, locale: Terms.legalLocale))
                Link("Read the full terms and disclaimer", destination: Terms.disclaimerURL).font(.aeFootnote)
                HStack {
                    #if os(macOS)
                    Button("Quit") { NSApp.terminate(nil) }.buttonStyle(EastSeaQuietButtonStyle())
                    #endif
                    Spacer()
                    Button("I understand and agree", action: accept).buttonStyle(EastSeaPrimaryButtonStyle()).keyboardShortcut(.defaultAction)
                }
                .padding(.top, DesignTokens.Space.s1)
            }
            .padding(DesignTokens.Space.s6)
        }
        .macMinSize(width: 440, height: 460)
        .eastSeaSheet()
        .interactiveDismissDisabled()
    }
}

#if os(macOS)
/// Once the node has caught up: invite this Mac to become a voting node.
struct VotingNodeInvite: View {
    let join: () -> Void
    let later: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: DesignTokens.Space.s4) {
            HStack(alignment: .top, spacing: DesignTokens.Space.s3) {
                EastSeaDawnMark().frame(width: 40, height: 40)
                Text("Join the network as a voting node?").font(.aeTitle)
            }
            Bullet(icon: "clock", text: String(localized: "Your Mac proves it is online every hour. After \(VotingRules.minStreakEpochs) hours in a row it can be drawn to sign blocks."))
            Bullet(icon: "bolt", text: String(localized: "Keep \(Brand.name) running. A signing Mac that goes offline hands its seat to the next one. It uses some network, CPU and power."))
            Bullet(icon: "iphone.and.arrow.forward", text: String(localized: "Registration sends an Apple DeviceCheck token to the registration service, currently run by Pipln, which checks with Apple that this is a real Mac: one Mac, one voting node. Touch ID signs it."))
            Bullet(icon: "person.3.fill", text: VotingRules.mainnetRewardsRule)
            Bullet(icon: "person.fill.checkmark", text: String(localized: "Running a voting node is your choice and your responsibility. You can turn the node off anytime."))
            HStack {
                Button("Not now", action: later).buttonStyle(EastSeaQuietButtonStyle())
                Spacer()
                Button(action: join) { Label("Join", systemImage: "touchid") }.buttonStyle(EastSeaPrimaryButtonStyle()).keyboardShortcut(.defaultAction)
            }
            .padding(.top, DesignTokens.Space.s1)
        }
        .padding(DesignTokens.Space.s6)
        .frame(width: 440)
        .eastSeaSheet()
    }
}
#endif
