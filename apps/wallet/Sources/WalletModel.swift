import Foundation
import SwiftUI

@MainActor
final class WalletModel: ObservableObject {
    @Published var connectionInfo = "Looking up validators on the Mainline DHT…"
    @Published var validators: UInt32 = 4
    @Published var address = ""
    @Published var account: VerifiedAccount?
    @Published var status: ChainStatus?
    @Published var blocks: [BlockInfo] = []
    @Published var verifyError: String?
    @Published var busy = false
    @Published var log: [String] = []
    @Published var sendTo = ""
    @Published var sendAmount = "1"

    private var enclave: EnclaveAccount?
    private var timer: Timer?

    func start() {
        pinCommittee()
        do {
            let acct = try EnclaveAccount.loadOrCreate(requireUserPresence: true)
            enclave = acct
            address = try accountAddress(p256PublicKey: acct.publicKey)
            note("Secure Enclave key ready. Signing asks for Touch ID or your password.")
        } catch {
            note("Key error: \(error.localizedDescription)")
        }
        refresh()
        timer = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.refresh() }
        }
    }

    /// Validators' node ids and the committee key, from the bundled network.json
    /// (written by `aether dkg`).
    func pinCommittee() {
        guard let url = Bundle.main.url(forResource: "network", withExtension: "json"),
              let json = try? String(contentsOf: url, encoding: .utf8) else {
            note("No network.json: using the public devnet validators and key.")
            return
        }
        do {
            validators = try configureNetwork(networkJson: json)
            let id = (try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])?["identity"] as? String ?? "devnet"
            note("Network: \(validators) validators · committee key \(id.prefix(16))… (network.json)")
        } catch {
            note("network.json rejected: \(error)")
        }
    }

    func note(_ s: String) {
        let t = DateFormatter.localizedString(from: Date(), dateStyle: .none, timeStyle: .medium)
        log.insert("\(t)  \(s)", at: 0)
        if log.count > 50 { log.removeLast() }
    }

    func refresh() {
        let addr = address, n = validators
        Task.detached {
            let st = try? chainStatus()
            let conn = connection()
            let bl = (try? recentBlocks(n: 8)) ?? []
            var acc: VerifiedAccount?
            var err: String?
            if !addr.isEmpty {
                do { acc = try verifiedAccount(address: addr, validators: n) } catch { err = "\(error)" }
            }
            await MainActor.run {
                self.status = st
                self.connectionInfo = conn
                self.blocks = bl
                if let acc { self.account = acc; self.verifyError = nil }
                if st == nil { self.verifyError = "No validator reachable yet (\(conn))" } else if let err { self.verifyError = err }
            }
        }
    }

    func faucet() {
        let addr = address
        busy = true
        Task.detached {
            do {
                let h = try devnetFaucet(to: addr, valueWei: Wei.from(aeth: "10")!)
                await self.track(h, label: "Faucet 10 AETH")
            } catch { await MainActor.run { self.note("Faucet failed: \(error)"); self.busy = false } }
        }
    }

    func send() {
        guard let enclave else { return }
        guard let wei = Wei.from(aeth: sendAmount) else { note("Invalid amount"); return }
        let to = sendTo.trimmingCharacters(in: .whitespaces), pk = enclave.publicKey
        busy = true
        Task.detached {
            do {
                let prepared = try prepareTransfer(p256PublicKey: pk, to: to, valueWei: wei)
                let sig = try enclave.sign(prepared.signingMessage)   // Secure Enclave, may prompt
                let h = try submitSigned(envelopeJson: prepared.envelopeJson, signature: sig, p256PublicKey: pk)
                await self.track(h, label: "Sent \(Wei.format(wei)) AETH (nonce \(prepared.nonce))")
            } catch { await MainActor.run { self.note("Send failed: \(error)"); self.busy = false } }
        }
    }

    private func track(_ hash: String, label: String) async {
        await MainActor.run { self.note("\(label) submitted \(hash.prefix(14))…") }
        for _ in 0..<60 {
            if let r = try? receipt(txHash: hash) {
                await MainActor.run {
                    self.note("\(label) finalized in block \(r.height) (\(r.success ? "success" : "failed"), gas \(r.gasUsed))")
                    self.busy = false
                    self.refresh()
                }
                return
            }
            try? await Task.sleep(nanoseconds: 500_000_000)
        }
        await MainActor.run { self.note("\(label): not finalized after 30s"); self.busy = false }
    }
}

/// Decimal AETH <-> wei strings (18 decimals), exact.
enum Wei {
    static func from(aeth: String) -> String? {
        let parts = aeth.trimmingCharacters(in: .whitespaces).split(separator: ".", omittingEmptySubsequences: false)
        guard parts.count <= 2, let whole = parts.first, !whole.isEmpty || parts.count == 2,
              (whole + (parts.count == 2 ? parts[1] : "")).allSatisfy(\.isNumber) else { return nil }
        let frac = parts.count == 2 ? String(parts[1]) : ""
        guard frac.count <= 18 else { return nil }
        let digits = String(whole) + frac + String(repeating: "0", count: 18 - frac.count)
        let trimmed = digits.drop(while: { $0 == "0" })
        return trimmed.isEmpty ? "0" : String(trimmed)
    }

    static func format(_ wei: String) -> String {
        let padded = String(repeating: "0", count: max(0, 19 - wei.count)) + wei
        let whole = padded.dropLast(18).drop(while: { $0 == "0" })
        var frac = String(padded.suffix(18))
        while frac.hasSuffix("0") { frac.removeLast() }
        let w = whole.isEmpty ? "0" : String(whole)
        return frac.isEmpty ? w : "\(w).\(frac.prefix(6))"
    }
}
