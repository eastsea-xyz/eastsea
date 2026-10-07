import Foundation

enum Brand {
    static let project = "EastSea"
    static let projectKo = "동해"
    /// The project's name in the language the app is shown in (its bundle
    /// localization): 동해 inside Korean sentences, EastSea inside English ones.
    /// User-visible text interpolates this, never `project` directly.
    static var name: String {
        (Bundle.main.preferredLocalizations.first?.hasPrefix("ko") ?? false) ? projectKo : project
    }
    static let coinName = "Doubloon"
    static let coinTicker = "DBLN"

    /// The legacy 7780 testnet. Its coin was labeled AETH until the 0.7.1
    /// wallet: the app now shows Doubloon/DBLN on every chain, so no screen
    /// carries the old brand (founder review of 0.7.0, 2026-10-07). The
    /// agent CLI and the token list still accept "AETH" as an alias.
    static let legacyTestnetChainId: UInt64 = 7780

    /// The chain this app build runs on, read from the bundled network.json.
    /// No file (a bare test build) means the legacy testnet, matching
    /// `Terms.isTestnet`'s conservative default.
    static let networkChainId: UInt64 = {
        guard let url = Bundle.main.url(forResource: "network", withExtension: "json"),
              let data = try? Data(contentsOf: url),
              let network = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let id = (network["chain_id"] as? NSNumber)?.uint64Value, id > 0 else { return legacyTestnetChainId }
        return id
    }()

    /// The coin's ticker as the wallet shows it on `chainId`: DBLN everywhere.
    static func coinTicker(chainId: UInt64) -> String { coinTicker }

    /// The coin's name as listed next to its ticker on `chainId`.
    static func coinName(chainId: UInt64) -> String { coinName }

    /// The coin's label on the network this build runs on.
    static var networkCoinTicker: String { coinTicker(chainId: networkChainId) }

    /// The coin's name on the network this build runs on, in the app's
    /// language (더블룬 in Korean).
    static var networkCoinName: String {
        (Bundle.main.preferredLocalizations.first?.hasPrefix("ko") ?? false) ? coinNameKo : coinName(chainId: networkChainId)
    }
    static let coinNameKo = "더블룬"
}
