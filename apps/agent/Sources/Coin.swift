import Foundation

/// The coin's label per chain: DBLN everywhere, "Test Doubloon" on the legacy
/// 7780 testnet (the native coin has no on-chain symbol; the old AETH label is
/// retired, docs/design/25-rename.md). "AETH" still answers as a resolver
/// alias for the native coin (Dex.Reader.resolve), so old prompts keep working.
enum Coin {
    static let legacyTestnetChainId: UInt64 = 7780

    static func ticker(_ chainId: UInt64?) -> String { "DBLN" }
    static func name(_ chainId: UInt64?) -> String { chainId == legacyTestnetChainId ? "Test Doubloon" : "Doubloon" }
    static func testName(_ chainId: UInt64?) -> String { "test DBLN" }
}
