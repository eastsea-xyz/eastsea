import SwiftUI

#if DEBUG
/// `-designPreview [rewards|verifying|empty|paused]` (debug builds only): sample data
/// for screenshots. No keys are made, nothing is read from the network and no node
/// is started. On the Mac, `-previewWidth 420` sizes the window.
enum DesignPreview {
    static var on: Bool { ProcessInfo.processInfo.arguments.contains("-designPreview") }
    static var variant: String { UserDefaults.standard.string(forKey: "designPreview") ?? "rewards" }
    /// `-rewardStatus 1`: the Network page's node-rewards standing card (the
    /// testnet would answer enabled:false and show nothing).
    static var rewardStatus: Bool { UserDefaults.standard.string(forKey: "rewardStatus") == "1" }
    /// `-historyNotice 1`: the history page's "this node cannot show your full
    /// history" notice, as an old node would leave it.
    static var historyNotice: Bool { UserDefaults.standard.string(forKey: "historyNotice") == "1" }

    /// One `aether_rewardStatus [operator]` answer: 9 operators, this Mac at
    /// warm-up level 4 ((14+4)/28 → 64%), a capped share of the last hour.
    static let sampleRewardStatus: [String: Any] = [
        "enabled": true,
        "height": 184_210,
        "epoch": 7_675,
        "epoch_blocks": 24,
        "operators_online_last_epoch": 9,
        "max_share": 16,
        "node_pool_last_epoch": "1600000000000000000",
        "issuance_per_block_now": "1000000000000000000",
        "operator": [
            "address": "0x5397a1c0De4b1b8F6A3cB2d1E0f9C7a6B5d4E502",
            "macs": [[
                "index": 1,
                "answered_slots_this_epoch": 48,
                "answered_slots_last_epoch": 44,
                "warmup_level": 4,
                "warmup_percent": 64,
                "attested_period": 7_676,
                "reattest_ok": true,
            ]],
            "weight_last_epoch": 792,
            "expected_share_last_epoch": "140000000000000000",
            "received_last_distribution": "140000000000000000",
            "capped": true,
        ],
    ]
}

extension WalletModel {
    func loadPreview() {
        let v = DesignPreview.variant, empty = v == "empty"
        address = "0x5397a1c0De4b1b8F6A3cB2d1E0f9C7a6B5d4E502"
        let now = Date()
        status = ChainStatus(chainId: 7780, height: 184_210, stateRoot: "0x", mempool: 0, transferFeeWei: "21000000000000", upgradesJson: "[]", supportedProtocol: 3, faucet: nil)
        account = VerifiedAccount(address: address, balanceWei: empty ? "0" : "12500000000000000000", nonce: 3,
                                  stateHeight: 184_209, certifiedBlock: 184_210, stateRoot: "0x", validators: 4)
        if v == "paused" { chainPausedSince = now.addingTimeInterval(-240) }
        if DesignPreview.historyNotice {
            historyFailure = .unsupportedNode
        }
        guard !empty else {
            tokensUpdated = now
            return
        }
        history = [BalancePoint(date: now.addingTimeInterval(-80_000), aeth: 0), BalancePoint(date: now.addingTimeInterval(-60_000), aeth: 10),
                   BalancePoint(date: now.addingTimeInterval(-30_000), aeth: 8), BalancePoint(date: now.addingTimeInterval(-3_000), aeth: 12.5)]
        activity = [
            ActivityItem(date: now.addingTimeInterval(-300), kind: .received, title: "Proof reward · block #184024", amount: 0.5, state: .done),
            ActivityItem(date: now.addingTimeInterval(-3_600), kind: .sent, title: "Sent to 0x12ab…90ab", amount: -2, state: .done),
            ActivityItem(date: now.addingTimeInterval(-60_000), kind: .received, title: "Test \(Brand.coinTicker) from faucet", amount: 10, state: .done),
            ActivityItem(date: now.addingTimeInterval(-9_000), kind: .sent, title: "Sent 5 USDX · 0x0000…00c1 to 0x12ab…90ab", amount: nil, state: .done,
                         token: "0x00000000000000000000000000000000000000c1"),
        ]
        tokens = [
            TokenHolding(token: TokenInfo(address: "0x6bc5ded76ccbdc8df35e7cd28b68fed245a74416", symbol: "NEB", name: "Nebula", decimals: 18), balance: "250000000000000000000"),
            TokenHolding(token: TokenInfo(address: "0x961f8add5ae93ff0700be8abd5f9f8ec69ba4347", symbol: "ORB", name: "Orb", decimals: 18), balance: "1500000000000000000"),
            // Off-list on purpose: the generated dashed glyph next to official art.
            TokenHolding(token: TokenInfo(address: "0x00000000000000000000000000000000000000c1", symbol: "USDX", name: "Test Dollar", decimals: 6, origin: "dex"), balance: "5000000"),
            TokenHolding(token: TokenInfo(address: "0x00000000000000000000000000000000000000d4", symbol: "VVDBLN", name: "Doubloon Cash", decimals: 18, origin: "launchpad"), balance: "900000000000000000000"),
        ]
        tokensUpdated = now
    }
}
#endif
