import Foundation

/// What "새 가격으로 다시 보내기" needs to sign a dropped plain transfer again
/// (contracts-live bug #5): its recipient, exact amount in wei and nonce. The
/// same nonce means at most one of the two transactions can ever run.
/// Pure Foundation (Tests/resend).
struct ResendIntent: Codable, Equatable {
    let to: String
    let valueWei: String
    let nonce: UInt64

    /// The send sheet still holds exactly this transfer: the same recipient
    /// (any letter case) and the same wei amount. Anything else is a new send
    /// and takes the next nonce.
    func matches(to other: String, valueWei wei: String) -> Bool {
        other.lowercased() == to.lowercased() && wei == valueWei
    }
}
