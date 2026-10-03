import Foundation

// Token-safety guard, ported from the extension's audit R2-2/R2-5 fixes
// (docs/research/audit-2-2026-10-03.md; see apps/extension/src/lib/knownTokens.js
// and lib/sendIntent.js). Unauthenticated node metadata must never define the
// units a transfer signs: the ONLY trusted denomination is the immutable list
// shipped in this file, keyed by chain id + lowercase token address. A token on
// the list sends with these decimals no matter what any node answers; a token
// not on it may be displayed, but its units are "unverified" and sending them
// needs an explicit confirmation of the exact base-unit count. The send intent
// freezes what the review screen showed (recipient, token, raw amount); the
// signing step re-checks it against the state at signing time and refuses when
// anything moved. Everything here is pure (Foundation only), so
// apps/wallet/Tests/token-guard checks it without an app or a node — including
// a drift check against the extension's own table, which stays the source of
// truth.

/// One entry of the shipped allowlist: the denomination this wallet itself
/// vouches for (from the repo's deployment records, never an RPC read).
struct KnownToken: Equatable {
    let symbol: String
    let name: String
    let decimals: Int
}

/// The built-in table of official token denominations. Mirrors the extension's
/// lib/knownTokens.js exactly; Tests/token-guard fails if either side drifts.
/// How to add an entry: paste the token's chain id and 0x address exactly as
/// deployed, with the symbol, name and decimals from the deployment record
/// (apps/agent/Resources/dex/*.json, token-sources.json, or the contract
/// source), NOT from an RPC read — a wrong decimals here mis-sizes every send
/// of that token, so a human checks it against the deployed contract first.
enum KnownTokens {
    /// The native coin of each chain (not an ERC-20; recorded so the trusted
    /// denomination table is complete). The app always shows the native amount
    /// at 18 decimals, independent of any node answer.
    static let native: [UInt64: KnownToken] = [
        7780: KnownToken(symbol: Brand.coinTicker, name: Brand.coinName, decimals: 18),
    ]

    /// Known ERC-20 tokens, by chain id and lowercase address. Chain 7780
    /// (aether-testnet), from apps/agent/Resources/dex/aether-testnet.json and
    /// the seed deployment in aether-dex's script/Deploy.s.sol: WAETH
    /// (src/WAETH.sol) and the NEB/ORB/CMT seed tokens (src/Token.sol via
    /// TokenFactory, 18 decimals each).
    static let tokens: [UInt64: [String: KnownToken]] = [
        7780: [
            "0xa2521982a17474cb2f8741c85de653b5282d72b0": KnownToken(symbol: "WAETH", name: "Wrapped AETH", decimals: 18),
            "0x6bc5ded76ccbdc8df35e7cd28b68fed245a74416": KnownToken(symbol: "NEB", name: "Test Nebula", decimals: 18),
            "0x961f8add5ae93ff0700be8abd5f9f8ec69ba4347": KnownToken(symbol: "ORB", name: "Test Orbit", decimals: 18),
            "0xc91367bac92c6de822de8afd0f34ff19fd8f7670": KnownToken(symbol: "CMT", name: "Test Comet", decimals: 18),
        ],
    ]

    /// The shipped entry for `address` on `chainId`, or nil (address matched
    /// case-insensitively; lookups never throw).
    static func knownToken(chainId: UInt64, address: String) -> KnownToken? {
        tokens[chainId]?[address.lowercased()]
    }
}

/// The denomination a display or a send must use for a token (the port of the
/// extension's tokenPin.denominationOf): the shipped list decides; the node's
/// claimed metadata only ever supplies unverified details. `claimed` is what
/// the token scanner last read on this device.
struct TokenDenomination: Equatable {
    let decimals: Int?
    let symbol: String?
    let name: String?
    let trusted: Bool
    /// Not on the shipped list: any amount shown is the token's own unverified
    /// claim, and sending needs an explicit base-unit confirmation.
    var unverifiedUnits = false
    /// On the shipped list, but the device's claim differs from it (display
    /// noise; the list still decides the units).
    var nodeDisagrees = false
    /// Nothing is known about this token at all (not even an unverified claim).
    var unconfirmed = false

    static func of(chainId: UInt64, address: String, claimed: TokenInfo?) -> TokenDenomination {
        if let known = KnownTokens.knownToken(chainId: chainId, address: address) {
            return TokenDenomination(decimals: known.decimals, symbol: known.symbol, name: known.name, trusted: true,
                                     nodeDisagrees: claimed.map { !sameMetadata($0, known) } ?? false)
        }
        guard let claimed else {
            return TokenDenomination(decimals: nil, symbol: nil, name: nil, trusted: false, unconfirmed: true)
        }
        return TokenDenomination(decimals: claimed.decimals, symbol: claimed.symbol, name: claimed.name,
                                 trusted: false, unverifiedUnits: true)
    }

    private static func sameMetadata(_ claimed: TokenInfo, _ known: KnownToken) -> Bool {
        claimed.decimals == known.decimals && claimed.symbol == known.symbol && claimed.name == known.name
    }
}

/// Why a send was refused. The texts are the plain sentences the extension
/// shows, so both front-ends say the same thing for the same refusal.
enum TokenGuardError: Error, Equatable {
    case tokenNotAddress
    case recipientNotAddress
    case unusableDecimals
    case unusableAmount
    case unusableBaseUnits
    case staleIntent
    case listDecimals
    case needsAcknowledgement
    case unconfirmed
}

extension TokenGuardError: LocalizedError {
    var errorDescription: String? {
        switch self {
        case .tokenNotAddress: return "the token is not an address"
        case .recipientNotAddress: return "the recipient is not an address"
        case .unusableDecimals: return "the token’s decimals are not usable"
        case .unusableAmount: return "the amount is not a plain number like 1.5"
        case .unusableBaseUnits: return "The confirmed amount is not a usable number of base units."
        case .staleIntent: return "This token’s details changed since the send was confirmed. Close this and confirm the send again."
        case .listDecimals: return "This token’s units come from the list shipped with the wallet; confirm the send again."
        case .needsAcknowledgement: return "This token is not on the wallet’s trusted list, so sending it needs your confirmation of the exact number of units."
        case .unconfirmed: return "This token’s details are not confirmed yet. Open Assets and let the wallet confirm them first."
        }
    }
}

/// One immutable send intent (audit R2-5): the token, the recipient and the
/// exact base-unit integer the review screen showed, frozen when that screen
/// opened. The signing step executes exactly this — it never re-parses an
/// amount text under whatever decimals are stored later.
struct SendIntent: Equatable {
    struct Token: Equatable {
        /// Lowercase 0x address of the ERC-20 contract.
        let address: String
        /// The decimals the review screen showed the amount under.
        let decimals: Int
        /// The units came from the shipped list (no acknowledgement needed).
        let trusted: Bool
        /// The user confirmed the exact base-unit count (unverified units only).
        var acknowledged: Bool
    }

    let token: Token
    /// As typed, trimmed (mixed case allowed; calldata encodes lowercased).
    let recipient: String
    /// A plain non-negative decimal integer of base units.
    let baseUnits: String

    /// 2^256 - 1, the largest amount a `uint256` transfer can carry.
    static let maxUint256 = "115792089237316195423570985008687907853269984665640564039457584007913129639935"

    /// Build the intent from what the user saw: `amountText` is parsed ONCE,
    /// under `token.decimals` — the decimals the review screen displayed — and
    /// every later step uses the resulting integer, never the text again.
    static func build(recipient: String, amountText: String, token: Token) throws -> SendIntent {
        let address = token.address.lowercased()
        guard SendSafety.isValidAddress(address) else { throw TokenGuardError.tokenNotAddress }
        let to = recipient.trimmingCharacters(in: .whitespacesAndNewlines)
        guard SendSafety.isValidAddress(to) else { throw TokenGuardError.recipientNotAddress }
        guard (0...77).contains(token.decimals) else { throw TokenGuardError.unusableDecimals }
        guard let units = TokenAmount.parse(amountText, decimals: token.decimals) else { throw TokenGuardError.unusableAmount }
        return SendIntent(token: Token(address: address, decimals: token.decimals,
                                       trusted: token.trusted, acknowledged: token.acknowledged),
                          recipient: to, baseUnits: units)
    }

    /// Check the frozen intent against the state at signing time and derive the
    /// calldata to sign. `current` is what the wallet knows about this token
    /// NOW (the scanner's latest claim, nil if it no longer knows the token);
    /// `known` is the shipped-list entry for it, or nil. Throws one plain
    /// sentence when the intent no longer matches; the caller must not fall
    /// back to any re-derived amount.
    static func check(_ intent: SendIntent, current: TokenInfo?, known: KnownToken?) throws -> String {
        guard !intent.baseUnits.isEmpty, intent.baseUnits.allSatisfy({ $0.isASCII && $0.isNumber }),
              WeiMath.compare(intent.baseUnits, maxUint256) <= 0 else { throw TokenGuardError.unusableBaseUnits }
        guard SendSafety.isValidAddress(intent.recipient) else { throw TokenGuardError.recipientNotAddress }
        if let known {
            // Trusted denomination: the shipped list decides the units. A node
            // answer that disagrees is display noise, never a reason to change
            // them — and an intent carrying anything but the list's decimals
            // (whatever the node said when it was built) is refused outright.
            guard intent.token.decimals == known.decimals else { throw TokenGuardError.listDecimals }
            guard let data = ERC20.transferCalldata(to: intent.recipient, amount: intent.baseUnits) else {
                throw TokenGuardError.unusableBaseUnits
            }
            return data
        }
        // Unverified units: only an explicit acknowledgement of the exact
        // base-unit count authorizes signing, against a token the wallet still
        // knows, with the details the confirmation showed.
        guard intent.token.acknowledged else { throw TokenGuardError.needsAcknowledgement }
        guard let current else { throw TokenGuardError.unconfirmed }
        guard current.decimals == intent.token.decimals else { throw TokenGuardError.staleIntent }
        guard let data = ERC20.transferCalldata(to: intent.recipient, amount: intent.baseUnits) else {
            throw TokenGuardError.unusableBaseUnits
        }
        return data
    }

    /// The same intent with the acknowledgement the confirm screen collected
    /// (the shown facts — recipient, token, amount — stay frozen).
    func with(acknowledged: Bool) -> SendIntent {
        SendIntent(token: Token(address: token.address, decimals: token.decimals,
                                trusted: token.trusted, acknowledged: acknowledged),
                   recipient: recipient, baseUnits: baseUnits)
    }

    /// "1000000000" → "1,000,000,000", for the confirm screen's unit count.
    static func grouped(_ units: String) -> String {
        var out: [Character] = []
        var fromEnd = 0
        for c in units.reversed() {
            if fromEnd > 0 && fromEnd % 3 == 0 { out.append(",") }
            out.append(c)
            fromEnd += 1
        }
        return String(out.reversed())
    }
}
