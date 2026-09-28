import SwiftUI

#if DEBUG
/// `-designPreview [rewards|verifying|empty|paused]` (debug builds only): sample data
/// for screenshots. No keys are made, nothing is read from the network and no node
/// is started. On the Mac, `-previewWidth 420` sizes the window.
enum DesignPreview {
    static var on: Bool { ProcessInfo.processInfo.arguments.contains("-designPreview") }
    static var variant: String { UserDefaults.standard.string(forKey: "designPreview") ?? "rewards" }
}

extension WalletModel {
    func loadPreview() {
        let v = DesignPreview.variant, empty = v == "empty"
        address = "0x5397a1c0De4b1b8F6A3cB2d1E0f9C7a6B5d4E502"
        let now = Date()
        status = ChainStatus(chainId: 7780, height: 184_210, stateRoot: "0x", mempool: 0, transferFeeWei: "21000000000000")
        account = VerifiedAccount(address: address, balanceWei: empty ? "0" : "12500000000000000000", nonce: 3,
                                  stateHeight: 184_209, certifiedBlock: 184_210, stateRoot: "0x", validators: 4)
        if v == "paused" { chainPausedSince = now.addingTimeInterval(-240) }
        guard !empty else {
            tokensUpdated = now
            return
        }
        history = [BalancePoint(date: now.addingTimeInterval(-80_000), aeth: 0), BalancePoint(date: now.addingTimeInterval(-60_000), aeth: 10),
                   BalancePoint(date: now.addingTimeInterval(-30_000), aeth: 8), BalancePoint(date: now.addingTimeInterval(-3_000), aeth: 12.5)]
        activity = [
            ActivityItem(date: now.addingTimeInterval(-300), kind: .received, title: "Proof reward · block #184024", amount: 0.5, state: .done),
            ActivityItem(date: now.addingTimeInterval(-3_600), kind: .sent, title: "Sent to 0x12ab…90ab", amount: -2, state: .done),
            ActivityItem(date: now.addingTimeInterval(-60_000), kind: .received, title: "Test AETH from faucet", amount: 10, state: .done),
        ]
        tokens = [
            TokenHolding(token: TokenInfo(address: "0x6bc5ded76ccbdc8df35e7cd28b68fed245a74416", symbol: "NEB", name: "Nebula", decimals: 18), balance: "250000000000000000000"),
            TokenHolding(token: TokenInfo(address: "0x961f8add5ae93ff0700be8abd5f9f8ec69ba4347", symbol: "ORB", name: "Orb", decimals: 18), balance: "1500000000000000000"),
        ]
        tokensUpdated = now
    }
}
#endif
