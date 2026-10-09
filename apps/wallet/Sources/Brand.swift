import Foundation

enum Brand {
    static let project = "EastSea"
    /// The project's name in the language the app is shown in (its bundle
    /// localization): 동해 inside Korean sentences, EastSea inside English ones.
    /// User-visible text interpolates this, never `project` directly.
    static var name: String { localizedName() }

    static func localizedName(locale: Locale = .current, bundle: Bundle = .main) -> String {
        String(localized: "EastSea", bundle: bundle, locale: locale)
    }
    static let coinName = "Doubloon"
    static let coinTicker = "DBLN"

    /// The legacy 7780 testnet. The native coin has no on-chain symbol, so
    /// its label is display only: DBLN, "Test Doubloon" / 테스트 더블룬 here
    /// (the old AETH label is retired). The look-alike list and the agent
    /// still treat "AETH" as a name of the native coin.
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

    /// The coin's name as listed next to its ticker on `chainId`: "Test
    /// Doubloon" on the legacy testnet, Doubloon on new-genesis chains.
    static func coinName(chainId: UInt64) -> String { chainId == legacyTestnetChainId ? "Test Doubloon" : coinName }

    /// The coin's label on the network this build runs on.
    static var networkCoinTicker: String { coinTicker(chainId: networkChainId) }

    /// The coin's name on the network this build runs on, in the app's
    /// language (더블룬 in Korean).
    static var networkCoinName: String {
        networkChainId == legacyTestnetChainId
            ? String(localized: "Test Doubloon")
            : String(localized: "Doubloon")
    }
}
