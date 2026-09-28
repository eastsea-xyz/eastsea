import Foundation
import SwiftUI
#if os(macOS)
import AppKit
#endif
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
    /// Since when verification has been failing (nil while it works).
    @Published var verifyFailingSince: Date?
    /// The network this app knows no longer matches the chain (reset or upgrade): update.
    @Published var networkOutdated = false
    /// Called once when the network looks outdated (the app checks for its update).
    var onOutdated: (() -> Void)?
    /// A payment a web page or another app asked for (`aether://pay?...`), shown for approval.
    @Published var paymentRequest: PaymentRequest?
    /// A contract call or deployment a page asked for (`aether://call?...`).
    @Published var callRequest: CallRequest?
    /// A page asking for this wallet's address (`aether://connect?...`).
    @Published var connectRequest: ConnectRequest?
    /// Voting-node registration in progress or failed (nil: idle or done).
    @Published var registration: RegistrationState?
    @Published var busy = false
    @Published var log: [String] = []
    @Published var sendTo = ""
    @Published var sendAmount = "1"
    @Published var recoveryCode = ""
    @Published var keyLabel = "Key in Secure Enclave"
    @Published var guardianInput = ""
    @Published var lostInput = ""
    /// Freshly generated recovery words, shown once until registered or dismissed.
    @Published var paperWords: String?
    /// Recovery words typed in to recover a lost account.
    @Published var paperWordsInput = ""
    /// Balance over time (this device's observations), for the dashboard chart.
    @Published var history: [BalancePoint] = []
    /// This wallet's own actions, newest first, for the simple-mode feed.
    @Published var activity: [ActivityItem] = []
    /// A recovery someone started on THIS account (cancel it if it was not you).
    @Published var incomingRecovery: RecoveryStatus?
    /// A recovery this device proposed for another account, waiting for its delay.
    @Published var outgoingRecovery: PendingRecovery?
    /// ERC-20 tokens with a non-zero balance (read from the node, not light-client verified).
    @Published var tokens: [TokenHolding] = []
    /// When `tokens` was last read (nil: never, for this account).
    @Published var tokensUpdated: Date?
    @Published var tokensError: String?
    /// Since when the chain has made no new block (nil while it moves). Light
    /// verification needs the next block and a recent certificate, so a paused chain
    /// cannot verify: the last verified balance stays on screen meanwhile.
    @Published var chainPausedSince: Date?
    private func setVerifyError(_ e: String?) {
        if verifyError != e { verifyError = e }
    }

    private var refreshes = 0
    private var lastHeight: UInt64?
    private var heightChangedAt: Date?
    private var tokenScanRunning = false

    /// No new block for this long means the network is paused.
    static let pauseAfter: TimeInterval = 60
    /// Token balances are read at most this often (they are not on the 2 s cadence).
    static let tokenRefreshSeconds: TimeInterval = 30

    private var enclave: EnclaveAccount?
    /// Why this device has no wallet key yet (e.g. the Mac was locked when the
    /// app started: the Secure Enclave only makes keys while it is unlocked).
    @Published var keyError: String?
    private var lastKeyAttempt = Date.distantPast
    private var timer: Timer?

    func start() {
        guard timer == nil else { return }
        #if DEBUG
        if DesignPreview.on { return loadPreview() }
        #endif
        pinCommittee()
        loadKey()
        refresh()
        timer = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.refresh() }
        }
    }

    /// Load or create the wallet key. Retried from `refresh` until it works.
    private func loadKey() {
        lastKeyAttempt = Date()
        do {
            let acct = try EnclaveAccount.loadOrCreate(requireUserPresence: true)
            enclave = acct
            address = try accountAddress(p256PublicKey: acct.publicKey)
            keyError = nil
            loadSaved()
            outgoingRecovery = PendingRecovery.load()
            loadTokens()
            recoveryCode = try recoveryKeyCode(p256PublicKey: acct.publicKey)
            keyLabel = acct.isSecureEnclave ? "Key in Secure Enclave" : "Simulator: software key (no Secure Enclave)"
            note(acct.isSecureEnclave ? "Secure Enclave key ready. Signing asks for Touch ID / Face ID or your passcode." : "Simulator: software key (no Secure Enclave here). Use a real device for hardware-bound keys.")
        } catch {
            let locked = (error as NSError).code == Int(errSecInteractionNotAllowed)
            keyError = locked ? "Unlock this device to create your wallet key." : "Could not create the wallet key: \(error.localizedDescription)"
            note("Key error: \(error.localizedDescription)")
        }
    }

    /// Validators' node ids and the committee key, from the bundled network.json
    /// (written by `aether dkg`).
    func pinCommittee() {
        guard let url = Bundle.main.url(forResource: "network", withExtension: "json"),
              let json = try? String(contentsOf: url, encoding: .utf8) else {
            note("No network.json: nothing is verified without the committee key it names.")
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
                // Added next to any existing recovery keys (their threshold and delay stay).
                let prepared = try prepareAddRecoveryKey(p256PublicKey: pk, recoveryCode: code)
                let sig = try enclave.sign(prepared.signingMessage)
                let h = try submitSigned(envelopeJson: prepared.envelopeJson, signature: sig, p256PublicKey: pk)
                await self.track(h, label: "Recovery key set to \(code.prefix(12))…", item: ActivityItem(kind: .security, title: "Recovery device added", amount: nil))
            } catch { await MainActor.run { self.note("Set recovery key failed: \(error)"); self.busy = false } }
        }
    }

    /// 24 recovery words that stand in for a recovery device (the key in the
    /// Secure Enclave itself can never be written down).
    func createPaperKey() {
        paperWords = paperKeyNew()
    }

    /// Register the shown words as this account's recovery key (one Touch ID).
    func registerPaperKey() {
        guard let enclave, let words = paperWords else { return }
        let pk = enclave.publicKey
        busy = true
        Task.detached {
            do {
                let code = try recoveryKeyCode(p256PublicKey: try paperKeyPublic(words: words))
                let prepared = try prepareAddRecoveryKey(p256PublicKey: pk, recoveryCode: code)
                let sig = try enclave.sign(prepared.signingMessage)
                let h = try submitSigned(envelopeJson: prepared.envelopeJson, signature: sig, p256PublicKey: pk)
                await MainActor.run { self.paperWords = nil }
                await self.track(h, label: "Recovery words registered as a recovery key", item: ActivityItem(kind: .security, title: "Recovery words added", amount: nil))
            } catch { await MainActor.run { self.note("Registering recovery words failed: \(error)"); self.busy = false } }
        }
    }

    /// With every device lost: the recovery words propose moving `lostInput`'s
    /// funds to this Mac's account; they move after the account's delay.
    func recoverWithWords() {
        guard let enclave else { return }
        let words = paperWordsInput.trimmingCharacters(in: .whitespacesAndNewlines), lost = lostInput.trimmingCharacters(in: .whitespacesAndNewlines)
        let pk = enclave.publicKey, me = address, n = validators
        busy = true
        Task.detached {
            do {
                let request = try prepareRecoveryTo(p256PublicKey: try paperKeyPublic(words: words), lostAccount: lost, to: me, validators: n)
                let guardianSig = try paperKeySign(words: words, message: request.message)      // the words authorize
                let prepared = try prepareRecoverySubmit(p256PublicKey: pk, request: request, guardianSignature: guardianSig)
                let sig = try enclave.sign(prepared.signingMessage)                              // this Mac relays
                let h = try submitSigned(envelopeJson: prepared.envelopeJson, signature: sig, p256PublicKey: pk)
                let pending = PendingRecovery(request: request, readyAt: Date().addingTimeInterval(TimeInterval(request.delaySeconds)))
                await MainActor.run { self.outgoingRecovery = pending; pending.save(); self.paperWordsInput = "" }
                await self.track(h, label: "Recovery of \(lost.prefix(10))… proposed with recovery words; funds can move after \(pending.readyAt.formatted())",
                                 item: ActivityItem(kind: .security, title: "Recovery started for \(Short.address(lost))", amount: nil))
            } catch { await MainActor.run { self.note("Recovery with words failed: \(error)"); self.busy = false } }
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

    /// Remove every recovery key (and any pending recovery with them), e.g. when a
    /// recovery device or the recovery words may be in someone else's hands.
    /// Trusted devices or new words are then added again.
    func removeRecoveryKeys() {
        guard let enclave else { return }
        let pk = enclave.publicKey
        busy = true
        Task.detached {
            do {
                let prepared = try prepareRemoveRecoveryKeys(p256PublicKey: pk)
                let sig = try enclave.sign(prepared.signingMessage)
                let h = try submitSigned(envelopeJson: prepared.envelopeJson, signature: sig, p256PublicKey: pk)
                await MainActor.run { self.incomingRecovery = nil }
                await self.track(h, label: "Removed every recovery key", item: ActivityItem(kind: .security, title: "Recovery keys removed", amount: nil))
            } catch { await MainActor.run { self.note("Remove recovery keys failed: \(error)"); self.busy = false } }
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
            registration = .failed("The node is still starting. Try again in a minute.")
            return
        }
        let pk = enclave.publicKey
        busy = true
        registration = .working
        Task.detached {
            do {
                guard DCDevice.current.isSupported else { throw NodeRegistrationError.unsupported }
                let token = try await DCDevice.current.generateToken().base64EncodedString()
                let prepared = try prepareRegisterNode(p256PublicKey: pk, deviceToken: token, validatorKey: c.validatorKey, nodeId: c.nodeId, beaconer: c.beaconer, ownership: ownership)
                let sig = try enclave.sign(prepared.signingMessage)
                let h = try submitSigned(envelopeJson: prepared.envelopeJson, signature: sig, p256PublicKey: pk)
                let ok = await self.track(h, label: "This Mac is registered as a voting node", item: ActivityItem(kind: .security, title: "Mac joined as a voting node", amount: nil))
                await MainActor.run { self.registration = ok ? nil : .failed("The registration transaction did not go through. Try again.") }
            } catch {
                let reason = (error as? LocalizedError)?.errorDescription ?? "\(error)"
                await MainActor.run {
                    self.note("Voting-node registration failed: \(error)")
                    self.registration = .failed(reason)
                    self.busy = false
                }
            }
        }
    }
    #endif

    func note(_ s: String) {
        let t = DateFormatter.localizedString(from: Date(), dateStyle: .none, timeStyle: .medium)
        log.insert("\(t)  \(s)", at: 0)
        if log.count > 50 { log.removeLast() }
    }

    func refresh() {
        if enclave == nil, Date().timeIntervalSince(lastKeyAttempt) > 5 { loadKey() }
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
                // Published only when something actually changed: an unchanged set
                // would still invalidate every view watching this model (the whole
                // window), which lands right on top of live resizes.
                if self.status != st { self.status = st }
                if self.connectionInfo != conn { self.connectionInfo = conn }
                if self.blocks != bl { self.blocks = bl }
                if let acc {
                    if self.account != acc { self.account = acc }
                    if self.verifyError != nil { self.verifyError = nil }
                    self.record(balanceWei: acc.balanceWei)
                }
                // A node that is still catching up serves an older block than one this wallet
                // already verified; the FFI refuses it (finalized blocks never go back). That
                // is not an error to show: keep the newer verified balance.
                let behindNode = err?.contains("finalized blocks never go back") == true && self.account != nil
                if st == nil { self.setVerifyError("No validator reachable yet (\(conn))") } else if let err, !behindNode { self.setVerifyError(err) }
                self.trackChainProgress(st, blocks: bl)
                self.trackVerification()
                self.refreshTokens()
            }
        }
    }

    /// The chain is paused when its height has not moved for `pauseAfter`, or its
    /// newest block is that old (while the height is not moving here either, so a
    /// wrong clock on this device alone never looks like a pause).
    private func trackChainProgress(_ st: ChainStatus?, blocks: [BlockInfo]) {
        guard let st else { return }
        let now = Date()
        if st.height != lastHeight {
            lastHeight = st.height
            heightChangedAt = now
        }
        let newest = blocks.max(by: { $0.height < $1.height }).map { Date(timeIntervalSince1970: TimeInterval($0.timestampMs) / 1000) }
        let still = heightChangedAt.map { now.timeIntervalSince($0) } ?? 0
        let oldBlock = newest.map { now.timeIntervalSince($0) > Self.pauseAfter } ?? false
        let paused = still > Self.pauseAfter || (oldBlock && still > 20)
        let since = paused ? min(newest ?? heightChangedAt ?? now, heightChangedAt ?? now) : nil
        if since != chainPausedSince { chainPausedSince = since }
    }

    // MARK: tokens

    /// Read token balances again if the last read is older than `tokenRefreshSeconds`
    /// (`force`: a few seconds, e.g. when the Assets sheet opens).
    func refreshTokens(force: Bool = false) {
        let minAge = force ? 5 : Self.tokenRefreshSeconds
        #if DEBUG
        if DesignPreview.on { return }
        #endif
        guard !tokenScanRunning, !address.isEmpty, let chain = status?.chainId,
              tokensUpdated.map({ Date().timeIntervalSince($0) >= minAge }) ?? true else { return }
        guard let sources = TokenSources.bundled(chainId: chain) else {
            tokensError = nil
            tokensUpdated = Date()
            return
        }
        tokenScanRunning = true
        let owner = address, catalogKey = "tokenCatalog.\(chain)"
        let catalog = UserDefaults.standard.data(forKey: catalogKey).flatMap { try? JSONDecoder().decode(TokenCatalog.self, from: $0) } ?? TokenCatalog()
        Task.detached {
            let result = Result { try TokenScanner.scan(owner: owner, sources: sources, catalog: catalog, read: { try ethCall(to: $0, dataHex: $1) }) }
            await MainActor.run {
                self.tokenScanRunning = false
                guard owner == self.address else { return }
                switch result {
                case .success(let (cat, held)):
                    UserDefaults.standard.set(try? JSONEncoder().encode(cat), forKey: catalogKey)
                    self.tokens = held
                    self.tokensError = nil
                    self.tokensUpdated = Date()
                    UserDefaults.standard.set(try? JSONEncoder().encode(held), forKey: self.tokensKey)
                case .failure(let e):
                    self.tokensError = "\(e)"
                    // Try again on the normal cadence, not every 2 s.
                    self.tokensUpdated = Date()
                }
            }
        }
    }

    private var tokensKey: String { "tokenHoldings.\(address)" }

    /// The last token balances read for this account, shown until the next read.
    private func loadTokens() {
        tokens = UserDefaults.standard.data(forKey: tokensKey).flatMap { try? JSONDecoder().decode([TokenHolding].self, from: $0) } ?? []
    }

    /// Verification that keeps failing on certificates means the chain moved on
    /// (a new genesis or protocol) and this app is outdated: say so and update.
    private func trackVerification() {
        guard account == nil || verifyError != nil, let err = verifyError else {
            verifyFailingSince = nil
            return
        }
        let since = verifyFailingSince ?? Date()
        verifyFailingSince = since
        let certificate = err.localizedCaseInsensitiveContains("certificate") || err.localizedCaseInsensitiveContains("chain id")
        // A paused chain fails verification too, but updating the app would not help.
        if certificate, chainPausedSince == nil, Date().timeIntervalSince(since) > 30, !networkOutdated {
            networkOutdated = true
            onOutdated?()
        }
    }

    /// `aether://pay?to=0x…&amount=1.5&memo=…&callback=https://…` from a web page
    /// (no extension needed): the payment is shown for approval, never sent by itself.
    func open(url: URL) {
        guard url.scheme == "aether", let c = URLComponents(url: url, resolvingAgainstBaseURL: false) else { return }
        // A repeated parameter keeps its first value (never a crash on odd links).
        let q = Dictionary((c.queryItems ?? []).compactMap { i in i.value.map { (i.name, $0) } }, uniquingKeysWith: { first, _ in first })
        let action = c.host ?? c.path
        // One request at a time, never written into what the user is typing.
        guard paymentRequest == nil, callRequest == nil, connectRequest == nil else {
            note("Ignored a link while another request is waiting for approval")
            return
        }
        let callback = q["callback"].flatMap(URL.init(string:)).flatMap(Self.allowedCallback)
        switch action {
        case "pay":
            guard let to = q["to"], Wei.from(aeth: q["amount"] ?? "") != nil else { return note("Ignored a payment link that is not complete") }
            paymentRequest = PaymentRequest(to: to, amount: q["amount"] ?? "", memo: q["memo"], callback: callback)
        case "call":
            let value = q["value"] ?? "0"
            guard Wei.from(aeth: value) != nil, (q["data"] ?? "").hasPrefix("0x"), (q["data"] ?? "").count >= 10 || q["to"] == nil else {
                return note("Ignored a contract call link that is not complete")
            }
            callRequest = CallRequest(to: q["to"] ?? "", value: value, data: q["data"] ?? "0x", gas: UInt64(q["gas"] ?? "") ?? 0, memo: q["memo"], origin: q["origin"], callback: callback)
        case "connect":
            guard let callback else { return note("Ignored a connect link without a callback") }
            connectRequest = ConnectRequest(origin: q["origin"] ?? callback.host ?? "a page", callback: callback)
        default:
            note("Ignored an unknown aether:// link")
        }
    }

    /// Results go back only to https pages (or a page served from this Mac).
    static func allowedCallback(_ u: URL) -> URL? {
        if u.scheme == "https" { return u }
        if u.scheme == "http", ["localhost", "127.0.0.1"].contains(u.host ?? "") { return u }
        return nil
    }

    /// Tell the page this wallet's address (after the user approved).
    func approveConnect() {
        guard let r = connectRequest else { return }
        connectRequest = nil
        reply(r.callback, ["address": address, "chain_id": String(status?.chainId ?? 0)])
    }

    /// Sign and send a contract call a page asked for (Touch ID), then tell the page.
    func approveCall() {
        guard let enclave, let r = callRequest, let wei = Wei.from(aeth: r.value) else { return }
        callRequest = nil
        let pk = enclave.publicKey
        busy = true
        Task.detached {
            do {
                let prepared = try prepareCall(p256PublicKey: pk, to: r.to, valueWei: wei, dataHex: r.data, gasLimit: r.gas)
                let sig = try enclave.sign(prepared.signingMessage)
                let h = try submitSigned(envelopeJson: prepared.envelopeJson, signature: sig, p256PublicKey: pk)
                let title = r.to.isEmpty ? "Deployed a contract" : "Called \(Short.address(r.to))"
                let ok = await self.track(h, label: title, item: ActivityItem(kind: .sent, title: title, amount: nil))
                await MainActor.run { if let cb = r.callback { self.reply(cb, ["tx": h, "status": ok ? "success" : "failed"]) } }
            } catch { await MainActor.run { self.note("Call failed: \(error)"); self.busy = false } }
        }
    }

    private func reply(_ back: URL, _ items: [String: String]) {
        guard var c = URLComponents(url: back, resolvingAgainstBaseURL: false) else { return }
        c.queryItems = (c.queryItems ?? []) + items.sorted { $0.key < $1.key }.map { URLQueryItem(name: $0.key, value: $0.value) }
        #if os(macOS)
        if let u = c.url { NSWorkspace.shared.open(u) }
        #endif
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
        // A payment link is sent exactly as it asked; otherwise the form's values.
        let (toText, amountText) = paymentRequest.map { ($0.to, $0.amount) } ?? (sendTo, sendAmount)
        guard let wei = Wei.from(aeth: amountText) else { note("Invalid amount"); return }
        // One or more recipients (comma/space separated); each gets the amount.
        let recipients = toText.split(whereSeparator: { $0 == "," || $0.isWhitespace }).map(String.init).filter { !$0.isEmpty }
        guard !recipients.isEmpty else { return }
        let pk = enclave.publicKey
        let callback = paymentRequest?.callback
        paymentRequest = nil
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
                let ok = await self.track(h, label: label, item: item)
                // A web page that asked for this payment hears back (https only).
                if let back = callback, var c = URLComponents(url: back, resolvingAgainstBaseURL: false) {
                    c.queryItems = (c.queryItems ?? []) + [URLQueryItem(name: "tx", value: h), URLQueryItem(name: "status", value: ok ? "success" : "failed")]
                    #if os(macOS)
                    if let u = c.url { await MainActor.run { _ = NSWorkspace.shared.open(u) } }
                    #endif
                }
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

    /// A node reward this wallet received (iPhone Home shows those as one line).
    var isNodeReward: Bool { kind == .received && title.hasPrefix("Proof reward") }
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

    /// All 18 decimals (records, exports).
    static func exact(_ wei: String) -> String {
        let padded = String(repeating: "0", count: max(0, 19 - wei.count)) + wei
        let whole = padded.dropLast(18).drop(while: { $0 == "0" })
        return "\(whole.isEmpty ? "0" : String(whole)).\(padded.suffix(18))"
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

/// Where a voting-node registration stands, for the Network page.
enum RegistrationState: Equatable {
    case working
    case failed(String)
}

/// A payment asked for by a link; shown in the send sheet for approval.
struct PaymentRequest: Equatable {
    let to: String
    let amount: String
    let memo: String?
    let callback: URL?
}

/// A contract call asked for by a link; shown decoded for approval.
struct CallRequest: Equatable {
    let to: String
    let value: String
    let data: String
    let gas: UInt64
    let memo: String?
    let origin: String?
    let callback: URL?

    /// What the call does, when its 4-byte selector is a well-known one.
    var method: String {
        if to.isEmpty { return "Deploy a contract (\((data.count - 2) / 2) bytes)" }
        let known: [String: String] = [
            "0xa9059cbb": "Token transfer", "0x095ea7b3": "Token approval (allows spending)", "0x23b872dd": "Token transfer from",
            "0x38ed1739": "Swap tokens", "0xe8e33700": "Add liquidity", "0xbaa2abde": "Remove liquidity",
        ]
        return known[String(data.prefix(10)).lowercased()] ?? "Contract call \(data.prefix(10))"
    }
}

/// A page asking to know this wallet's address.
struct ConnectRequest: Equatable {
    let origin: String
    let callback: URL
}
