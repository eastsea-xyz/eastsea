import Foundation

/// UI conveniences share AccountStore's durable selection and retirement
/// guards. Saving an outgoing account still uses its captured address.
@MainActor
extension AccountStore {
    @discardableResult
    func createAndSelect(name: String? = nil, color: String = "violet") throws -> WalletAccount {
        let account = try create(name: name, color: color)
        try select(account.id)
        return account
    }

    func activeDataStore(chainID: UInt64, defaults: UserDefaults = .standard) -> AccountDataStore? {
        guard state == .ready, let activeAccount else { return nil }
        return AccountDataStore(chainID: chainID, address: activeAccount.address, defaults: defaults)
    }

    /// A preview explains why the action is unavailable. `delete` performs
    /// its own fresh check at confirmation, even after this returned nil.
    func retirementFailure(for id: Int) -> Failure? {
        guard canChangeAccount() else { return .operationInProgress }
        guard state == .ready, let account = list().first(where: { $0.id == id }) else { return .accountNotFound }
        guard list().count > 1 else { return .lastAccount }
        guard account.address != payoutAddress else { return .payoutAccount }
        do {
            let balances = try readBalances(account)
            guard ([balances.nativeWei] + balances.tokenBalances).allSatisfy({ value in
                !value.isEmpty && value.allSatisfy { $0.isASCII && $0.isNumber }
            }) else { return .balanceUnavailable }
            return balances.isZero ? nil : .balanceNotZero
        } catch {
            return .balanceUnavailable
        }
    }
}
