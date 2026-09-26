import Foundation
import SwiftUI
#if os(macOS)
import DeviceCheck
#endif

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
    @Published var recoveryCode = ""
    @Published var keyLabel = "Key in Secure Enclave"
    @Published var guardianInput = ""
    @Published var lostInput = ""
    /// Balance over time (this device's observations), for the dashboard chart.
    @Published var history: [BalancePoint] = []
    /// This wallet's own actions, newest first, for the simple-mode feed.
    @Published var activity: [ActivityItem] = []
    /// A recovery someone started on THIS account (cancel it if it was not you).
    @Published var incomingRecovery: RecoveryStatus?
    /// A recovery this device proposed for another account, waiting for its delay.
    @Published var outgoingRecovery: PendingRecovery?
    private var refreshes = 0

    private var enclave: EnclaveAccount?
    private var timer: Timer?

    func start() {
        guard timer == nil else { return }
        pinCommittee()
        do {
            let acct = try EnclaveAccount.loadOrCreate(requireUserPresence: true)
            enclave = acct
            address = try accountAddress(p256PublicKey: acct.publicKey)
            loadSaved()
            outgoingRecovery = PendingRecovery.load()
            recoveryCode = try recoveryKeyCode(p256PublicKey: acct.publicKey)
            keyLabel = acct.isSecureEnclave ? "Key in Secure Enclave" : "Simulator: software key (no Secure Enclave)"
            note(acct.isSecureEnclave ? "Secure Enclave key ready. Signing asks for Touch ID / Face ID or your passcode." : "Simulator: software key (no Secure Enclave here). Use a real device for hardware-bound keys.")
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

    /// Register another device's key as this account's recovery key (one signature).
    func setRecoveryKey() {
        guard let enclave else { return }
        let code = guardianInput.trimmingCharacters(in: .whitespacesAndNewlines), pk = enclave.publicKey
        busy = true
        Task.detached {
            do {
                let prepared = try prepareSetRecoveryKey(p256PublicKey: pk, recoveryCode: code)
                let sig = try enclave.sign(prepared.signingMessage)
                let h = try submitSigned(envelopeJson: prepared.envelopeJson, signature: sig, p256PublicKey: pk)
                await self.track(h, label: "Recovery key set to \(code.prefix(12))…", item: ActivityItem(kind: .security, title: "Recovery device added", amount: nil))
            } catch { await MainActor.run { self.note("Set recovery key failed: \(error)"); self.busy = false } }
        }
    }

    /// As a recovery device of `lostInput`: propose moving its funds here (two
    /// Secure Enclave signatures). They move only after the owner's delay, and
    /// the owner can cancel meanwhile; then `finishRecovery()`.
    func recover() {
        guard let enclave else { return }
        let lost = lostInput.trimmingCharacters(in: .whitespacesAndNewlines), pk = enclave.publicKey, n = validators
        busy = true
        Task.detached {
            do {
                let request = try prepareRecovery(p256PublicKey: pk, lostAccount: lost, validators: n)
                let guardianSig = try enclave.sign(request.message)          // authorize as recovery key
                let prepared = try prepareRecoverySubmit(p256PublicKey: pk, request: request, guardianSignature: guardianSig)
                let sig = try enclave.sign(prepared.signingMessage)           // relay from this account
                let h = try submitSigned(envelopeJson: prepared.envelopeJson, signature: sig, p256PublicKey: pk)
                let pending = PendingRecovery(request: request, readyAt: Date().addingTimeInterval(TimeInterval(request.delaySeconds)))
                await MainActor.run { self.outgoingRecovery = pending; pending.save() }
                await self.track(h, label: "Recovery of \(lost.prefix(10))… proposed; funds can move after \(pending.readyAt.formatted())",
                                 item: ActivityItem(kind: .security, title: "Recovery started for \(Short.address(lost))", amount: nil))
            } catch { await MainActor.run { self.note("Recovery failed: \(error)"); self.busy = false } }
        }
    }

    /// After the delay: move the recovered funds (one Secure Enclave signature).
    func finishRecovery() {
        guard let enclave, let pending = outgoingRecovery else { return }
        let pk = enclave.publicKey
        busy = true
        Task.detached {
            do {
                let prepared = try prepareFinishRecovery(p256PublicKey: pk, request: pending.request)
                let sig = try enclave.sign(prepared.signingMessage)
                let h = try submitSigned(envelopeJson: prepared.envelopeJson, signature: sig, p256PublicKey: pk)
                // Keep the request until the chain confirms it ran: a revert (e.g. the
                // delay counted from inclusion, not from submission) can be retried.
                let ok = await self.track(h, label: "Recovered \(Wei.format(pending.request.valueWei)) AETH from \(pending.request.lost.prefix(10))…",
                                 item: ActivityItem(kind: .received, title: "Recovered from \(Short.address(pending.request.lost))",
                                                    amount: Double(Wei.format(pending.request.valueWei))))
                await MainActor.run {
                    if ok {
                        self.outgoingRecovery = nil
                        PendingRecovery.clear()
                    } else {
                        self.note("The recovery did not run yet (still inside its delay, or cancelled by the owner). You can try again.")
                    }
                }
            } catch { await MainActor.run { self.note("Finishing recovery failed: \(error)"); self.busy = false } }
        }
    }

    /// Stop a recovery of this account that you did not start (one Secure Enclave signature).
    func cancelIncomingRecovery() {
        guard let enclave else { return }
        let pk = enclave.publicKey
        busy = true
        Task.detached {
            do {
                let prepared = try prepareCancelRecovery(p256PublicKey: pk)
                let sig = try enclave.sign(prepared.signingMessage)
                let h = try submitSigned(envelopeJson: prepared.envelopeJson, signature: sig, p256PublicKey: pk)
                await MainActor.run { self.incomingRecovery = nil }
                await self.track(h, label: "Cancelled a recovery of this account", item: ActivityItem(kind: .security, title: "Recovery cancelled", amount: nil))
            } catch { await MainActor.run { self.note("Cancel failed: \(error)"); self.busy = false } }
        }
    }

    #if os(macOS)
    /// Register this Mac as a voting node, operated by this wallet (one Touch ID).
    /// Apple's DeviceCheck token proves it is a real Mac that never registered
    /// before: one Mac, one voting node.
    func registerNode(_ c: NodeController.Candidate, node: NodeController) {
        guard let enclave else { return }
        guard let chainId = status?.chainId, let ownership = node.ownership(account: address, chainId: chainId) else {
            note("Voting-node registration: the node's keys are not ready yet")
            return
        }
        let pk = enclave.publicKey
        busy = true
        Task.detached {
            do {
                guard DCDevice.current.isSupported else { throw NodeRegistrationError.unsupported }
                let token = try await DCDevice.current.generateToken().base64EncodedString()
                let prepared = try prepareRegisterNode(p256PublicKey: pk, deviceToken: token, validatorKey: c.validatorKey, nodeId: c.nodeId, beaconer: c.beaconer, ownership: ownership)
                let sig = try enclave.sign(prepared.signingMessage)
                let h = try submitSigned(envelopeJson: prepared.envelopeJson, signature: sig, p256PublicKey: pk)
                await self.track(h, label: "This Mac is registered as a voting node", item: ActivityItem(kind: .security, title: "Mac joined as a voting node", amount: nil))
            } catch { await MainActor.run { self.note("Voting-node registration failed: \(error)"); self.busy = false } }
        }
    }
    #endif

    func note(_ s: String) {
        let t = DateFormatter.localizedString(from: Date(), dateStyle: .none, timeStyle: .medium)
        log.insert("\(t)  \(s)", at: 0)
        if log.count > 50 { log.removeLast() }
    }

    func refresh() {
        let addr = address, n = validators
        refreshes += 1
        // Recovery status needs several proofs; every 30 s is enough to warn within the delay.
        let checkRecovery = refreshes % 15 == 1 && !addr.isEmpty
        Task.detached {
            if checkRecovery, let rs = try? recoveryStatus(account: addr, validators: n) {
                await MainActor.run { self.incomingRecovery = rs.pending ? rs : nil }
            }
            let st = try? chainStatus()
            let conn = connection()
            let bl = (try? recentBlocks(n: 24)) ?? []
            var acc: VerifiedAccount?
            var err: String?
            if !addr.isEmpty {
                do { acc = try verifiedAccount(address: addr, validators: n) } catch { err = "\(error)" }
            }
            await MainActor.run {
                self.status = st
                self.connectionInfo = conn
                self.blocks = bl
                if let acc { self.account = acc; self.verifyError = nil; self.record(balanceWei: acc.balanceWei) }
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
                await self.track(h, label: "Faucet 10 AETH", item: ActivityItem(kind: .received, title: "Test AETH from faucet", amount: 10))
            } catch { await MainActor.run { self.note("Faucet failed: \(error)"); self.busy = false } }
        }
    }

    func send() {
        guard let enclave else { return }
        guard let wei = Wei.from(aeth: sendAmount) else { note("Invalid amount"); return }
        // One or more recipients (comma/space separated); each gets the amount.
        let recipients = sendTo.split(whereSeparator: { $0 == "," || $0.isWhitespace }).map(String.init).filter { !$0.isEmpty }
        guard !recipients.isEmpty else { return }
        let pk = enclave.publicKey
        busy = true
        Task.detached {
            do {
                let prepared: PreparedTx
                let label: String
                let each = Double(Wei.format(wei)) ?? 0
                let who = recipients.count == 1 ? Short.address(recipients[0]) : "\(recipients.count) people"
                let item = ActivityItem(kind: .sent, title: "Sent to \(who)", amount: -each * Double(recipients.count))
                if recipients.count == 1 {
                    prepared = try prepareTransfer(p256PublicKey: pk, to: recipients[0], valueWei: wei)
                    label = "Sent \(Wei.format(wei)) AETH (nonce \(prepared.nonce))"
                } else {
                    // All payments in one tx: one signature, all or nothing (EIP-7702 batch).
                    prepared = try prepareBatch(p256PublicKey: pk, payments: recipients.map { Payment(to: $0, valueWei: wei) })
                    label = "Paid \(recipients.count) recipients \(Wei.format(wei)) AETH each with one signature (nonce \(prepared.nonce))"
                }
                let sig = try enclave.sign(prepared.signingMessage)   // Secure Enclave, may prompt
                let h = try submitSigned(envelopeJson: prepared.envelopeJson, signature: sig, p256PublicKey: pk)
                await self.track(h, label: label, item: item)
            } catch { await MainActor.run { self.note("Send failed: \(error)"); self.busy = false } }
        }
    }

    /// Wait for finality; returns whether the tx succeeded.
    @discardableResult
    private func track(_ hash: String, label: String, item: ActivityItem) async -> Bool {
        await MainActor.run {
            self.note("\(label) submitted \(hash.prefix(14))…")
            self.activity.insert(item.with(state: .pending), at: 0)
        }
        for _ in 0..<60 {
            if let r = try? receipt(txHash: hash) {
                await MainActor.run {
                    self.settle(item.id, state: r.success ? .done : .failed)
                    self.note("\(label) finalized in block \(r.height) (\(r.success ? "success" : "failed"), gas \(r.gasUsed))")
                    self.busy = false
                    self.refresh()
                }
                return r.success
            }
            try? await Task.sleep(nanoseconds: 500_000_000)
        }
        await MainActor.run { self.settle(item.id, state: .failed); self.note("\(label): not finalized after 30s"); self.busy = false }
        return false
    }

    // MARK: dashboard data (kept per account in UserDefaults)

    private var historyKey: String { "balanceHistory.\(address)" }
    private var activityKey: String { "activity.\(address)" }

    private func loadSaved() {
        let d = UserDefaults.standard
        history = d.data(forKey: historyKey).flatMap { try? JSONDecoder().decode([BalancePoint].self, from: $0) } ?? []
        activity = d.data(forKey: activityKey).flatMap { try? JSONDecoder().decode([ActivityItem].self, from: $0) } ?? []
    }

    private func save() {
        let d = UserDefaults.standard
        d.set(try? JSONEncoder().encode(history), forKey: historyKey)
        d.set(try? JSONEncoder().encode(Array(activity.prefix(100))), forKey: activityKey)
    }

    /// Add a chart point when the balance changes, or once a minute otherwise.
    private func record(balanceWei: String) {
        let aeth = Double(Wei.format(balanceWei)) ?? 0
        let now = Date()
        if let last = history.last, last.aeth == aeth, now.timeIntervalSince(last.date) < 60 { return }
        history.append(BalancePoint(date: now, aeth: aeth))
        if history.count > 500 { history.removeFirst(history.count - 500) }
        save()
    }

    private func settle(_ id: UUID, state: ActivityItem.State) {
        if let i = activity.firstIndex(where: { $0.id == id }) {
            activity[i] = activity[i].with(state: state)
            save()
        }
    }
}

struct BalancePoint: Codable, Identifiable, Equatable {
    var id: Date { date }
    let date: Date
    let aeth: Double
}

struct ActivityItem: Codable, Identifiable, Equatable {
    enum Kind: String, Codable { case sent, received, security }
    enum State: String, Codable { case pending, done, failed }
    var id = UUID()
    var date = Date()
    let kind: Kind
    let title: String
    /// Signed AETH change (nil for non-transfers).
    let amount: Double?
    var state: State = .pending

    func with(state: State) -> ActivityItem {
        var c = self
        c.state = state
        return c
    }
}

enum Short {
    static func address(_ a: String) -> String {
        a.count > 12 ? "\(a.prefix(6))…\(a.suffix(4))" : a
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

/// A recovery this device proposed, kept until it is finished (survives restarts).
struct PendingRecovery {
    let request: RecoveryRequest
    let readyAt: Date

    var isReady: Bool { Date() >= readyAt }

    private static let key = "pendingRecovery"

    func save() {
        let r = request
        let d: [String: Any] = ["lost": r.lost, "to": r.to, "value": r.valueWei, "nonce": r.guardianNonce, "index": Int(r.guardianIndex),
                                "delay": r.delaySeconds, "message": r.message.base64EncodedString(), "readyAt": readyAt.timeIntervalSince1970]
        UserDefaults.standard.set(d, forKey: Self.key)
    }

    static func load() -> PendingRecovery? {
        guard let d = UserDefaults.standard.dictionary(forKey: key), let lost = d["lost"] as? String, let to = d["to"] as? String,
              let value = d["value"] as? String, let nonce = d["nonce"] as? UInt64, let index = d["index"] as? Int,
              let delay = d["delay"] as? UInt64, let msg = (d["message"] as? String).flatMap({ Data(base64Encoded: $0) }),
              let ready = d["readyAt"] as? Double else { return nil }
        let r = RecoveryRequest(lost: lost, to: to, valueWei: value, guardianNonce: nonce, guardianIndex: UInt8(index), delaySeconds: delay, message: msg)
        return PendingRecovery(request: r, readyAt: Date(timeIntervalSince1970: ready))
    }

    static func clear() { UserDefaults.standard.removeObject(forKey: key) }
}

enum NodeRegistrationError: LocalizedError {
    case unsupported
    var errorDescription: String? { "This Mac cannot create a DeviceCheck token (needs a signed Aether app on a real Mac)." }
}
