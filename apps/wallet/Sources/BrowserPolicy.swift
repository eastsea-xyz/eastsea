import Foundation

/// The Explore tab's provider rules (docs/design/09-wallet.md "인앱 브라우저"):
/// which methods a page may call, how its `eth_sendTransaction` is checked,
/// and what the confirmation sheet says a call does. Everything here mirrors
/// apps/extension/src/lib/methods.js check for check, so the extension and
/// the in-app browser refuse the same junk — apps/extension/test/
/// wallet-provider.test.mjs asserts the method set stays equal.
enum ProviderErrorCode {
    /// The wallet is locked (or has no key): nothing is answered, not even reads.
    static let locked = 4100
    /// A method neither the extension nor this wallet supports.
    static let unsupported = 4200
    /// The user rejected the request in the sheet.
    static let denied = 4001
    /// A malformed request (not an object, bad address, bad hex…).
    static let params = -32602
    /// Too many unanswered requests from one origin.
    static let timeout = -32002
    /// The bridge itself failed.
    static let internalError = -32603
    /// `method` was not a string (EIP-1193 leaves this to the wallet).
    static let notAString = -32600
}

/// A JSON-RPC-style refusal the bridge resolves the page's promise with
/// (the code survives to the page, as in the extension).
struct ProviderError: Error, Equatable {
    let code: Int
    let message: String
}

/// The methods the injected provider answers — READ ∪ ACCOUNT ∪ SEND from
/// methods.js, plus the three background.js answers by hand (eth_chainId,
/// wallet_disconnect, aether_disconnect).
enum ProviderMethod {
    static let read: Set<String> = [
        "eth_blockNumber", "eth_call", "eth_estimateGas", "eth_getBalance", "eth_getCode",
        "eth_getLogs", "eth_getStorageAt", "eth_getTransactionCount", "eth_gasPrice",
        "net_version", "aether_status", "aether_getReceipt", "aether_getAccount", "aether_accountHistory",
    ]
    static let account: Set<String> = ["eth_requestAccounts", "aether_requestAccounts", "eth_accounts", "aether_accounts"]
    static let send: Set<String> = ["eth_sendTransaction", "aether_sendTransaction"]
    static let answered: Set<String> = [
        "eth_chainId", "wallet_disconnect", "aether_disconnect",
    ]
    static let supported: Set<String> = read.union(account).union(send).union(answered)

    /// The extension's gas cap (methods.js MAX_GAS).
    static let maxGas: UInt64 = 10_000_000
}

/// A page's `eth_sendTransaction`, normalized exactly the way the extension's
/// normalizeTx normalizes it: same accepted forms for value and gas (hex
/// quantities, decimal strings, safe integers), same address/hex checks, same
/// creation rule, same gas cap. This is the value the confirmation sheet
/// shows and the builder receives — one parse, never re-read.
struct PageTransaction: Equatable {
    let to: String
    /// Wei, decimal string (values can exceed UInt64, so never an integer here).
    let valueWei: String
    /// Lowercased 0x-prefixed bytes ('0x' for a plain transfer).
    let data: String
    /// Gas limit the page asked for (0: the wallet's default).
    let gas: UInt64

    /// A plain native-coin transfer: what the send sheet's FeeChanged flow covers.
    var isPlainTransfer: Bool { data == "0x" && !to.isEmpty }

    /// normalizeTx, mirrored. `from` must be the address this page is
    /// connected to (extension: a SEND needs the connection, and the account
    /// must be its own).
    static func parse(_ raw: Any?, from connectedAddress: String?) -> Result<PageTransaction, ProviderError> {
        guard let tx = raw as? [String: Any] else {
            return .failure(ProviderError(code: ProviderErrorCode.params, message: "expected a transaction object"))
        }
        if let claimed = tx["from"] as? String, let connected = connectedAddress,
           claimed.lowercased() != connected.lowercased() {
            return .failure(ProviderError(code: ProviderErrorCode.locked,
                                          message: "the request names a different account than the one this site may use"))
        }
        // /^0x[0-9a-fA-F]{40}$/ (methods.js ADDRESS), inlined to keep this
        // file free of UI-side dependencies.
        func isAddress(_ s: String) -> Bool {
            s.count == 42 && s.hasPrefix("0x") && s.dropFirst(2).allSatisfy(\.isHexDigit)
        }
        let bad = ProviderError(code: ProviderErrorCode.params, message: "`to` is not an address")
        let to: String
        if let v = tx["to"], !(v is NSNull) {
            guard let s = v as? String else { return .failure(bad) }
            to = s
            guard to.isEmpty || isAddress(to) else { return .failure(bad) }
        } else {
            to = ""
        }
        let data: String
        if let v = tx["data"] ?? tx["input"] {
            guard let s = v as? String, s.hasPrefix("0x"),
                  s.dropFirst(2).count % 2 == 0, s.dropFirst(2).allSatisfy({ $0.isHexDigit }) else {
                return .failure(ProviderError(code: ProviderErrorCode.params, message: "`data` must be 0x-prefixed hex bytes"))
            }
            data = s
        } else {
            data = "0x"
        }
        if to.isEmpty && data == "0x" {
            return .failure(ProviderError(code: ProviderErrorCode.params, message: "a contract creation needs init code in `data`"))
        }
        // value == null → 0n, gas == null → 0 (methods.js normalizeTx).
        let valueWei: String
        if let v = tx["value"], !(v is NSNull) {
            switch quantity(v, what: "`value`") {
            case .success(let decimal): valueWei = decimal
            case .failure(let e): return .failure(e)
            }
        } else {
            valueWei = "0"
        }
        let gas: UInt64
        if let v = tx["gas"] ?? tx["gasLimit"], !(v is NSNull) {
            switch quantity(v, what: "`gas`") {
            case .success(let decimal):
                guard let g = UInt64(decimal), g <= ProviderMethod.maxGas else {
                    return .failure(ProviderError(code: ProviderErrorCode.params,
                                                   message: "gas above the wallet cap (\(ProviderMethod.maxGas))"))
                }
                gas = g
            case .failure(let e): return .failure(e)
            }
        } else {
            gas = 0
        }
        return .success(PageTransaction(to: to, valueWei: valueWei, data: data.lowercased(), gas: gas))
    }

    /// A hex quantity, a decimal string, or a safe non-negative integer —
    /// returned as an exact decimal string (value can exceed UInt64).
    static func quantity(_ v: Any?, what: String) -> Result<String, ProviderError> {
        let bad = ProviderError(code: ProviderErrorCode.params, message: "\(what) must be a hex quantity")
        if let s = v as? String {
            if s.hasPrefix("0x"), s.dropFirst(2).count >= 1, s.dropFirst(2).allSatisfy({ $0.isHexDigit }) {
                return .success(Self.decimal(fromHex: s))
            }
            if !s.isEmpty, s.allSatisfy({ $0.isNumber && $0.isASCII }) { return .success(s) }
            return .failure(bad)
        }
        if let n = v as? NSNumber, n !== kCFBooleanTrue, n !== kCFBooleanFalse,
           let i = Int64(exactly: n), i >= 0 {
            return .success(String(i))
        }
        return .failure(bad)
    }

    /// "0x…" → exact decimal string, for values past UInt64.
    static func decimal(fromHex hex: String) -> String {
        let body = hex.hasPrefix("0x") ? String(hex.dropFirst(2)) : hex
        guard !body.isEmpty else { return "0" }
        var digits: [UInt8] = [0]   // little-endian base 10
        for ch in body.lowercased() {
            guard let v = UInt32(String(ch), radix: 16) else { return "0" }
            var carry = v
            for i in digits.indices {
                let cur = UInt32(digits[i]) * 16 + carry
                digits[i] = UInt8(cur % 10)
                carry = cur / 10
            }
            while carry > 0 { digits.append(UInt8(carry % 10)); carry /= 10 }
        }
        let out = digits.reversed().map(String.init).joined()
        let trimmed = out.drop(while: { $0 == "0" })
        return trimmed.isEmpty ? "0" : String(trimmed)
    }

    /// Exact decimal string → "0x…" (hex quantity), the way back for the
    /// balances the FFI verifies (wei can exceed UInt64, so never an integer).
    static func hex(fromDecimal decimal: String) -> String {
        var digits = decimal.compactMap { $0.wholeNumberValue.map(UInt8.init) }
        guard digits.count == decimal.count, !digits.isEmpty else { return "0x0" }
        var out: [Character] = []
        while !(digits.count == 1 && digits[0] == 0) {
            var carry = 0
            for i in digits.indices {
                let cur = Int(digits[i]) + carry * 10
                digits[i] = UInt8(cur / 16)
                carry = cur % 16
            }
            out.append(["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "a", "b", "c", "d", "e", "f"][carry])
            while digits.first == 0 && digits.count > 1 { digits.removeFirst() }
        }
        let body = String(out.reversed())
        let trimmed = body.drop(while: { $0 == "0" })
        return "0x" + (trimmed.isEmpty ? "0" : String(trimmed))
    }
}

/// Which wallet path answers a page's call — background.js's routing,
/// mirrored. Reads split into the verified FFI paths and the local-node
/// fallback the Explore tab marks "unverified".
enum ProviderRouter {
    enum Route: Equatable {
        /// chain id, answered from the FFI's configured network.
        case chainId
        /// eth_accounts / aether_accounts: the address this origin is connected to.
        case accounts
        /// eth_requestAccounts: always a native sheet.
        case requestAccounts
        /// wallet_disconnect: forget the origin's permission.
        case disconnect
        /// eth_sendTransaction: always a native sheet, then the FeeChanged flow.
        case send
        /// A read. `verified` names the path that answers it: the certificate
        /// paths in the FFI, or the local node's RPC (which nothing here
        /// vouches for — the tab says so).
        case read(verified: Bool)
        /// Neither the extension nor this wallet answers it (EIP-1193 4200 —
        /// personal_sign, wallet_switchEthereumChain, …).
        case refused
    }

    /// The extension's cap on unanswered requests from one origin.
    static let maxPendingPerOrigin = 3

    static func route(method: String, params: [Any]) -> Route {
        if method == "eth_chainId" { return .chainId }
        if method == "eth_accounts" || method == "aether_accounts" { return .accounts }
        if method == "eth_requestAccounts" || method == "aether_requestAccounts" { return .requestAccounts }
        if method == "wallet_disconnect" || method == "aether_disconnect" { return .disconnect }
        if ProviderMethod.send.contains(method) { return .send }
        guard ProviderMethod.read.contains(method) else { return .refused }
        // The verified FFI paths — only the reads whose node response shape
        // the FFI can reproduce exactly (aether_status and aether_getReceipt
        // carry fields the FFI does not have, so the node answers those and
        // the tab marks them unverified).
        switch method {
        case "eth_blockNumber", "eth_getBalance", "net_version", "aether_accountHistory":
            return .read(verified: true)
        case "eth_call":
            // ethCall cannot say who is calling; a request with `from` needs
            // the node's view of that account.
            let hasFrom = (params.first as? [String: Any])?["from"] != nil
            return .read(verified: !hasFrom)
        default:
            return .read(verified: false)
        }
    }
}

/// The lock rule both entry points share: while the wallet is locked (or has
/// no key yet), nothing is answered at all — reads included, exactly as the
/// extension's vault refuses everything until unlocked.
enum ProviderGate {
    static func check(locked: Bool) -> ProviderError? {
        locked ? ProviderError(code: ProviderErrorCode.locked,
                               message: "The wallet is locked. Unlock \(Brand.project) and try again.") : nil
    }
}

/// The confirmation sheet's "Action" line — the extension's describeCall,
/// mirrored selector for selector (display only; the sheet also shows the
/// raw calldata under a disclosure).
enum CallDescribe {
    static func action(to: String, data: String) -> String {
        if to.isEmpty { return String(localized: "Deploy a contract (\((data.count - 2) / 2) bytes)") }
        if data == "0x" { return String(localized: "Send \(Brand.networkCoinTicker)") }
        let known: [String: String] = [
            "0xa9059cbb": String(localized: "Token transfer"), "0x095ea7b3": String(localized: "Token approval (allows spending)"), "0x23b872dd": String(localized: "Token transfer from"),
            // EastSea DEX router
            "0x38ed1739": String(localized: "Swap tokens"), "0xac344b4d": String(localized: "Swap \(Brand.networkCoinTicker) for tokens"), "0x3f070ce1": String(localized: "Swap tokens for \(Brand.networkCoinTicker)"),
            "0xe8e33700": String(localized: "Add liquidity"), "0xcf2df7c6": String(localized: "Add liquidity with \(Brand.networkCoinTicker)"), "0xbaa2abde": String(localized: "Remove liquidity"),
            "0x0fb9ca68": String(localized: "Remove liquidity to \(Brand.networkCoinTicker)"), "0xd0e30db0": String(localized: "Wrap \(Brand.networkCoinTicker)"), "0x2e1a7d4d": String(localized: "Unwrap \(Brand.networkCoinTicker)"),
            "0x3ca6d100": String(localized: "Create a token"), "0xc7ff321d": String(localized: "Create a token"),
            // EastSea launchpad
            "0x42a81515": String(localized: "Launch a token"), "0xcce7ec13": String(localized: "Buy on the launch curve"), "0x6a272462": String(localized: "Sell on the launch curve"),
            "0x5cf66fe1": String(localized: "Buy with \(Brand.networkCoinTicker) (graduated pool)"), "0xff5b07d8": String(localized: "Sell for \(Brand.networkCoinTicker) (graduated pool)"),
        ]
        return known[String(data.prefix(10))] ?? String(localized: "Contract call \(String(data.prefix(10)))")
    }
}
