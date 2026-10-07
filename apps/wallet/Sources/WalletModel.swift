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
    @Published var connectionInfo = String(localized: "Looking for the network…")
    @Published private(set) var developmentNetwork = false
    @Published private(set) var developmentPort: UInt16 = 18546
    @Published private(set) var networkChainId: UInt64 = 0
    @Published var validators: UInt32 = 4
    @Published var address = ""
    @Published var account: VerifiedAccount?
    @Published var status: ChainStatus?
    var scheduledUpgrades: [NetworkUpgrade] {
        guard let status else { return [] }
        return NetworkUpgrade.parse(status.upgradesJson, height: status.height)
    }
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
    /// Local agent receipt link (`aether://tx?hash=...`), shown in Security.
    @Published var agentTransactionHash: String?
    /// Voting-node registration in progress or failed (nil: idle or done).
    @Published var registration: RegistrationState?
    @Published var busy = false
    @Published var log: [String] = []
    @Published var sendTo = ""
    @Published var sendAmount = "1"
    /// "새 가격으로 다시 보내기" (contracts-live bug #5): the dropped transfer
    /// the send sheet is re-sending, at its nonce. Applies only while the
    /// sheet's recipient and amount are still exactly that transfer's.
    @Published var resend: ActivityItem.Resend?
    /// Bumped to open the send sheet for a resend.
    @Published var resendRequest: UUID?
    @Published var recoveryCode = ""
    @Published var keyLabel = String(localized: "Key in the Secure Enclave")
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
    @Published private(set) var linkedWallets: [String] = []
    @Published private(set) var activityHistoryStart: UInt64?
    @Published private(set) var olderActivityAvailable = false
    /// Why the history could not be read from the node (nil: it answered).
    /// The history page says it in plain words instead of showing a stale
    /// or empty list as if it were complete. (Set from `refreshChainActivity`
    /// and the design preview.)
    @Published var historyFailure: HistoryFailure?
    /// "Where does my balance come from?": the exact-wei itemization of this
    /// account's own history against the certificate-verified balance. Nil
    /// until the first page of history has landed (and on a network too old
    /// to answer it — the Activity page then shows no breakdown card).
    @Published private(set) var breakdown: BalanceBreakdown?
    /// A recovery someone started on THIS account (cancel it if it was not you).
    @Published var incomingRecovery: RecoveryStatus?
    /// A recovery this device proposed for another account, waiting for its delay.
    @Published var outgoingRecovery: PendingRecovery?
    /// ERC-20 tokens with a non-zero balance (read from the node, not light-client verified).
    @Published var tokens: [TokenHolding] = []
    /// The token chosen in the Send sheet (nil: an AETH transfer, as before).
    @Published var sendToken: TokenHolding?
    /// The user's own token display choices, per chain (this device only —
    /// everything else in the display policy is derived from the wallet's own
    /// on-chain history and the bundled list).
    @Published private(set) var tokenChoices = TokenChoices()
    /// When `tokens` was last read (nil: never, for this account).
    @Published var tokensUpdated: Date?
    @Published var tokensError: String?
    /// Sites the Explore tab is connected to (per-origin grants, revocable in
    /// Security). Kept per device, like the other Explore-tab state.
    @Published private(set) var sitePermissions = SitePermissionStore()
    /// Since when the chain has made no new block (nil while it moves). Light
    /// verification needs the next block and a recent certificate, so a paused chain
    /// cannot verify: the last verified balance stays on screen meanwhile.
    @Published var chainPausedSince: Date?
    private func setVerifyError(_ e: String?) {
        if verifyError != e { verifyError = e }
    }

    // MARK: Explore tab (the in-app browser)

    /// While this is true, the Explore tab's provider answers nothing — reads
    /// included — exactly as the extension's vault does while locked.
    var exploreLocked: Bool { enclave == nil || keyError != nil }

    /// The port this Mac's own node serves JSON-RPC on (the dev network gets
    /// its own port). The Explore tab's unverified reads go here.
    var nodeRpcPort: UInt16 { developmentNetwork ? developmentPort : 18545 }

    /// The address a site may see, nil unless this exact origin was granted
    /// the account the wallet holds right now (a switched account disconnects
    /// every site — the grant names an address, not "whatever is active").
    func connectedSiteAddress(origin: String) -> String? {
        guard !exploreLocked else { return nil }
        return sitePermissions.connectedAddress(origin: origin, current: address)
    }

    /// Remember (or replace) a site's grant after the user approved the sheet.
    func grantSitePermission(origin: String, address: String) {
        sitePermissions.grant(origin: origin, address: address)
        sitePermissions.save()
    }

    /// Forget one site's grant (the site's next request asks again).
    func revokeSitePermission(origin: String) {
        sitePermissions.revoke(origin: origin)
        sitePermissions.save()
    }

    /// Forget every site (Security's "Disconnect all").
    func revokeAllSitePermissions() {
        sitePermissions.revokeAll()
        sitePermissions.save()
    }

    /// Sign and submit a page's transaction after the sheet was approved.
    /// A plain transfer reuses the send sheet's FeeChanged flow: `shownFeeWei`
    /// is the maximum the sheet displayed, and a rise since then refuses the
    /// send instead of silently signing above it. A contract call runs with
    /// the gas the page asked for (its default when it asked for none).
    /// Returns the tx hash once submitted (the page watches it with
    /// aether_getReceipt); a refusal comes back as text and nothing is signed.
    func sendPageTransaction(_ tx: PageTransaction, origin: String, title: String,
                             shownFeeWei: String?) async -> (hash: String?, refusal: String?) {
        guard let enclave else { return (nil, String(localized: "The wallet key is not ready yet.")) }
        let pk = enclave.publicKey
        let validatorsNow = validators
        let action = CallDescribe.action(to: tx.to, data: tx.data)
        let who = tx.to.isEmpty ? "a new contract" : Short.address(tx.to)
        let item = ActivityItem(kind: .sent, title: "\(action) at \(origin)",
                                amount: Double(Wei.format(tx.valueWei)).map { -$0 },
                                recipients: tx.to.isEmpty ? [] : [tx.to.lowercased()])
        do {
            let prepared: PreparedTx
            if tx.isPlainTransfer {
                prepared = try prepareTransfer(p256PublicKey: pk, to: tx.to, valueWei: tx.valueWei,
                                               shownFeeWei: shownFeeWei, validators: validatorsNow)
            } else {
                let gas: UInt64 = tx.gas == 0 ? 3_000_000 : tx.gas
                prepared = try prepareCall(p256PublicKey: pk, to: tx.to, valueWei: tx.valueWei,
                                           dataHex: tx.data, gasLimit: gas)
            }
            let sig = try enclave.sign(prepared.signingMessage)   // Secure Enclave, may prompt
            let h = try submitSigned(envelopeJson: prepared.envelopeJson, signature: sig, p256PublicKey: pk)
            note("\(title): \(action) to \(who) submitted \(h.prefix(14))…")
            // The page gets its hash now; finality lands in the activity feed.
            Task.detached { await self.track(h, label: "\(title): \(action) to \(who)", item: item) }
            return (h, nil)
        } catch {
            let refusal = WalletModel.ffiMessage(error)
            note("\(title) was not sent: \(refusal)")
            return (nil, refusal)
        }
    }

    private var refreshes = 0
    private var networkGeneration: UInt64 = 0
    private var lastHeight: UInt64?
    private var heightChangedAt: Date?
    private var tokenScanRunning = false
    private var activityLoading = false
    private var lastActivityHeight: UInt64?
    private var activityCursors: [String: String] = [:]
    private var activityExhausted: Set<String> = []
    /// This account's own history rows as the node reported them, merged
    /// across pages (the feed below is display data; the balance breakdown
    /// and the full CSV are itemized from these). Keyed by tx and address.
    private var chainRows: [String: ChainHistoryEntry] = [:]
    private var pendingBalanceRises: [(height: UInt64, wei: String)] = []
    /// Tokens known on this chain, kept between scans (also feeds the look-alike
    /// and provenance checks with official metadata).
    private(set) var tokenCatalog = TokenCatalog()
    private var tokenChoicesForChain: UInt64?

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
            keyLabel = acct.isSecureEnclave ? String(localized: "Key in the Secure Enclave") : String(localized: "Simulator: software key (no Secure Enclave)")
            note(acct.isSecureEnclave ? "Secure Enclave key ready. Signing asks for Touch ID / Face ID or your passcode." : "Simulator: software key (no Secure Enclave here). Use a real device for hardware-bound keys.")
        } catch {
            let locked = (error as NSError).code == Int(errSecInteractionNotAllowed)
            if case EnclaveAccount.KeyError.keyUnavailable = error {
                // The wallet exists but cannot be opened yet; retried from `refresh`.
                keyError = String(localized: "Unlock this device to open your wallet. Your wallet is safe.")
            } else {
                keyError = locked ? String(localized: "Unlock this device to create your wallet key.") : String(localized: "Could not create the wallet key: \(error.localizedDescription)")
            }
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
            let bundled = try JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any] ?? [:]
            let defaultChain = (bundled["chain_id"] as? NSNumber)?.uint64Value ?? 0
            let dev = UserDefaults.standard.bool(forKey: "useDevelopmentNetwork") && UserDefaults.standard.bool(forKey: "developerMode")
            if !dev { UserDefaults.standard.set(false, forKey: "useDevelopmentNetwork") }
            let savedPort = UserDefaults.standard.integer(forKey: "developmentNetworkPort")
            let port = (1024...65535).contains(savedPort) ? UInt16(savedPort) : 18546
            let devValidators = Array(((bundled["validators"] as? [[String: Any]]) ?? []).prefix(4))
            let devJSON = try JSONSerialization.data(withJSONObject: ["chain_id": 7777, "validators": devValidators, "devnet": true])
            validators = try configureNetwork(networkJson: dev ? String(decoding: devJSON, as: UTF8.self) : json)
            networkChainId = dev ? 7777 : defaultChain
            developmentNetwork = dev
            developmentPort = port
            if dev { useLocalNode(port: port) }
            let id = dev ? "local devnet keys" : (bundled["identity"] as? String ?? "missing identity")
            note("Network: \(validators) validators · committee key \(id.prefix(16))…")
        } catch {
            note("network.json rejected: \(error)")
        }
    }

    /// The health check's L2 (docs/design/32-health-signal.md §4.2): stuck on
    /// "connecting" for 90 s, or the person pressed [다시 시도]. Configuring
    /// the same bundled network again drops the cached client, so the next
    /// read looks the validators and follower Macs up afresh (DHT, new remote
    /// node list) instead of waiting on the ones found before the network
    /// came up. The verified-height floor starts over exactly as at launch;
    /// the committee key is the bundled one either way. The Mac's own node
    /// route is restored by the caller (`NodeController.refreshWalletRoute`).
    func rediscover() {
        guard !busy, !developmentNetwork else { return }
        note("Looking the network up again")
        pinCommittee()
    }

    func selectNetwork(development: Bool, port: UInt16 = 18546) {
        guard !development || UserDefaults.standard.bool(forKey: "developerMode") else { return }
        if development == developmentNetwork && (!development || port == developmentPort) { return }
        guard !busy else {
            UserDefaults.standard.set(developmentNetwork, forKey: "useDevelopmentNetwork")
            if developmentNetwork { UserDefaults.standard.set(true, forKey: "developerMode") }
            note("Wait for the pending transaction before switching networks.")
            return
        }
        save()
        networkGeneration &+= 1
        UserDefaults.standard.set(development, forKey: "useDevelopmentNetwork")
        if development { UserDefaults.standard.set(Int(port), forKey: "developmentNetworkPort") }
        pinCommittee()
        status = nil
        account = nil
        blocks = []
        verifyError = nil
        verifyFailingSince = nil
        networkOutdated = false
        chainPausedSince = nil
        lastHeight = nil
        lastActivityHeight = nil
        activityCursors = [:]
        activityExhausted = []
        chainRows = [:]
        breakdown = nil
        activityHistoryStart = nil
        olderActivityAvailable = false
        historyFailure = nil
        pendingBalanceRises = []
        activityLoading = false
        tokenScanRunning = false
        tokenChoicesForChain = nil
        tokensUpdated = nil
        tokensError = nil
        loadSaved()
        loadTokens()
        loadTokenChoices(chain: networkChainId)
        refresh()
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
                await self.track(h, label: "Recovery key set to \(code.prefix(12))…", item: ActivityItem(kind: .security, title: String(localized: "Recovery device added"), amount: nil))
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
                await self.track(h, label: "Recovery words registered as a recovery key", item: ActivityItem(kind: .security, title: String(localized: "Recovery words added"), amount: nil))
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
                                 item: ActivityItem(kind: .security, title: String(localized: "Recovery started for \(Short.address(lost))"), amount: nil))
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
                                 item: ActivityItem(kind: .security, title: String(localized: "Recovery started for \(Short.address(lost))"), amount: nil))
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
                let ok = await self.track(h, label: "Recovered \(Wei.format(pending.request.valueWei)) \(Brand.networkCoinTicker) from \(pending.request.lost.prefix(10))…",
                                 item: ActivityItem(kind: .received, title: String(localized: "Recovered from \(Short.address(pending.request.lost))"),
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
                await self.track(h, label: "Cancelled a recovery of this account", item: ActivityItem(kind: .security, title: String(localized: "Recovery cancelled"), amount: nil))
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
                await self.track(h, label: "Removed every recovery key", item: ActivityItem(kind: .security, title: String(localized: "Recovery keys removed"), amount: nil))
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
            registration = .failed(String(localized: "The node is still starting. Try again in a minute."))
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
                let ok = await self.track(h, label: "This Mac is registered as a voting node", item: ActivityItem(kind: .security, title: String(localized: "Mac joined as a voting node"), amount: nil))
                await MainActor.run { self.registration = ok ? nil : .failed(String(localized: "The registration did not go through. Try again.")) }
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
        let addr = address, n = validators, generation = networkGeneration
        refreshes += 1
        // Recovery status needs several proofs; every 30 s is enough to warn within the delay.
        let checkRecovery = refreshes % 15 == 1 && !addr.isEmpty
        Task.detached {
            if checkRecovery, let rs = try? recoveryStatus(account: addr, validators: n) {
                await MainActor.run { if self.networkGeneration == generation { self.incomingRecovery = rs.pending ? rs : nil } }
            }
            let st = try? chainStatus()
            let conn = connection()
            if let st { await MainActor.run { if self.networkGeneration == generation { self.loadTokenChoices(chain: st.chainId) } } }
            let bl = (try? recentBlocks(n: 24)) ?? []
            var acc: VerifiedAccount?
            var err: String?
            if !addr.isEmpty {
                do { acc = try verifiedAccount(address: addr, validators: n) } catch { err = "\(error)" }
            }
            let verified = acc, readError = err
            await MainActor.run {
                guard self.networkGeneration == generation else { return }
                // Published only when something actually changed: an unchanged set
                // would still invalidate every view watching this model (the whole
                // window), which lands right on top of live resizes.
                if self.status != st { self.status = st }
                if self.connectionInfo != conn { self.connectionInfo = conn }
                if self.blocks != bl { self.blocks = bl }
                if let acc = verified {
                    if let old = self.account, acc.stateHeight == old.stateHeight + 1,
                       let rise = ChainActivity.rise(acc.balanceWei, over: old.balanceWei) {
                        self.pendingBalanceRises.append((acc.stateHeight, rise))
                    }
                    if self.account != acc { self.account = acc }
                    if self.verifyError != nil { self.verifyError = nil }
                    self.record(balanceWei: acc.balanceWei)
                }
                // A node that is still catching up serves an older block than one this wallet
                // already verified; the FFI refuses it (finalized blocks never go back). That
                // is not an error to show: keep the newer verified balance.
                let behindNode = readError?.contains("finalized blocks never go back") == true && self.account != nil
                if st == nil { self.setVerifyError(String(localized: "The network cannot be reached yet.")) } else if let readError, !behindNode { self.setVerifyError(readError) }
                self.trackChainProgress(st, blocks: bl)
                self.trackVerification()
                self.refreshTokens()
                if let st, !self.activityLoading,
                   (st.height != self.lastActivityHeight || self.refreshes % 15 == 1) {
                    self.lastActivityHeight = st.height
                    self.refreshChainActivity()
                }
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
        let owner = address, catalogKey = "tokenCatalog.\(chain)", catalog = tokenCatalog, generation = networkGeneration
        Task.detached {
            let result = Result { try TokenScanner.scan(owner: owner, sources: sources, catalog: catalog, read: { try ethCall(to: $0, dataHex: $1) }) }
            await MainActor.run {
                guard self.networkGeneration == generation else { return }
                self.tokenScanRunning = false
                guard owner == self.address else { return }
                switch result {
                case .success(let (cat, held)):
                    UserDefaults.standard.set(try? JSONEncoder().encode(cat), forKey: catalogKey)
                    self.tokenCatalog = cat
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

    private var tokensKey: String { "tokenHoldings.\(networkChainId).\(address)" }

    /// The last token balances read for this account, shown until the next read.
    private func loadTokens() {
        if networkChainId == 7780, UserDefaults.standard.object(forKey: tokensKey) == nil,
           let old = UserDefaults.standard.data(forKey: "tokenHoldings.\(address)") {
            UserDefaults.standard.set(old, forKey: tokensKey)
        }
        tokens = UserDefaults.standard.data(forKey: tokensKey).flatMap { try? JSONDecoder().decode([TokenHolding].self, from: $0) } ?? []
        tokenCatalog = UserDefaults.standard.data(forKey: "tokenCatalog.\(networkChainId)")
            .flatMap { try? JSONDecoder().decode(TokenCatalog.self, from: $0) } ?? TokenCatalog()
    }

    // MARK: token display policy (docs/research/token-spam-2026.md §6)

    /// Addresses this wallet has sent to before — its own signed history, read
    /// back from the tracked activity. Derived, never stored.
    var sentAddresses: Set<String> {
        Set(activity.filter { $0.owner == nil || $0.owner?.lowercased() == address.lowercased() }
            .flatMap { $0.recipients ?? [] }.map { $0.lowercased() })
    }

    /// Tokens this wallet's own signed transactions touched (a send, an
    /// approval, a contract call to the token). Derived, never stored.
    var touchedTokens: Set<String> {
        Set(activity.filter { $0.owner == nil || $0.owner?.lowercased() == address.lowercased() }
            .compactMap(\.token).map { $0.lowercased() })
    }

    /// Official tokens of this chain (the bundled seed list + wrapped AETH).
    var officialTokenAddresses: Set<String> {
        guard let s = TokenSources.bundled(chainId: status?.chainId ?? 0) else { return [] }
        return Set((s.seed + [s.waeth].compactMap { $0 }).map { $0.lowercased() })
    }

    /// Their symbols and names, for the look-alike warning (the native coin first).
    var officialSymbols: [(symbol: String, name: String)] {
        let chain = status?.chainId ?? Brand.networkChainId
        // The legacy testnet's old coin label stays on the look-alike list:
        // a token calling itself "AETH" there still mimics the native coin.
        let legacy: [(symbol: String, name: String)] = chain == Brand.legacyTestnetChainId ? [("AETH", "Test AETH")] : []
        return [(Brand.coinTicker(chainId: chain), Brand.coinName(chainId: chain))] + legacy
            + officialTokenAddresses.sorted().compactMap { tokenCatalog.tokens[$0].map { ($0.symbol, $0.name) } }
    }

    /// Which holdings belong in the main Assets list and which in the collapsed
    /// Unverified section (out of any total). AETH and tokens this wallet
    /// acquired or moved by its own signed action are main-listed; what only
    /// arrived by someone else's transfer is not.
    var tokenSections: (main: [TokenHolding], unverified: [TokenHolding]) {
        TokenDisplayPolicy(touched: touchedTokens, official: officialTokenAddresses,
                           hidden: tokenChoices.hidden, shown: tokenChoices.shown).split(tokens)
    }

    /// Hide a token (moved out of the main list on this device), or show it again.
    func setTokenHidden(_ address: String, _ hidden: Bool) {
        let a = address.lowercased()
        if hidden {
            tokenChoices.hidden.insert(a)
            tokenChoices.shown.remove(a)
        } else {
            tokenChoices.hidden.remove(a)
        }
        saveTokenChoices()
    }

    /// Move a token someone else sent in into the main list (this device).
    func showTokenInMainList(_ address: String) {
        let a = address.lowercased()
        tokenChoices.shown.insert(a)
        tokenChoices.hidden.remove(a)
        saveTokenChoices()
    }

    private func saveTokenChoices() {
        guard let chain = status?.chainId else { return }
        UserDefaults.standard.set(try? JSONEncoder().encode(tokenChoices), forKey: "tokenChoices.\(chain)")
    }

    private func loadTokenChoices(chain: UInt64) {
        guard tokenChoicesForChain != chain else { return }
        tokenChoicesForChain = chain
        tokenChoices = UserDefaults.standard.data(forKey: "tokenChoices.\(chain)")
            .flatMap { try? JSONDecoder().decode(TokenChoices.self, from: $0) } ?? TokenChoices()
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
        // The rename kept every existing aether:// payment link alive: both
        // schemes stay registered and both are parsed the same way.
        guard url.scheme == "eastsea" || url.scheme == "aether",
              let c = URLComponents(url: url, resolvingAgainstBaseURL: false) else { return }
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
        case "tx":
            guard let hash = q["hash"], hash.count == 66, hash.hasPrefix("0x"), hash.dropFirst(2).allSatisfy(\.isHexDigit) else {
                return note("Ignored an invalid transaction link")
            }
            agentTransactionHash = hash
        default:
            note("Ignored an unknown EastSea link")
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
        // A call to a known token contract (an approval, a mint…) marks it as
        // moved by this wallet's own action, for the display policy.
        let token = r.to.isEmpty ? nil : tokenCatalog.tokens[r.to.lowercased()].map { _ in r.to.lowercased() }
        busy = true
        Task.detached {
            do {
                let prepared = try prepareCall(p256PublicKey: pk, to: r.to, valueWei: wei, dataHex: r.data, gasLimit: r.gas)
                let sig = try enclave.sign(prepared.signingMessage)
                let h = try submitSigned(envelopeJson: prepared.envelopeJson, signature: sig, p256PublicKey: pk)
                let title = r.to.isEmpty ? String(localized: "Deployed a contract") : String(localized: "Called \(Short.address(r.to))")
                let ok = await self.track(h, label: title,
                                          item: ActivityItem(kind: .sent, title: title, amount: nil, token: token))
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
        guard developmentNetwork && UserDefaults.standard.bool(forKey: "developerMode") else { return }
        let addr = address
        busy = true
        Task.detached {
            do {
                let h = try devnetFaucet(to: addr, valueWei: Wei.from(aeth: "10")!)
                await self.track(h, label: "Faucet 10 \(Brand.networkCoinTicker)", item: ActivityItem(kind: .received, title: String(localized: "Test \(Brand.networkCoinTicker) from the faucet"), amount: 10))
            } catch { await MainActor.run { self.note("Faucet failed: \(error)"); self.busy = false } }
        }
    }

    /// Send the sheet's transfer. `shownFeeWei` is the fee maximum the send
    /// sheet displayed (pre-audit 7, M1): when the same quote computed at
    /// signature time would cost more, the FFI refuses with `FeeChanged` and
    /// this returns the reason instead of sending — the sheet stays open,
    /// re-quotes, and asks the user to confirm the new maximum. Returns nil
    /// when the transaction was signed and submitted.
    @discardableResult
    func send(shownFeeWei: String? = nil) async -> String? {
        guard let enclave else { return String(localized: "The wallet key is not ready yet.") }
        // A payment link is sent exactly as it asked; otherwise the form's values.
        let (toText, amountText) = paymentRequest.map { ($0.to, $0.amount) } ?? (sendTo, sendAmount)
        guard let wei = Wei.from(aeth: amountText) else { note("Invalid amount"); return String(localized: "Check the amount: it is not a number this wallet can send.") }
        // One or more recipients (comma/space separated); each gets the amount.
        let recipients = toText.split(whereSeparator: { $0 == "," || $0.isWhitespace }).map(String.init).filter { !$0.isEmpty }
        guard !recipients.isEmpty else { return String(localized: "Add a recipient first.") }
        let pk = enclave.publicKey
        let callback = paymentRequest?.callback
        // Read main-actor state before detaching; the closure only signs.
        let validatorsNow = validators
        let resending = paymentRequest == nil ? resend : nil
        busy = true
        let refused: String? = await Task.detached { [weak self] () -> String? in
            guard let self else { return nil }
            do {
                let prepared: PreparedTx
                let label: String
                let each = Double(Wei.format(wei)) ?? 0
                let who = recipients.count == 1 ? Short.address(recipients[0]) : String(localized: "\(recipients.count) people")
                var item = ActivityItem(kind: .sent, title: String(localized: "Sent to \(who)"), amount: -each * Double(recipients.count),
                                        recipients: recipients.map { $0.lowercased() })
                if recipients.count == 1 {
                    // A resend of a dropped transfer signs its nonce again with a
                    // fresh fee (bug #5), so at most one of the two can ever run.
                    if let r = resending, r.matches(to: recipients[0], valueWei: wei) {
                        prepared = try prepareTransferAt(p256PublicKey: pk, to: recipients[0], valueWei: wei,
                                                         shownFeeWei: shownFeeWei, validators: validatorsNow, nonce: r.nonce)
                    } else {
                        prepared = try prepareTransfer(p256PublicKey: pk, to: recipients[0], valueWei: wei,
                                                       shownFeeWei: shownFeeWei, validators: validatorsNow)
                    }
                    item.resend = ActivityItem.Resend(to: recipients[0], valueWei: wei, nonce: prepared.nonce)
                    label = "Sent \(Wei.format(wei)) \(Brand.networkCoinTicker) (nonce \(prepared.nonce))"
                } else {
                    // All payments in one tx: one signature, all or nothing (EIP-7702 batch).
                    prepared = try prepareBatch(p256PublicKey: pk, payments: recipients.map { Payment(to: $0, valueWei: wei) },
                                                shownFeeWei: shownFeeWei)
                    label = "Paid \(recipients.count) recipients \(Wei.format(wei)) \(Brand.networkCoinTicker) each with one signature (nonce \(prepared.nonce))"
                }
                let sig = try enclave.sign(prepared.signingMessage)   // Secure Enclave, may prompt
                let h = try submitSigned(envelopeJson: prepared.envelopeJson, signature: sig, p256PublicKey: pk)
                // Submitted: the sheet closes now and the activity row follows
                // the transaction — up to the network's 10-minute mempool
                // lifetime, with its reason while it waits (bug #5).
                let tracked = item
                Task.detached { [weak self] in
                    guard let self else { return }
                    let ok = await self.track(h, label: label, item: tracked)
                    // A web page that asked for this payment hears back (https only).
                    if let back = callback, var c = URLComponents(url: back, resolvingAgainstBaseURL: false) {
                        c.queryItems = (c.queryItems ?? []) + [URLQueryItem(name: "tx", value: h), URLQueryItem(name: "status", value: ok ? "success" : "failed")]
                        #if os(macOS)
                        if let u = c.url { await MainActor.run { _ = NSWorkspace.shared.open(u) } }
                        #endif
                    }
                }
                return nil
            } catch {
                await MainActor.run { self.note("Send failed: \(error)"); self.busy = false }
                return WalletModel.ffiMessage(error)
            }
        }.value
        if refused == nil {
            // Only a submitted send consumes the payment request; a refusal
            // keeps it so the sheet can ask again under the fresh quote.
            await MainActor.run { self.paymentRequest = nil; self.resend = nil }
        }
        return refused
    }

    /// Send the frozen, confirmed intent (audits R2-2/R2-5, ported from the
    /// extension): the review screen froze what it showed — recipient, token,
    /// exact base-unit count — and this re-checks that intent against the state
    /// NOW and refuses when anything moved. It never re-parses an amount text
    /// under whatever decimals are stored later. Returns nil when the send was
    /// started; otherwise the refusal to show (the send sheet stays open).
    func sendTokenTx(_ intent: SendIntent) -> String? {
        guard let enclave else { return String(localized: "The wallet key is not ready yet.") }
        guard let chain = status?.chainId else {
            note("Not sent — the network is not ready yet")
            return String(localized: "The network is not ready yet.")
        }
        let known = KnownTokens.knownToken(chainId: chain, address: intent.token.address)
        let current = tokens.first { $0.token.address == intent.token.address }
        let data: String
        do {
            data = try SendIntent.check(intent, current: current?.token, known: known)
        } catch let e as TokenGuardError {
            note("Not sent — \(e.errorDescription ?? "\(e)")")
            return e.errorDescription
        } catch {
            note("Not sent — \(error)")
            return "\(error)"
        }
        guard let holding = current, WeiMath.compare(intent.baseUnits, holding.balance) <= 0 else {
            note("Not sent — this wallet now holds less than the confirmed amount")
            return String(localized: "This wallet now holds less than the confirmed amount.")
        }
        let symbol = TokenDenomination.of(chainId: chain, address: intent.token.address, claimed: current?.token).symbol ?? "?"
        let shown = TokenAmount.exact(intent.baseUnits, decimals: intent.token.decimals)
        let item = ActivityItem(kind: .sent, title: String(localized: "Sent \(shown) \(symbol) to \(TokenLabel.short(intent.recipient))"),
                                amount: nil, recipients: [intent.recipient.lowercased()], token: intent.token.address)
        let pk = enclave.publicKey
        sendToken = nil
        busy = true
        Task.detached {
            do {
                let prepared = try prepareCall(p256PublicKey: pk, to: intent.token.address, valueWei: "0", dataHex: data, gasLimit: 100_000)
                let sig = try enclave.sign(prepared.signingMessage)   // Secure Enclave, may prompt
                let h = try submitSigned(envelopeJson: prepared.envelopeJson, signature: sig, p256PublicKey: pk)
                await self.track(h, label: "Sent \(shown) \(symbol) to \(TokenLabel.short(intent.recipient)) (nonce \(prepared.nonce))", item: item)
                await MainActor.run { self.refreshTokens(force: true) }
            } catch { await MainActor.run { self.note("Token send failed: \(error)"); self.busy = false } }
        }
        return nil
    }

    /// Dry-run a send before it is signed: the transfer as an `eth_call` from
    /// this account, so a honeypot (a token that reverts on transfer) or a
    /// contract that cannot receive plain AETH is refused with its reason.
    /// Never stored, never signed — these checks read public chain data and
    /// settings on this device. Nothing new is written on chain.
    static func dryRun(from: String, to: String, valueWei: String, data: String) async -> DryRunOutcome {
        await SendDryRun.check(from: from, to: to, valueWei: valueWei, data: data, plainCall: { recipient, dataHex in
            do {
                return try ethCall(to: recipient, dataHex: dataHex)
            } catch {
                throw DryRunError(reason: ffiMessage(error))
            }
        })
    }

    /// The readable text of an FFI error (its message, not the Swift case
    /// reflection `WalletError.Network(message: …)` would print).
    nonisolated static func ffiMessage(_ e: Error) -> String {
        switch e {
        // The core's own words are English and technical: a Korean screen
        // gets one plain sentence per kind instead of a mixed-language line.
        case WalletError.Network(let m):
            return AppLanguage.korean ? "네트워크에 연결하지 못했어요. 잠시 뒤 다시 해 주세요." : m
        case WalletError.Invalid(let m):
            return AppLanguage.korean ? "입력한 내용이 올바르지 않아요. 주소와 금액을 다시 확인해 주세요." : m
        case WalletError.Rejected(let m):
            return AppLanguage.korean ? "네트워크가 이 거래를 받지 않았어요. 잔액과 수수료를 확인하고 다시 해 주세요." : m
        case WalletError.Verification(let m):
            return AppLanguage.korean ? "이 기기에서 확인하지 못했어요. 잠시 뒤 다시 해 주세요." : m
        case WalletError.FeeChanged(let m):
            return AppLanguage.korean
                ? "네트워크 수수료가 바뀌었어요. 보내지 않았으니 새 수수료를 확인하고 다시 보내 주세요."
                : "The network fee changed — \(m). Nothing was sent; check the new fee and send again."
        default:
            return (e as? LocalizedError)?.errorDescription ?? "\(e)"
        }
    }

    /// Wait for finality; returns whether the tx succeeded. While it is not
    /// in a block the row says why, in plain words (contracts-live bug #5):
    /// what a pending tx waits for, or why the network dropped it — never a
    /// silent "Failed". Waits out the network's 10-minute mempool lifetime.
    @discardableResult
    private func track(_ hash: String, label: String, item: ActivityItem) async -> Bool {
        await MainActor.run {
            self.note("\(label) submitted \(hash.prefix(14))…")
            self.activity.insert(item.with(state: .pending).with(hash: hash), at: 0)
            self.save()
        }
        let start = Date()
        var shownWhy: String?
        var unknownSince: Date?
        while Date().timeIntervalSince(start) < Self.trackLimit {
            let st = try? txStatus(txHash: hash)
            switch st?.state {
            case "included":
                guard let st, let r = st.receipt else { break }
                await MainActor.run {
                    self.settle(item.id, state: r.success ? .done : .failed, why: r.success ? nil : TxStatusText.sentence(st))
                    self.note("\(label) finalized in block \(r.height) (\(r.success ? "success" : "failed"), gas \(r.gasUsed)\(r.stateFeeWei != "0" ? ", state fee \(Amount.fee(r.stateFeeWei))" : ""))")
                    self.busy = false
                    self.refresh()
                }
                return r.success
            case "dropped":
                guard let st else { break }
                await MainActor.run {
                    self.settle(item.id, state: .failed, why: TxStatusText.sentence(st), canResend: st.canResend)
                    self.note("\(label): not included — \(st.detail)")
                    self.busy = false
                }
                return false
            case "pending":
                unknownSince = nil
                if let st, st.reason != nil, TxStatusText.sentence(st) != shownWhy {
                    let why = TxStatusText.sentence(st)
                    shownWhy = why
                    await MainActor.run { self.explain(item.id, why: why); self.note("\(label): \(st.detail)") }
                }
            default:
                // A node that has not heard of it yet, or a read that failed.
                unknownSince = unknownSince ?? Date()
                if let since = unknownSince, Date().timeIntervalSince(since) > 60 {
                    let why = st.map(TxStatusText.sentence) ?? TxStatusText.unknown()
                    await MainActor.run { self.settle(item.id, state: .failed, why: why); self.note("\(label): \(st?.detail ?? "no answer from the network")"); self.busy = false }
                    return false
                }
            }
            let waited = Date().timeIntervalSince(start)
            if waited > 30 { await MainActor.run { self.busy = false } }
            try? await Task.sleep(nanoseconds: waited < 30 ? 500_000_000 : 3_000_000_000)
        }
        await MainActor.run {
            self.settle(item.id, state: .failed, why: shownWhy ?? TxStatusText.timedOut())
            self.note("\(label): not finalized after \(Int(Self.trackLimit / 60)) minutes")
            self.busy = false
        }
        return false
    }

    /// How long `track` follows a transaction: the node's mempool lifetime
    /// (10 minutes) plus a minute, so a drop is seen with its reason.
    private static let trackLimit: TimeInterval = 11 * 60

    /// "새 가격으로 다시 보내기": open the send sheet filled with the dropped
    /// transfer, so it goes through the normal quote and confirmation and is
    /// signed at the same nonce with a fresh fee. Never re-signs by itself.
    func beginResend(_ item: ActivityItem) {
        guard let r = item.resend, item.state == .failed else { return }
        paymentRequest = nil
        sendToken = nil
        sendTo = r.to
        sendAmount = Wei.exact(r.valueWei)
        resend = r
        resendRequest = UUID()
    }

    // MARK: dashboard data (kept per account in UserDefaults)

    private var historyKey: String { "balanceHistory.\(networkChainId).\(address)" }
    private var activityKey: String { "activity.\(networkChainId).\(address)" }
    private var linkedKey: String { "linkedWallets.\(address)" }

    private func loadSaved() {
        let d = UserDefaults.standard
        if networkChainId == 7780 {
            if d.object(forKey: historyKey) == nil, let old = d.data(forKey: "balanceHistory.\(address)") { d.set(old, forKey: historyKey) }
            if d.object(forKey: activityKey) == nil, let old = d.data(forKey: "activity.\(address)") { d.set(old, forKey: activityKey) }
        }
        history = d.data(forKey: historyKey).flatMap { try? JSONDecoder().decode([BalancePoint].self, from: $0) } ?? []
        activity = d.data(forKey: activityKey).flatMap { try? JSONDecoder().decode([ActivityItem].self, from: $0) } ?? []
        // Backups from the 7780 chain carry reward times as seconds where the
        // feed wants milliseconds ("last one 56y ago"): scale those back up so
        // the day a payment happened is the day it happened.
        activity = activity.map { item in
            let ts = item.date.timeIntervalSince1970
            return (ts > 0 && ts < Double(Timestamp.secondsEraBound) / 1_000) ? item.with(date: Date(timeIntervalSince1970: ts * 1_000)) : item
        }
        linkedWallets = d.stringArray(forKey: linkedKey) ?? []
        _ = sitePermissions.load(defaults: d)
    }

    private func save() {
        let d = UserDefaults.standard
        d.set(try? JSONEncoder().encode(history), forKey: historyKey)
        d.set(try? JSONEncoder().encode(Array(activity.prefix(500))), forKey: activityKey)
    }

    func addLinkedWallet(_ input: String) -> Bool {
        guard linkedWallets.count < 8,
              let address = ChainActivity.validLinkedAddress(input, own: self.address, existing: linkedWallets) else { return false }
        linkedWallets.append(address)
        UserDefaults.standard.set(linkedWallets, forKey: linkedKey)
        lastActivityHeight = nil
        refreshChainActivity()
        return true
    }

    func removeLinkedWallet(_ address: String) {
        linkedWallets.removeAll { $0.lowercased() == address.lowercased() }
        UserDefaults.standard.set(linkedWallets, forKey: linkedKey)
        activity.removeAll { $0.owner?.lowercased() == address.lowercased() }
        activityCursors.removeValue(forKey: address.lowercased())
        activityExhausted.remove(address.lowercased())
        lastActivityHeight = nil
        save()
    }

    func loadOlderActivity() { refreshChainActivity(older: true) }

    /// This account's own history rows as reported (oldest-first pages merged,
    /// newest data winning): what the balance breakdown and the CSV itemize.
    var ownHistoryRows: [ChainHistoryEntry] {
        chainRows.values.sorted { ($0.height, $0.txIndex) > ($1.height, $1.txIndex) }
    }

    /// The reward rows of that history, for the day-by-day grouping.
    var rewardHistoryRows: [ChainHistoryEntry] {
        ownHistoryRows.filter { $0.kind == "proof_reward" || $0.kind == "node_reward" }
    }

    private func refreshChainActivity(older: Bool = false) {
        guard !activityLoading, !address.isEmpty else { return }
        let own = address
        let addresses = ([own] + linkedWallets).filter { !older || activityCursors[$0.lowercased()] != nil }
        guard !addresses.isEmpty else { return }
        let cursors = activityCursors
        let generation = networkGeneration
        let sources = status.flatMap { TokenSources.bundled(chainId: $0.chainId) }
        let names = ChainNames(router: sources?.router, launchpad: sources?.launchpad,
                               tokenFactory: sources?.tokenFactory, waeth: sources?.waeth,
                               tokens: tokenCatalog.tokens.mapValues { ChainTokenName(symbol: $0.symbol, decimals: $0.decimals, origin: $0.origin) })
        activityLoading = true
        Task.detached {
            var pages: [(String, ChainHistoryPage)] = []
            var failures: [HistoryFailure] = []
            for address in addresses {
                do {
                    let json = try accountHistory(address: address, cursor: older ? cursors[address.lowercased()] : nil, limit: 200)
                    pages.append((address, try ChainHistoryPage.decode(json)))
                } catch {
                    // Keep the last successful view on screen, but say why the
                    // list may be incomplete (an old node, a node that is gone).
                    failures.append(HistoryFailure.classify(message: WalletModel.ffiMessage(error)))
                }
            }
            let fetchedPages = pages
            await MainActor.run {
                guard self.networkGeneration == generation else { return }
                self.activityLoading = false
                self.historyFailure = failures.first
                guard self.address == own else { return }
                var incomingHashes = Set<String>()
                for (address, page) in fetchedPages {
                    let key = address.lowercased()
                    if key != own.lowercased(), !self.linkedWallets.contains(where: { $0.lowercased() == key }) { continue }
                    if older || (self.activityCursors[key] == nil && !self.activityExhausted.contains(key)) {
                        if let cursor = page.nextCursor { self.activityCursors[key] = cursor }
                        else {
                            self.activityCursors.removeValue(forKey: key)
                            self.activityExhausted.insert(key)
                        }
                    }
                    self.activityHistoryStart = max(self.activityHistoryStart ?? 0, page.historyStart)
                    if !older {
                        let noticeKey = "incomingNotice.\(self.networkChainId).\(key)"
                        var notice = UserDefaults.standard.data(forKey: noticeKey).flatMap { try? JSONDecoder().decode(IncomingNoticeState.self, from: $0) }
                        if notice == nil {
                            notice = IncomingNoticeState(height: page.entries.map(\.height).max() ?? page.indexedHeight,
                                                         hashes: Set(page.entries.map { $0.txHash.lowercased() }))
                        } else {
                            for row in notice!.consume(page.entries) where incomingHashes.insert(row.txHash.lowercased()).inserted {
                                #if os(macOS)
                                LocalNotice.post(title: String(localized: "Payment received"), body: ChainActivity.title(row, names: names))
                                #endif
                            }
                        }
                        UserDefaults.standard.set(try? JSONEncoder().encode(notice), forKey: noticeKey)
                    }
                    for row in page.entries {
                        let hash = row.txHash.lowercased()
                        let title = ChainActivity.title(row, names: names)
                        let amount = ["native_transfer", "node_reward", "proof_reward"].contains(row.kind)
                            ? (Double(ChainActivity.units(row.valueWei)).map { row.direction == "out" ? -$0 : $0 }) : nil
                        // A missing time renders as nothing, never as 1970; a
                        // seconds value (pre-fix rows) is scaled back to ms.
                        var item = ActivityItem(date: Timestamp.date(fromMs: row.timestampMs) ?? .distantPast,
                                                kind: row.direction == "in" ? .received : (row.kind == "contract_call" || row.kind == "deploy" ? .security : .sent),
                                                title: title, amount: amount, state: row.success ? .done : .failed)
                        item.hash = row.txHash
                        item.source = String(localized: "From the node")
                        item.owner = row.address
                        if key == own.lowercased(), row.direction == "out" {
                            item.recipients = row.kind == "native_transfer" ? row.to.map { [$0.lowercased()] }
                                : row.kind == "erc20_transfer" ? row.tokens.filter { $0.from.lowercased() == key }.map { $0.to.lowercased() }
                                : nil
                            if row.kind == "erc20_transfer" || row.method == "0x095ea7b3" {
                                item.token = row.to?.lowercased()
                            }
                        }
                        let identity = ChainActivity.historyKey(hash: hash, address: key)
                        if let i = self.activity.firstIndex(where: {
                            guard let oldHash = $0.hash else { return false }
                            if let owner = $0.owner {
                                return ChainActivity.historyKey(hash: oldHash, address: owner) == identity
                            }
                            return key == own.lowercased() && oldHash.lowercased() == hash
                        }) {
                            item.id = self.activity[i].id
                            item.recipients = self.activity[i].recipients ?? item.recipients
                            item.token = self.activity[i].token ?? item.token
                            self.activity[i] = item
                        } else { self.activity.append(item) }
                    }
                    if key == own.lowercased() {
                        // The exact rows behind the itemization (merged across
                        // pages, newest data winning any identity collision).
                        for row in page.entries {
                            self.chainRows["\(row.txHash.lowercased()):\(key)"] = row
                        }
                        let oldest = page.entries.last?.height ?? page.historyStart
                        for rise in self.pendingBalanceRises where page.indexedHeight >= rise.height && rise.height >= oldest {
                            let matched = page.entries.contains { $0.height == rise.height && $0.direction == "in" }
                            if !matched {
                                var item = ActivityItem(kind: .received,
                                    title: String(localized: "Balance increased by \(ChainActivity.units(rise.wei)) \(Brand.networkCoinTicker) · block #\(String(rise.height))"),
                                    amount: Double(ChainActivity.units(rise.wei)), state: .done)
                                item.source = String(localized: "From the node")
                                item.owner = own
                                item.hash = "balance:\(rise.height):\(own.lowercased())"
                                if !self.activity.contains(where: { $0.hash == item.hash }) { self.activity.append(item) }
                            }
                        }
                        self.pendingBalanceRises.removeAll { page.indexedHeight >= $0.height && $0.height >= oldest }
                    }
                }
                self.olderActivityAvailable = !self.activityCursors.isEmpty
                self.activity.sort { $0.date > $1.date }
                self.save()
                self.refreshBreakdown()
            }
        }
    }

    /// Itemize where this account's balance comes from — every loaded history
    /// row summed against the certificate-verified balance, in exact wei inside
    /// the Rust core (`balance_sources`; integer math only). Runs after each
    /// history merge, including "load older", so the card's "not yet itemized"
    /// remainder shrinks as older pages land. A network too old to answer keeps
    /// the last good breakdown on screen instead of showing a wrong zero-sum.
    private func refreshBreakdown() {
        let own = address
        let key = own.lowercased()
        guard !key.isEmpty, !chainRows.isEmpty, let balance = account?.balanceWei else { return }
        let rows = Array(chainRows.values)
        let generation = networkGeneration
        let faucet = status?.faucet
        let waeth = status.flatMap { TokenSources.bundled(chainId: $0.chainId)?.waeth }
        Task.detached {
            let encoder = JSONEncoder()
            encoder.keyEncodingStrategy = .convertToSnakeCase
            guard let data = try? encoder.encode(rows),
                  let json = String(data: data, encoding: .utf8),
                  let answer = try? balanceSources(entriesJson: json, balanceWei: balance, faucet: faucet, waeth: waeth),
                  let parsed = try? BalanceBreakdown.decode(answer) else { return }
            await MainActor.run {
                guard self.networkGeneration == generation else { return }
                self.breakdown = parsed
            }
        }
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

    private func settle(_ id: UUID, state: ActivityItem.State, why: String? = nil, canResend: Bool = false) {
        if let i = activity.firstIndex(where: { $0.id == id }) {
            var settled = activity[i].with(state: state)
            settled.why = why
            // Only a drop a fresh fee can fix keeps what a resend needs.
            if !(state == .failed && canResend) { settled.resend = nil }
            activity[i] = settled
            save()
        }
    }

    /// A pending row's current reason (it is still waiting).
    private func explain(_ id: UUID, why: String) {
        if let i = activity.firstIndex(where: { $0.id == id }), activity[i].state == .pending {
            activity[i].why = why
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
    /// Who a send went to (lowercased), for the address-poisoning and
    /// first-send checks. Nil in records written before token send existed.
    var recipients: [String]? = nil
    /// A token this action moved (address), for the display policy.
    var token: String? = nil
    /// Node rows have this hash; local rows acquire it before submission.
    var hash: String? = nil
    var source: String? = nil
    var owner: String? = nil
    /// Why this row is pending or failed, in plain words (contracts-live
    /// bug #5): the network's answer, never a bare "Failed".
    var why: String? = nil
    /// What a "새 가격으로 다시 보내기" needs: a plain transfer's recipient,
    /// exact amount and nonce. Kept only while the drop can be fixed by a resend.
    var resend: Resend? = nil

    typealias Resend = ResendIntent

    func with(state: State) -> ActivityItem {
        var c = self
        c.state = state
        return c
    }

    func with(date: Date) -> ActivityItem {
        var c = self
        c.date = date
        return c
    }

    /// The node named a time for this row (a missing time is rendered as
    /// nothing, never as an epoch countdown).
    var timeKnown: Bool { date.timeIntervalSince1970 > 86_400 }

    func with(hash: String) -> ActivityItem {
        var c = self
        c.hash = hash
        return c
    }

    /// A node reward this wallet received (iPhone Home shows those as one line).
    var isNodeReward: Bool { kind == .received && (title.hasPrefix("Proof reward") || title.hasPrefix(String(localized: "Proof reward"))) }
}

/// The user's own choices about which tokens to show, kept on this device only
/// (UserDefaults, per chain). The rest of the display policy is derived from
/// the wallet's own on-chain history, so every device agrees on that part.
struct TokenChoices: Codable, Equatable {
    var hidden: Set<String> = []
    var shown: Set<String> = []
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

extension TxStatusText {
    /// The sentence for one `tx_status` answer.
    static func sentence(_ st: TxStatus) -> String {
        sentence(state: st.state, reason: st.reason, success: st.receipt?.success, message: st.message)
    }
}

enum NodeRegistrationError: LocalizedError {
    case unsupported
    var errorDescription: String? { String(localized: "This Mac cannot prove it is a real Mac to Apple right now (it needs the signed \(Brand.name) app on a real Mac).") }
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
        if to.isEmpty { return String(localized: "Deploy a contract (\((data.count - 2) / 2) bytes)") }
        let known: [String: String] = [
            "0xa9059cbb": String(localized: "Token transfer"), "0x095ea7b3": String(localized: "Token approval (allows spending)"), "0x23b872dd": String(localized: "Token transfer from"),
            "0x38ed1739": String(localized: "Swap tokens"), "0xe8e33700": String(localized: "Add liquidity"), "0xbaa2abde": String(localized: "Remove liquidity"),
        ]
        return known[String(data.prefix(10)).lowercased()] ?? String(localized: "Contract call \(String(data.prefix(10)))")
    }
}

/// A page asking to know this wallet's address.
struct ConnectRequest: Equatable {
    let origin: String
    let callback: URL
}

#if DEBUG
extension WalletModel {
    /// Design preview only (DesignPreview.loadPreview): the sample state whose
    /// setters are private to this file.
    func loadPreviewExtras() {
        breakdown = try? BalanceBreakdown.decode("""
            {"proof_rewards_wei":"2500000000000000000","node_rewards_wei":"0","faucet_wei":"10000000000000000000",
             "received_wei":"0","unwrapped_wei":"0","sent_wei":"0","fees_wei":"42000000000000",
             "total_in_wei":"12500000000000000000","total_out_wei":"42000000000000","balance_wei":"12500000000000000000",
             "difference_wei":"42000000000000","itemizes_completely":false,"rows":5}
            """)
        sitePermissions.grant(origin: "https://eastsea.xyz", address: address)
        if UserDefaults.standard.string(forKey: "previewIncomingRecovery") == "1" {
            incomingRecovery = RecoveryStatus(guardians: 1, threshold: 1, delaySeconds: 172_800, pending: true,
                                              readyAt: UInt64(Date().addingTimeInterval(150_000).timeIntervalSince1970))
        }
        recoveryCode = "ae1q7m3kx9w2c8v4r6t0y5u1p3s7d9f2g4h6j8k0l"
        keyLabel = String(localized: "Key in the Secure Enclave")
    }
}
#endif
