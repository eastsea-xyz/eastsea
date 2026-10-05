import Foundation

enum Brand {
    static let project = "EastSea"
    static let projectKo = "동해"
    static let coinName = "Doubloon"
    static let coinTicker = "DBLN"

    /// The legacy 7780 testnet, whose coin kept its original label (test
    /// AETH); every new-genesis chain — mainnet and later testnets — uses
    /// Doubloon/DBLN (docs/design/25-rename.md).
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

    /// The coin's ticker on `chainId` (a chain that is not the legacy
    /// testnet is treated as new-genesis).
    static func coinTicker(chainId: UInt64) -> String {
        chainId == legacyTestnetChainId ? "AETH" : coinTicker
    }

    /// The coin's name as listed next to its ticker on `chainId`.
    static func coinName(chainId: UInt64) -> String {
        chainId == legacyTestnetChainId ? "Test AETH" : coinName
    }

    /// The coin's label on the network this build runs on.
    static var networkCoinTicker: String { coinTicker(chainId: networkChainId) }

    /// The coin's name on the network this build runs on.
    static var networkCoinName: String { coinName(chainId: networkChainId) }
}
