import Foundation

/// Uses the same selected node transport as native signing: the owner's local
/// node when selected, otherwise the wallet's peers (including on iOS).
enum DappRPC {
    static func simulate(_ tx: PageTransaction, context: DappRequestContext,
                         publicKey: Data) async throws -> SimulatedPageTransaction {
        let preview = try await Task.detached {
            try simulateDappTransaction(p256PublicKey: publicKey, to: tx.to, valueWei: tx.valueWei,
                                        dataHex: tx.data, gasLimit: tx.gas)
        }.value
        guard preview.gasLimit > 0, preview.gasLimit <= ProviderMethod.maxGas,
              tx.gas == 0 || preview.gasLimit == tx.gas,
              preview.resultJson.utf8.count <= 8 * 1024 * 1024,
              let data = preview.resultJson.data(using: .utf8) else {
            throw ProviderError(code: ProviderErrorCode.internalError,
                                message: String(localized: "The simulation could not be read. Try again."))
        }
        let result = try DappSimulation.parse(JSONSerialization.jsonObject(with: data), account: context.account)
        let transaction = PageTransaction(to: tx.to, valueWei: tx.valueWei, data: tx.data, gas: preview.gasLimit)
        return .init(transaction: transaction, context: context, result: result)
    }
}
