import Foundation

/// The coin's label per chain: the legacy 7780 testnet kept its original coin
/// (test AETH); every new-genesis chain — mainnet and later testnets — uses
/// Doubloon/DBLN (docs/design/25-rename.md). "AETH" also stays accepted as a
/// resolver alias for the native coin (Dex.Reader.resolve).
enum Coin {
    static let legacyTestnetChainId: UInt64 = 7780

    static func ticker(_ chainId: UInt64?) -> String { chainId == legacyTestnetChainId ? "AETH" : "DBLN" }
    static func name(_ chainId: UInt64?) -> String { chainId == legacyTestnetChainId ? "Test AETH" : "Doubloon" }
    static func testName(_ chainId: UInt64?) -> String { chainId == legacyTestnetChainId ? "test AETH" : "test DBLN" }
}
