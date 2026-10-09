import Foundation

/// Signature approval and visible busy state belong to an operation, rather
/// than to whichever transaction tracker finishes next. Use on the model actor.
struct WalletOperationGate {
    struct Token: Equatable {
        let id: UUID
    }

    private var signing: Token?
    private var current: Token?

    init() {}

    var blocksAccountChange: Bool { signing != nil }

    /// Tracking a submitted transaction may overlap a new signature; two
    /// signatures must never overlap or allow an account change underneath one.
    mutating func begin() -> Token? {
        guard signing == nil else { return nil }
        let token = Token(id: UUID())
        signing = token
        current = token
        return token
    }

    mutating func submitted(_ token: Token) {
        if signing == token { signing = nil }
    }

    /// Only a true result authorizes the caller to clear the model's busy flag.
    /// An older tracker's timeout, receipt, or failure cannot release a new one.
    @discardableResult
    mutating func release(_ token: Token) -> Bool {
        if signing == token { signing = nil }
        guard current == token else { return false }
        current = nil
        return true
    }
}
