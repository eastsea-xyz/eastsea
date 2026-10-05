import Foundation

/// The `window.eastsea.verify` surface (Resources/provider.js): a page inside
/// the app asks the wallet to check a block, an account or a transaction
/// receipt with the native Rust verifier — ≈0.68 ms a committee certificate,
/// where the browser wasm module a public page would load takes ≈11.7 ms
/// (docs/research/wasm-speed-2026-10-05.md). Pure logic on purpose: no WebKit
/// types, so the origin rule and the ask parsing run in tests without a view
/// (BrowserController owns the WebKit half and the FFI calls).
enum VerifyBridge {
    /// What a page asked for: `{what: "block" | "account" | "receipt", param}`.
    enum Ask: Equatable {
        case block(height: UInt64)
        case account(address: String)
        case receipt(txHash: String)
    }

    /// One answer to a page: `{verified, height?, reason}`. A check that ran
    /// and failed is a *successful* answer with `verified: false` and the
    /// reason — only a malformed or unauthorized ask is a bridge error.
    struct Verdict: Equatable {
        let verified: Bool
        let height: UInt64?
        let reason: String

        static func certified(height: UInt64) -> Verdict {
            Verdict(verified: true, height: height, reason: "")
        }

        static func refused(_ reason: String) -> Verdict {
            Verdict(verified: false, height: nil, reason: reason)
        }

        /// The one answer receipts get today: no block commits to a receipt
        /// yet (crates/light block.rs Payload has no receipts root), so there
        /// is nothing a certificate could vouch for. When a commitment lands,
        /// route this through an FFI check like the other two asks.
        static let notCommitted = Verdict.refused("not committed")

        /// The JSON the bridge resolves the page's promise with.
        func asDictionary() -> [String: Any] {
            var out: [String: Any] = ["verified": verified]
            if let height { out["height"] = height }
            out["reason"] = reason
            return out
        }
    }

    /// Parse a verify message body. `what` names the check, `param` is its
    /// one argument; anything else is a malformed ask (EIP-1193's -32602).
    static func parse(_ body: [String: Any]) -> Result<Ask, ProviderError> {
        let bad = ProviderError(code: ProviderErrorCode.params, message: "expected {what, param}")
        guard let what = body["what"] as? String else { return .failure(bad) }
        switch what {
        case "block":
            guard let n = body["param"] as? NSNumber, n !== kCFBooleanTrue, n !== kCFBooleanFalse,
                  let height = UInt64(exactly: n) else {
                return .failure(ProviderError(code: ProviderErrorCode.params, message: "block height must be a non-negative integer"))
            }
            return .success(.block(height: height))
        case "account":
            guard let address = body["param"] as? String, isAddress(address) else {
                return .failure(ProviderError(code: ProviderErrorCode.params, message: "account param is not an address"))
            }
            return .success(.account(address: address))
        case "receipt":
            guard let hash = body["param"] as? String,
                  hash.count == 66, hash.hasPrefix("0x"),
                  hash.dropFirst(2).allSatisfy({ $0.isHexDigit }) else {
                return .failure(ProviderError(code: ProviderErrorCode.params, message: "receipt param is not a tx hash"))
            }
            return .success(.receipt(txHash: hash))
        default:
            return .failure(ProviderError(code: ProviderErrorCode.unsupported,
                                          message: "EastSea Wallet does not verify \(what)."))
        }
    }

    /// Which pages may ask at all: the pages the app bundles (the explorer and
    /// any registry app under eastsea-page://) always, and an external page
    /// only over https and only while its origin is connected to an account —
    /// a page that never went through a connect sheet gets no verifier, even
    /// though the provider script is injected into every page the tab loads.
    static func allows(scheme: String, connected: Bool) -> Bool {
        let s = scheme.lowercased()
        if s == BrowserOriginPolicy.bundledScheme { return true }
        return s == "https" && connected
    }

    /// /^0x[0-9a-fA-F]{40}$/ (BrowserPolicy's address rule, for this file's
    /// own parse path).
    private static func isAddress(_ s: String) -> Bool {
        s.count == 42 && s.hasPrefix("0x") && s.dropFirst(2).allSatisfy(\.isHexDigit)
    }
}
