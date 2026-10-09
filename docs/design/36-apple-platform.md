# 36. iCloud, Keychain, passkeys and the rest of the Apple platform (2026-10-07)

The founder (2026-10-07): "icloud나 키체인에 대한 이점을 활용하는게 없네?" Then:
"애플용 숨겨진 레이어 통신등등" and "분명히 무임승차의 기회인데." We ship only on
Apple hardware and use almost none of what that hardware comes with. This
document goes through what we could use, what it costs, what it risks, and
whether it touches the genesis.

Design only. Nothing here is built yet. Platform facts that this document relies
on but did not test are marked **(verify)**. The first task in each lane
checks them (§12).

## 0. Summary

| # | Idea | Recommendation | Genesis? |
|---|---|---|---|
| 1 | Passkey as a **recovery** key | Yes, as a guardian behind the 48 h delay. Never an owner, never an ERC-1271 signer. Build native WebAuthn into **account v3**, deployed at a new address after beta. Accounts move to it by delegating again. **Do not change v2 before genesis.** | No |
| 1b | Passkey PRF to derive a v2 guardian key | No, not as a recovery path. Apple's PRF output differs between synced devices in reports from the field, and a recovery key that comes out different on the new phone is worse than none. | No |
| 2 | CloudKit private DB as the user's own push relay | Yes. It is the only serverless way to wake a suspended iPhone. Optional, with an end-to-end encrypted payload and a static alert text. | No |
| 3 | iCloud sync of non-secret wallet data | Yes, for contacts, labels and settings. Use CloudKit `encryptedValues`. A fixed never-sync list (§5.3). | No |
| 4 | Keychain for node secrets | Mostly no. Apple's data protection keychain cannot be read by a launchd daemon, so it would break unattended restart (design 29). Instead, bind the key files to the Mac's hardware and refuse to sign anywhere else. That closes the real equivocation hole: Migration Assistant, Time Machine and an external disk moved to another Mac. | No |
| 5 | Handoff, App Intents, Watch, lost-device flow | App Intents "send" and the lost-device flow first. Handoff next. Watch last. No Sign in with Apple. | No |
| 6 | Local layers (Bonjour, AWDL, AirDrop, Local Push) | iPhone ↔ own Mac over LAN/AWDL, and payment requests over AirDrop and the share sheet, first. Public APIs only. | No |
| 7 | Free Apple infrastructure | APNs through CloudKit, the user's own iCloud quota, AWDL, and App Store/TestFlight for the iPhone. The public CloudKit DB as a CDN is not worth it. Each one is an accelerator, verified against the chain, with a fallback. | No |

**Nothing in this document needs a genesis change.** That is a finding, not an
assumption. `crates/execution/src/forks.rs` already says it: "Account
contract fixes deploy at a new address instead; accounts move to it by
delegating again (EIP-7702)." Execution does not special-case `0x…7702`.
`AETHER_ACCOUNT` appears in execution only as a constant. The node's activity
indexer (`crates/node/src/chain.rs:2674-2687`) and the wallet's delegate sites
(`crates/ffi/src/lib.rs`, 8 places; `crates/node/src/main.rs:1231,1250`) do refer to it. Those are a node release
and a wallet release, not a fork (§10).

## 1. What we use today

- **Wallet key.** `SecureEnclave.P256.Signing` with
  `kSecAttrAccessibleWhenUnlockedThisDeviceOnly` and `.userPresence` (Touch ID
  or the login password). Its handle (`dataRepresentation`) is a file written
  with `.completeFileProtection`, not a keychain item
  (`apps/wallet/Sources/EnclaveKey.swift:78-87`). The chain address is
  `keccak256(0x01 ‖ compressed key)[12:]`. One device = one account.
- **Recovery.** Guardians are raw P-256 keys, k of n, with a delay (default
  48 h) and owner cancel. The calls are `setGuardians`, `addGuardian`,
  `proposeRecovery` and `executeRecovery` (`contracts/src/EastSeaAccount.sol`).
  A guardian is another device's SE key or the 24-word paper key
  (`crates/ffi/src/paper.rs`, a software P-256 key). Recovery can add an
  owner, and that owner then signs `ownerExecute` immediately with no delay.
- **F-05 still holds.** Recovery does not revoke the original key
  (09-wallet.md, the "원래 키는 복구로 폐기되지 않는다" section). Nothing here
  changes that.
- **Signature check.** Every account path is `P256VERIFY(sha256 digest, r,
  s, x, y)`, and ERC-1271 also requires low-s. No WebAuthn.
- **Node secrets.** `<data>/validator.key`, `validator.pub.json`,
  `node-account.key` are 0600 files (`supervisor.rs:1554`
  `KEEP_ACROSS_NETWORKS`). The voting-node identity is in `<data>`
  (`main.rs:432`). The DeviceCheck token is written hourly by the app to
  `<data>/devicecheck-token` (design 29). Before login, the node runs as the
  user through a root stub (design 29).
- **Distribution.** Mac: Developer ID DMG and Sparkle, with release approval
  under design 19. App Attest is unavailable on Developer ID; DeviceCheck works
  (12-launch-plan.md, row 7). iPhone: `com.pipln.eastsea.ios`, through
  TestFlight and the App Store.
- **Nothing syncs.** We use no iCloud, no CloudKit and no APNs.

## 2. Ground rules for every idea below

1. **Public APIs only.** We use no private frameworks, private entitlements,
   `dlopen` of `PrivateFrameworks`, or undocumented AWDL/`IO80211` calls.
   Apps that use them are rejected by App Review, lose their notarization
   standing, and break when the OS updates. "Hidden layer" in this document
   means public but rarely used, never private.
2. **Apple is an accelerator, never a source of truth.** Anything that reaches
   us through iCloud, CloudKit, APNs, AirDrop, Bonjour or the App Store is
   checked the way we check a peer. Blocks and state need a finality
   certificate plus a proof (`aether-light`). Content needs its hash. Releases
   need the 2/3 builder manifest (design 19). If Apple drops it, delays it or
   alters it, the result is "slower", never "wrong".
3. **No founder control.** A CloudKit container
   (`iCloud.com.pipln.eastsea`), an associated domain, an APNs topic or an App
   Store asset belongs to Pipln's developer account. Pipln or Apple can
   switch it off. That is acceptable only for **optional accelerators with a
   working fallback**. It is never acceptable for anything a user needs in
   order to hold, send or recover funds, or for anything a node needs to
   follow the chain. Every item in §9 says which kind it is.
4. **A validator key never leaves its Mac.** If two Macs sign with one
   validator key, that is equivocation. No sync, no backup and no migration
   may copy one into a running state elsewhere (§6).
5. **Consumer framing.** The user sees "Recover with your iCloud account" and
   "Your iPhone will tell you", never "passkey PRF" or "CloudKit" (memory:
   product-philosophy).

## 3. Passkeys as a recovery key

### 3.1 Benefit

A passkey is a P-256 key held by the iCloud Keychain, synced end to end
between the user's Apple devices. Today, if a user loses every device and has
no paper key, the account is gone. With a passkey guardian, the user can buy a
new iPhone, sign in to iCloud, open EastSea, tap "Recover", approve with Face
ID, and wait 48 h. There is still no seed phrase.

That is the largest consumer gain in this document. "Lose your phone, lose
your money" is the main reason people do not keep money in self-custody
wallets.

### 3.2 Why not just register the passkey's public key as a v2 guardian

A passkey never signs our raw digest. WebAuthn signs:

```
sig = ECDSA-P256( sha256( authenticatorData ‖ sha256(clientDataJSON) ) )
authenticatorData = rpIdHash(32) ‖ flags(1) ‖ signCount(4) ‖ [extensions]
clientDataJSON    = {"type":"webauthn.get","challenge":"<base64url(challenge)>","origin":"https://<rpId>",...}
```

The `recoveryDigest` the contract builds is the *challenge* inside
`clientDataJSON`. It is not the hash that was signed. v2's `_verify` calls
`P256VERIFY` on the digest directly, so a passkey assertion can never pass
`proposeRecovery`. Passkeys need contract code that rebuilds the WebAuthn
message.

### 3.3 What the contract must check (WebAuthn verification)

To accept a guardian signature from a key marked `kind = WEBAUTHN`:

1. **Rebuild the challenge.** Compute `recoveryDigest(calls, nonce)` on chain,
   base64url-encode it with no padding, and require
   `clientDataJSON[challengeIndex:]` to start with
   `"challenge":"<that>"`. Require `"type":"webauthn.get"` at `typeIndex`. The
   indexes come with the signature. Matching against a fixed prefix stops a
   forged `"challenge"` from being hidden inside another field.
2. **Check authenticatorData.** At least 37 bytes. `rpIdHash ==
   sha256(rpId)` for the rpId stored with the key (bind to the domain, and do
   not rely on `origin`). The flags must have **UP** (0x01) and **UV** (0x04)
   set: user present and user verified (Face ID, Touch ID or the device
   passcode). If **BS** (backed up, 0x10) is set, **BE** (0x08) must be set
   too. Ignore `signCount`, because synced passkeys report 0.
3. **Hash.** `h = sha256(authenticatorData ‖ sha256(clientDataJSON))`, then
   `P256VERIFY(h, r, s, x, y)`.
4. **Low-s.** Authenticators give a high `s` about half the time. The relayer
   or wallet converts the DER signature to `(r, s)` and replaces `s` with
   `n − s` when `s > n/2`. Both values verify, so this is safe. The contract
   then *requires* low-s like every other path. This keeps the chain-wide rule
   that one message has no malleable twin (review R-05 still applies: low-s is
   not uniqueness, and the nonce does the replay protection).
5. **Domain separation by key kind.** A key is stored with its kind (`RAW` or
   `WEBAUTHN`). A raw signature is never checked against a WebAuthn key, and a
   WebAuthn signature is never checked against a raw key. `abi.encode`
   includes a new tag (`aether.recovery.webauthn`), so an assertion for
   another purpose cannot be reused.
6. **No new owner powers.** A WebAuthn key can be a guardian. It cannot be an
   owner (`ownerExecute` has no delay), and `_checkSignature` (ERC-1271)
   rejects it.

Use **audited code**, not new code. OpenZeppelin Contracts ships a `WebAuthn`
library (5.4+), Coinbase's Smart Wallet uses `webauthn-sol`, and Daimo has
`p256-verifier`'s WebAuthn wrapper. Port one of them, keep a differential
test against the other two, and change only the challenge binding. (verify:
the version and audit report of the OZ library at port time.)

### 3.4 Roles: which key does what

| Key | Where it lives | Role | Why |
|---|---|---|---|
| Secure Enclave key (this device) | One Mac or iPhone, cannot be exported | **Daily spending.** The account's own key. Touch ID or Face ID per payment | Device-bound. Stealing it means stealing the unlocked device and the finger |
| Second device SE key | Another Mac or iPhone | Guardian (exists today) | Device-bound, independent |
| **Passkey** | iCloud Keychain, synced | **Guardian only, behind the delay.** It can start a recovery that adds the new device's SE key as an owner, or moves the funds | Synced, so it inherits iCloud account takeover risk. The delay turns a takeover into a 48 h visible event the owner can cancel |
| Paper key (24 words) | Paper | Guardian (exists today) | For users who want no Apple dependency |
| Session key | Agent's SE | Limited payments (exists today) | Unchanged |

The founder's hypothesis was SE key for daily spending and passkey for
recovery or owner rotation behind the delay. That is right, with one
sharpening: the passkey is a **guardian**, and owner rotation happens *through*
a delayed recovery. It is never a direct `addOwner`. A passkey that could act
without the delay would make iCloud account takeover an instant theft.

Default setup for a new user with one iPhone: SE key plus passkey guardian
(1 of 1, 48 h). A user with a Mac and an iPhone: each device is the other's
guardian, plus the passkey. With 1 of 3, any one can start a recovery and any
owner can cancel. A careful user can choose 2 of 3: passkey plus paper, or
passkey plus the other device. The wallet suggests that only if the user asks
for "more protection", because the 2-of-3 path fails when the user has lost two
of the three.

### 3.5 Threat model

| Threat | Result with passkey as guardian | Mitigation |
|---|---|---|
| iCloud account takeover (Apple ID password plus a trusted device's passcode, the 2023 "passcode thief" pattern) | Attacker gets the passkey and starts a recovery | 48 h delay. Owner devices are told through §4 and in-app. One tap cancels. Recommend Stolen Device Protection (iOS 17.3+) during passkey setup |
| User loses every device | Recovery completes after 48 h, because nobody is left to cancel | That is the purpose |
| Domain (rpId) hijack. The passkey is bound to `eastsea.xyz` (or whichever rpId we choose). Whoever controls that domain can serve a web page that asks the user's passkey for an assertion over any challenge | One Face ID tap by a phished user gives one recovery proposal | 48 h delay plus cancel. Bind `rpIdHash` in the contract. Treat the domain as security-critical (DNSSEC, registrar lock, no CNAMEs to third parties). This is a **Pipln-controlled point**, the same class as the registrar. It is acceptable only because the passkey's power is delay-gated and cancellable |
| Passkey shared through iOS 17 shared credential groups, or exported through the Credential Exchange Protocol (iOS 26) | Another person now holds a guardian | The wallet's setup screen says so in plain words. Make the passkey name obvious ("EastSea recovery — do not share"). The contract cannot detect this |
| Apple itself | iCloud Keychain is end-to-end encrypted even without Advanced Data Protection. Escrow is HSM-guarded and limited by the device passcode. Apple cannot read it under its published design, but it is a trust assumption | A user who distrusts Apple uses the paper key or a second device instead. Passkey is optional |
| Recovery spam (a stolen passkey is used again and again) | Each proposal needs a fresh assertion (fresh nonce), so it needs the attacker's continued access. The owner removes the passkey guardian with `setGuardians` | Owner UI: "Remove this recovery method" after a cancelled recovery |

**Advanced Data Protection (ADP).** ADP does not change passkey
confidentiality, because iCloud Keychain is already end-to-end. What ADP
changes is account recovery. Without ADP, Apple can help a user back into the
account and its non-E2E categories. With ADP, only the device passcode, a
recovery contact or a recovery key can. It also makes CloudKit private-DB
fields end-to-end even when they are not `encryptedValues` (§4, §5). We
**recommend** ADP in the setup flow, as one sentence, and we do not require
it. Requiring ADP would lock out users who later lose their recovery key,
which is exactly the user this feature is for.

### 3.6 Genesis decision: v2 now, or v3 later?

**Recommendation: defer to v3. Leave the v2 bytes and code hash exactly as
reviewed.**

Reasons:

- **There is no need to rush.** Accounts can move to a new account contract
  by self-delegation at any time (`EvmCall.delegate`, any target address, the
  tx signature is the authorization: `tx.rs:18-22`, `block.rs:259`). The
  wallet can do it with the user's next Touch ID, inside a batched payment.
  Waiting costs one extra delegation per account, nothing more.
- **Changing v2 now resets the beta gate.** The v2 runtime was just reviewed
  byte for byte (docs/research/account-1271-review-2026-10-07.md: 17,480
  bytes, `0xdeaca4e6…`, 172 tests). Any change means new predeploy bytes, a
  new genesis root, a new ceremony record, a new review and a new 72 h soak.
  It also adds the riskiest code in the contract (JSON and base64 handling)
  to the one artifact we cannot patch after launch.
- **v3 is already planned** (ERC-7739 readable signing, R-01). WebAuthn
  guardians belong in the same audit: both change how signatures are checked.

**v3 requirements so that it can come later without a fork:**

- The same ERC-7201 slot (`STATE_SLOT`). Fields are only appended (key kind,
  rpIdHash). The existing `Key[] guardians` stays readable as `RAW`.
- Deployed by the predeployed Arachnid CREATE2 proxy at a fixed address. No
  owner, no initializer, no admin. The wallet pins the v3 code hash in its
  release (design 19), and it delegates only to a pinned hash.
- The node indexer recognizes a set of account targets instead of one
  (`chain.rs:2674-2687`). This is a node release.
- `ffi` gets a single `ACCOUNT_TARGET` from the network config instead of 8
  literal uses of `AETHER_ACCOUNT`. This is a wallet release.

**What the v3 audit needs** (for WebAuthn; ERC-7739 has its own list):

1. A spec of the exact byte layout and index rules (§3.3), with a proof
   sketch that the challenge match is unambiguous.
2. Real test vectors: iCloud Keychain on iPhone and Mac, Chrome's password
   manager, a YubiKey, and the hybrid (QR) flow. High-s and low-s variants.
   UV=0, UP=0, BE=0/BS=1, a wrong rpIdHash, the wrong type, a challenge with
   padding, a challenge hidden in `origin`.
3. A fuzz target over `clientDataJSON` and `authenticatorData`, and a
   differential test against OZ, Coinbase and Daimo's libraries.
4. Key-kind confusion tests: a raw signature against a WebAuthn key, the
   reverse, and the same x,y registered as both kinds.
5. Gas: the worst-case `clientDataJSON` length is bounded, so the proposal
   cannot be made too expensive to relay.
6. Storage layout diff against v2 (append-only), and a redelegation test on a
   live v2 account with a pending recovery, owners, sessions and token
   limits.
7. A red-team pass on the domain-hijack case (§3.5) and on the wallet UI that
   shows what a recovery proposal will do.

### 3.7 Why not the PRF shortcut

WebAuthn's PRF extension (iCloud Keychain, iOS 18+ / macOS 15+) returns 32
bytes derived from the passkey and a salt. We could hash that into a software
P-256 key, exactly like the paper key, and register it as a **v2** guardian
with no contract change. It is tempting.

We reject it as a recovery path. Developer reports say that the same synced
passkey gives **different PRF output on different devices**, and also between
on-device and hybrid use (Apple Developer Forums threads 774111 and 822523,
2025-2026). A recovery key that derives differently on the new iPhone fails
at the exact moment it is needed, and silently. It also turns a one-time
phished assertion into a stolen reusable secret. We keep it only as a lab
item: re-test it on each major OS release with a two-device consistency
check. If Apple documents cross-device stability, we can reconsider it as a
bridge until v3.

## 4. CloudKit private database as the user's own push relay

### 4.1 The problem

A suspended iPhone app does not run. Nothing on chain can wake it: no RPC
subscription, no iroh connection, no on-chain bus (design 35, on branch
`codex/chain-push`, is being written to say this plainly). Only APNs wakes an
iPhone. Sending APNs directly needs a provider key (`.p8`) per developer
team. Shipping that key to every node is impossible, and running a Pipln push
server breaks the "no Pipln server" goal.

### 4.2 Design

The user's own Mac is the sender. CloudKit is the mailbox. APNs is the
doorbell. Apple runs both for free.

```
Mac (wallet app or its user-session agent)
  sees a finalized event for one of the user's accounts
  (payment received, recovery proposed, agent limit hit, update needs you)
  → encrypts {kind, amount, counterparty, height, tx} to the iPhone's key
  → saves a CKRecord "Note" in the user's PRIVATE database, custom zone "inbox"
       fields: ciphertext (encryptedValues), createdAt
iCloud → CKDatabaseSubscription on zone "inbox" (created by the iPhone)
  → APNs push, shouldSendMutableContent = true,
    static alert text set by the iPhone: "EastSea: new activity" / "새 활동이 있어요"
iPhone Notification Service Extension
  → fetches the record, decrypts with the key in the app group keychain,
    rewrites the alert: "Mina sent you 3,000 DOKDO"
  → (when the app next runs) verifies the event against the chain
    (finality certificate + receipt proof); a note that fails is dropped and never shown again
  → deletes the record
```

- **Pairing.** The iPhone makes a P-256 key-agreement key
  (`SecureEnclave.P256.KeyAgreement`, this device only) and writes its public
  key to the private DB. The Mac reads it there. It is the same iCloud account,
  so no QR code is needed, but the Mac shows the iPhone's name and asks once.
- **Encryption.** Two layers: CloudKit `encryptedValues` (end-to-end with
  keys in iCloud Keychain, even without ADP), plus our own ECDH (P-256) +
  HKDF + AES-GCM to the iPhone's SE key. With the second layer, a leak of the
  iCloud Keychain still does not show the note to anyone without the iPhone.
- **Verification.** The note is a hint, not a fact. A "payment received" note
  only creates a banner. The balance the app shows comes from `aether-light`
  as it does today. The banner wording is "Mina sent you…", but the app's
  activity row appears only after the proof checks. If it fails, the app
  shows nothing.

### 4.3 Honest limits

- **The Mac must be on, online and logged in.** CloudKit needs a user session
  with iCloud signed in. The pre-login node under design 29 cannot write.
  Before login, notes wait until the user logs in, or until any other path
  (the iPhone app opening) catches up. Say so in Settings: "Your iPhone hears
  about payments while this Mac is on."
- **Without a Mac, nothing changes.** An iPhone-only user gets no wake-ups
  from this design. Design 35's options apply to them.
- **Rate limits.** CloudKit returns `CKError.requestRateLimited` with
  `retryAfter`. Apple publishes no per-user number (verify). We batch: at
  most one record per 30 s per user, merging events. Each note is a few
  hundred bytes. The inbox keeps at most 64 records, the oldest are deleted,
  and the iPhone deletes records it has read.
- **APNs is best effort.** Visible notifications through a subscription are
  generally delivered, but coalesced. Silent pushes are throttled hard, so we
  never rely on them.
- **Storage is the user's iCloud quota.** At 64 × 1 KB it is negligible, even
  on the free 5 GB.
- **Metadata Apple sees.** Apple sees the Apple ID, that this app wrote a
  small record, when, and how large it was. It can therefore see the timing of
  the user's incoming activity, which is roughly a payment timeline. Padding
  every note to 1 KB hides sizes. A cover note per day could hide
  inactivity, but we do not build that at first. It goes in the privacy text.
- **Pipln-controlled point.** The container `iCloud.com.pipln.eastsea`
  belongs to Pipln's team. Pipln can break it, but cannot read the private
  DB. This is acceptable because it is an optional accelerator: with the
  relay off, the iPhone learns everything the next time it opens, as it does
  today.
- **Developer ID.** The Mac app is not on the Mac App Store. CloudKit and
  push are listed as available to Developer ID apps with a provisioning
  profile (verify on our team, task A1).

### 4.4 Genesis

None. The events already exist as receipts and logs. Design 35 may add
account topics; this relay consumes whatever the Mac's node finalizes.

## 5. iCloud sync of non-secret wallet data

### 5.1 What syncs

| Data | Store | Why |
|---|---|---|
| Contacts and labels (name ↔ address, notes) | CloudKit private DB, `encryptedValues` | The address book follows the user. Labels are personal, and a leak is a privacy harm, so they are E2E |
| Settings (language, currency, hide small tokens, notification choices) | `NSUbiquitousKeyValueStore` | Small (1 MB, 1024 keys), simple, free. Not E2E, so settings only, never addresses |
| Account list (my accounts: address, device name, which is guardian of which) | CloudKit private DB, `encryptedValues` | "Your Mac's account" appears on the iPhone, and recovery knows which accounts to offer to recover |
| Activity cache (recent rows, token metadata) | CloudKit private DB, `encryptedValues`, capped | Faster first screen on a new device. Always re-verified before display as confirmed |
| Pairing keys for §4 (public only) | CloudKit private DB | Public keys only |

Conflict rule: last writer wins per record, with records small enough
(one contact per record) that conflicts are rare. Deletes are tombstones for
30 days.

### 5.2 Trust

A synced address is still an address the user is about to send money to. A
compromised iCloud account could swap a contact's address. So: (1) a contact
edited on another device is shown with "changed on iPhone, 2 min ago" for 24 h
before the send screen uses it silently; (2) when the name service (design 26)
has a name for the address, the send screen shows the name from the chain,
not the label from iCloud.

### 5.3 Must NEVER sync

- The Secure Enclave key handle, any session key, the paper words or anything
  derived from them.
- `validator.key`, `node-account.key`, the voting identity, threshold
  shares, `run.lock`, the DeviceCheck token, the node data directory.
- Anything that grants a permission, for example a "trusted site" or
  "approved payee" list that skips a confirmation. These stay per device, or
  are on chain. Syncing them would let a hijacked iCloud account turn off
  prompts on every device.
- Unconfirmed balances presented as confirmed.

The code enforces it: one `SyncPolicy` table with an allowlist of record types,
and a unit test that fails if any type outside the allowlist reaches the
CloudKit layer. The data directory gets `NSURLIsExcludedFromBackupKey` and
`tmutil addexclusion` (§6).

### 5.4 Genesis

None.

## 6. Keychain for node secrets

### 6.1 What the keychain can and cannot do here

The founder's proposal: move `validator.key`, `node-account.key`, the node
identity and the DeviceCheck token into keychain items with
`kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`, not synchronizable.

The blocker is Apple's own rule: **the data protection keychain is not
available to launchd daemons**, and on macOS it needs a GUI login session
(TN3137, "On Mac keychain APIs and implementations"; Apple DTS on the
Developer Forums). Our node must vote *before anyone logs in*. That is the
whole point of design 29. If `validator.key` lived only in the data protection
keychain, every reboot without a login would stop the vote. That is the
failure design 29 was written to fix.

The other stores:

| Store | Pre-login readable by our node? | Stops same-user malware? | Survives Migration Assistant / Time Machine to a new Mac? |
|---|---|---|---|
| 0600 file (today) | Yes | No | **Yes: the danger** |
| Data protection keychain, `ThisDeviceOnly`, access group `LBKUT88RTX.com.pipln.eastsea.node` | **No** (TN3137) | Yes: only our signed binaries with the entitlement | No |
| Login (file-based) keychain | No: locked until login | Partly (ACL by code signature) | Yes |
| System keychain via the root stub | Yes (root) | Yes, against non-root processes | (verify) |
| Secure Enclave wrapping key | No: SE keys need the user's keybag (TN3137) | — | No |

### 6.2 The real hazard is migration, not theft

What actually threatens the chain is a second running copy of a validator
key:

- Migration Assistant or a Time Machine restore onto a new Mac, while the old
  Mac still runs;
- an external disk holding the data directory, plugged into another Mac
  (the external-disk data-location work);
- a disk clone, or someone copying the folder by hand.

`ThisDeviceOnly` would solve this, but it is unavailable before login. So we
solve it directly.

**Hardware binding (recommended, effort S).**

1. When the node creates `validator.key` (and `node-account.key`), it writes
   `key-binding.json`: `{validator_pub, platform_uuid_hash, created_at}`.
   `platform_uuid_hash` is `sha256("eastsea.bind" ‖ IOPlatformUUID)`.
   IOPlatformUUID uses public IOKit APIs. An unavailable read is not evidence
   of another Mac, including before login or under a sandbox policy.
2. On every start and before validator content signing, the node compares the
   hash with this Mac's. Only a successfully read different hardware hash
   exits 15 (`EXIT_KEY_ELSEWHERE`). Unavailable verification keeps the process
   running, pauses content signing, and retries with bounded backoff, showing
   "waiting to confirm this Mac". Raw transport identities have a bounded
   lifecycle check; their TLS/DHT/handshake signatures authenticate transport,
   not validator content (see `p2p.rs` for the pinned dependency limitation).
3. In 0.7.3 the shipped owner recovery is `aether keys rebind --data <dir>`:
   stopped node (`run.lock`), typed validator address, atomic durable binding
   replacement, and old→new hash audit. The wallet requires its existing
   Secure Enclave owner authentication and exposes this only for a proven
   mismatch. Follow [moving a validator](../ops/moving-a-validator.md) to retire
   the old signer and preserve the current share and vote journals.

   A future automated move would use `aether validator move-out` on
   the old Mac writes a signed "retired at height H" record and stops; `move-in`
   on the new Mac accepts only a key with a retirement record, and only after
   H + a safety margin is finalized). This matches the manual move in the
   testnet-validator-layout notes. These commands are not implemented.
   DeviceCheck's "one Mac,
   one identity" (design 14) is enforced by the registrar too. Moving a seat
   to a new Mac can need registrar reconciliation. Rebinding preserves an
   existing committee identity; registering a new key is not seat recovery.
4. Exclude the data directory from Time Machine (`tmutil addexclusion` on
   the key files at least) and from any iCloud Drive path. Refuse to place the
   data directory under `~/Desktop` or `~/Documents` when "Desktop &
   Documents" iCloud sync is on (check
   `NSURLUbiquitousItemIsUploadedKey` on the parent).

**External disk.** Keys stay on the internal disk, always. When the user moves
chain data to an external disk, `validator.key`, `validator.pub.json`,
`node-account.key`, `key-binding.json` and the voting identity stay in
`~/Library/Application Support/…` (internal). Only blocks, shards and
snapshots move. The node takes `--key-dir` separately from `--data-dir`. A
data directory found on an external disk that *contains* a key is refused
with "keys must stay on this Mac".

### 6.3 Where the keychain does help

- **Agent and app-only secrets.** Anything only the app or `aether-agent`
  uses in a user session can use the data protection keychain with an access
  group. The SE key handle could move into a keychain item
  (`kSecClassKey` with `kSecAttrTokenIDSecureEnclave`) instead of a file.
  The gain is small, because the handle is already useless off this device's
  SE, so this is optional.
- **Stronger node key at rest, opt-in (later, effort M).** For users who turn
  *off* "keep running after restart", the node could hold `validator.key` in
  the data protection keychain, and the app hands it to the node at start over
  a pipe. This blocks same-user malware from reading the file. It is a second
  code path with its own fault tests, so do it only when someone asks.
- **System keychain through the root stub (rejected for now).** It works
  before login, but it routes the validator secret through the root stub.
  Design 29's rule 3 is "only the stub runs as root"; the stub should not
  carry secrets. Revisit only together with a hardened stub review.
- **DeviceCheck token.** It is short-lived, and its only value is the daily
  re-attestation. Leave it a file.

### 6.4 Genesis

None. Binding and `keys rebind` are node-local. The proposed `move-out` /
`move-in` retirement record would be a local file, not a chain object.

## 7. Other Apple-platform features

Ranked by consumer value per effort.

| Rank | Feature | Value | Effort | Notes |
|---|---|---|---|---|
| 1 | **Lost-device flow** | High | S-M | There is no public Find My API for apps: no "device marked lost" signal and no location. So the flow starts from what the user has: (a) on another device, "I lost my iPhone" → remove that device's sessions; if it was an added owner, `removeOwner`; if it was the account's original key, explain F-05 and offer "move everything to this device's account" in one batch; (b) on a new device with only iCloud, "Recover with iCloud" (passkey guardian, §3). Same screens as today's recovery, with consumer words |
| 2 | **App Intents / Shortcuts "Send"** | Medium-high | S-M | `SendIntent(amount, contact)` opens a confirmation sheet, and the SE signs with Touch ID / Face ID. Never `openAppWhenRun = false` for a payment. Never allow "run without asking" for spending: the SE's `.userPresence` enforces it anyway. Read-only intents ("balance", "last payment") can run in the background, but show them only after unlock. Siri mishears amounts, so the confirmation shows amount and name in large type |
| 3 | **Handoff** | Medium | S | `NSUserActivity` "pay request" (to, amount, memo). Start on the Mac, finish on the iPhone or the reverse. Each device has its own account, so the receiving side pays from *its* account. If the user has made one device an owner of the other's account, it can sign `ownerExecute` for that account. The handed-off draft is re-validated (name lookup, balance) and never pre-approved |
| 4 | **Universal Clipboard** | Low (already free) | 0 | Addresses copied on the Mac paste on the iPhone. Risk: clipboard address swapping by malware. When the pasted address is not in the contacts, the send screen shows the first and last 6 characters large and asks for a look |
| 5 | **Apple Watch approval** | Low-medium | L | A Watch has its own SE (verify: `SecureEnclave.P256` on watchOS for our deployment target). Best fit: a **session key** on the Watch with small limits (existing `addSession`, no contract change). Approve by double-click, with wrist detection as user presence. Not an owner. A new target, a new UI and new QA: last |
| — | Sign in with Apple | None | — | Not needed. We have no accounts and no server. Adding it would create a Pipln-held identity link, which we do not want |

Genesis: none. All use existing account functions.

## 8. Local, peer-to-peer and less-used communication layers

**Public APIs only.** Everything here is a documented framework. AWDL is
reached only through `includePeerToPeer`, MultipeerConnectivity, AirDrop
and the share sheet. It is never reached directly. Private Wi-Fi or AWDL
interfaces are rejected by App Review and break on OS updates.

### 8.1 The layers

| Layer | What it gives us | Entitlements / prompts | Privacy |
|---|---|---|---|
| **Bonjour (mDNS/DNS-SD) via Network.framework** `NWBrowser` / `NWListener` | Find the user's Mac on the LAN | iOS 14+ and macOS 15+ show a local network prompt. `NSLocalNetworkUsageDescription` plus `NSBonjourServices` (`_eastsea._tcp`). Bonjour does **not** need the multicast entitlement (`com.apple.developer.networking.multicast`). Only raw multicast/broadcast sockets need it, and Apple grants it by request | Advertising shows "an EastSea node is here" to everyone on the network. Use an opaque instance name derived from the pairing key, an empty TXT record, and advertise only on networks the user marked as home |
| **`includePeerToPeer = true`** (Network.framework) | The same browse and connect over **AWDL**: Mac ↔ iPhone with no router and no internet | Same prompt | AWDL advertising is visible to nearby Apple devices. Use the same opaque name, and turn it on only while the iPhone app is in the foreground or syncing |
| **MultipeerConnectivity** | An older API over AWDL plus infrastructure Wi-Fi plus Bluetooth | Same prompt | Shows a display name to nearby peers. Prefer Network.framework, which is newer, gives us our own TLS-PSK, and has no broadcast name |
| **AirDrop / share sheet** (`UIActivityViewController`, `NSSharingServicePicker`) | Send a payment request file or `eastsea:` URL to a nearby person. AWDL underneath. **No permissions** | None | AirDrop shows the sender's name. Contacts-only is the user's own setting |
| **Wi-Fi Aware** (iOS/iPadOS 26) | Standard NAN peer-to-peer, mainly for non-Apple devices (EU interoperability) | Its own entitlement and pairing UI (verify). We found no macOS support | Discovery is scoped to paired devices. Not needed between Apple devices, because AWDL already covers that |
| **Nearby Interaction (UWB)** | Distance and direction between iPhones, for "point at your friend" | Per-session user permission. A discovery token must be exchanged first over another channel | Precise presence is shared during the session only. No Mac support (no UWB) |
| **Core NFC** | Read NDEF tags (a sticker with a shop's payment request) | `NFCReaderUsageDescription` and the NFC entitlement | Fine |
| NFC phone-to-phone / Tap to Pay | Not usable. iPhone-to-iPhone NFC is not open to apps. HCE needs Apple's NFC & SE Platform entitlement, an agreement and regional limits. Tap to Pay on iPhone is for card payments through a payment provider | — | — |
| **Local Push Connectivity** (`NEAppPushProvider`) | Wake the iPhone app from the user's Mac **directly on listed Wi-Fi networks**, with no Apple server | A Network Extension entitlement that Apple grants by request. The provider runs only on configured SSIDs | Nothing leaves the LAN |
| **Continuity / Handoff / Universal Clipboard** | §7 | Same iCloud account, same team | Apple relays Handoff activity metadata (encrypted per Apple) |
| **Find My network** | Third parties get access only through the Find My network accessory program (hardware makers, MFi). Apps cannot query device loss or location | — | Not usable for us, except the §7 flow |
| **iCloud Private Relay** | Covers Safari, DNS and *unencrypted* HTTP from apps. It does **not** proxy our iroh QUIC/UDP. Node P2P is unaffected, and we cannot route node traffic through it either | — | Nothing to do. A note in the FAQ |
| **BGTaskScheduler** | `BGAppRefreshTask` (opportunistic, a few times a day, not guaranteed) for a light-client catch-up. `BGProcessingTask` (charging and idle) for larger syncs. `BGContinuedProcessingTask` (iOS 26) to finish a sync the user started | `BGTaskSchedulerPermittedIdentifiers` | None |
| **Background Assets** | Download large assets outside the app's runtime. Apple-hosted asset packs (iOS 26) are App Store/TestFlight only | Manifest entries | Apple sees downloads, which every App Store user does anyway |

### 8.2 The five uses, judged

**(a) The iPhone syncs and verifies from the user's own Mac over LAN or
AWDL.** The Mac's node already serves RPC. The iPhone light client already
verifies everything (finality certificate plus proof), so the Mac is just a
fast, private transport and needs no trust. Pairing is through §4's private DB
(public keys) or a QR code. The connection is TLS 1.3 PSK from the pairing
secret over `NWConnection`, with `includePeerToPeer` for AWDL.
- *Benefit:* fast sync at home, works with no internet, and no third-party
  RPC sees the iPhone's address queries. That last one is a real privacy gain.
- *Limits:* the Mac must be awake (AWDL needs Wi-Fi powered on). There is the
  local network prompt on both devices.
- *Protocol change:* none, it is the same RPC. *Effort:* M.

**(b) Nearby Macs share blocks and snapshots on the LAN.** iroh can find
peers on the LAN (its local discovery). Our `crates/net` does not enable
it today (no mDNS in `crates/net/src`). Content is verified by hash and
finality as always.
- *Benefit:* catch-up at LAN speed, and less ISP upload for households or
  offices with several Macs.
- *Limits:* useful only where there are two or more nodes. Presence leaks on
  café Wi-Fi, so it is on only for networks marked home or work.
- *Protocol change:* none (transport-level). *Effort:* S-M.

**(c) Person-to-person payment between nearby devices.** Baseline: the
receiver shows a QR payment request, and the payer scans it. That needs no
permissions and works everywhere. Upgrade 1: **AirDrop / share sheet** of an
`eastsea:` request (address, amount, memo, the receiver's name-service
name). Everyone knows AirDrop, and it needs no permissions. The payer's
wallet resolves the name on chain and shows it, then the SE signs with
Touch ID or Face ID. Upgrade 2, later: Nearby Interaction "point at the
person" to choose between several nearby receivers.
- *Risk:* a nearby attacker AirDrops a fake request. The request is
  untrusted input. The payer sees the on-chain name, or "unknown address", and
  the amount, and signs only after that.
- *Protocol change:* none. *Effort:* S for AirDrop, L for UWB.

**(d) Offline payment intent, broadcast when either side is online.** The
payer, offline, signs a normal transaction (cached nonce, a fee cap from the
last quote) and hands it over by AirDrop, QR or local link. Whoever gets
online first submits it.
- *Honest limit:* it is a **promise, not a payment**, until it is final. The
  payer could sign another transaction with the same nonce. The receiver's
  wallet shows "pending, not yet received". The envelope has **no expiry**
  (`TxHeader`: chain_id, sender, nonce, gas, max_fee, tip, commitment,
  scheme, group). A handed-over transaction stays valid until its nonce is
  used. The payer's wallet must be able to void it ("cancel": the same nonce,
  zero-value self-call) and must warn about it.
- *Protocol change:* a `valid_until` header field would make this clean. It
  is a signing-bytes change, added by a protocol upgrade (as `group` was, with
  `serde(default)`), **not** a genesis item. Do not add it unless (d) is
  built.
- *Effort:* M (wallet). It has the most edge cases. Rank it after (a)-(c).

**(e) Waking the iPhone: CloudKit relay or local push.** §4 (CloudKit)
works anywhere the iPhone has internet and the Mac is on. Local Push
Connectivity works with **no Apple server** on the home Wi-Fi, but needs an
entitlement Apple must grant and covers only listed SSIDs. Plan: CloudKit
first (works everywhere, no special entitlement). Apply for Local Push as a
privacy upgrade at home. Fall back to BGAppRefresh catch-up.
- *Protocol change:* none. *Effort:* CloudKit M. Local Push M-L, plus the
  entitlement wait.

### 8.3 Ranking (consumer value per effort)

1. (c) AirDrop / share-sheet payment requests: S, high value, no
   permissions.
2. (e) CloudKit wake (§4): M, high value.
3. (a) iPhone ↔ own Mac over LAN/AWDL: M, high value (privacy, speed,
   offline).
4. BGTaskScheduler catch-up windows on the iPhone: S, medium.
5. (b) LAN block sharing between Macs: S-M, medium for multi-Mac homes.
6. Local Push Connectivity at home: M-L plus an entitlement, medium.
7. (d) Offline intents: M, medium, many edge cases.
8. Nearby Interaction "point to pay": L, low-medium.
9. Wi-Fi Aware: only if Android interop ever matters.

Genesis: none of them. (d)'s `valid_until` would be a later protocol upgrade.

## 9. Apple infrastructure we can ride for free

We already ride the BitTorrent DHT to find addresses (design 33). Apple runs
a lot more infrastructure that every one of our users already pays for with
their device. Each row says what it replaces, who pays, the App Review risk,
how we verify it, and whether a Pipln-controlled point is involved.

| Rank | Item | Replaces | Quota / who pays | ToS / App Review risk | Trust model | No-founder-control |
|---|---|---|---|---|---|---|
| 1 | **APNs through CloudKit subscriptions** (§4) | A push server, its provider key, uptime and on-call | Free. No per-push fee | Low: the documented use | Note is a hint. The app verifies the event on chain | Container is Pipln's. **Acceptable**: optional, and the fallback is "learn on next open" |
| 2 | **CloudKit private DB** (§4, §5) | A sync server and its database, storage and a privacy policy for it | **The user's iCloud quota** (5 GB free; ours is ≈ 1 MB per user) | Low | Synced data is never authority: addresses re-checked against the name service; activity re-verified | Same container. **Acceptable**: wallet works fully without it |
| 3 | **AWDL / LAN between the user's own devices** (§8) | RPC bandwidth for iPhone sync, a public RPC endpoint, and the privacy of address queries | Free, zero internet bandwidth | Low with public APIs (local network prompt) | Light-client verification as today | No Pipln point at all |
| 4 | **iCloud Keychain sync (passkeys)** (§3) | A key-backup service, which we would never run | Free | Low | Guardian power only, delayed 48 h, cancellable | rpId domain is a Pipln point. **Acceptable only** because of the delay. Paper key and device guardians stay available without it |
| 5 | **App Store / TestFlight for the iPhone app** | Our own iOS distribution (none exists outside the store anyway) | Free with the developer program. TestFlight builds expire after 90 days; up to 10,000 external testers | App Review on every release. A wallet that touches crypto must meet guideline 3.1.5 and 2.5 | Release still needs the builder manifest for any *node* code. The iPhone has no node | Apple and Pipln's account control iPhone distribution. **Unavoidable** on iOS. The iPhone wallet is never needed for funds: the same account can be recovered on a Mac through a guardian |
| 6 | **BGTaskScheduler / Background Assets** | Our own background download scheduling | Free | Low | Downloaded bytes verified by hash / proof | Apple-hosted asset packs are tied to Pipln's App Store record: **acceptable** as an accelerator only |
| 7 | **DeviceCheck** (already used) | A Sybil-resistance service | Free. 2 bits per device kept by Apple | Low | The registrar checks it. Consensus never trusts Apple directly | The registrar's DeviceCheck key is Pipln's: already decided and capped (12-launch-plan.md, 레지스트라 row). Not new |
| 8 | **CloudKit PUBLIC DB as a CDN** for snapshots and era files | A CDN or S3 bucket | Developer container quota, which grows with active users. Apple's historical published figures were about 10 GB asset storage + 250 MB per user, and 2 GB transfer + 50 MB per user per month (verify: Apple no longer shows a calculator). At 10,000 active users, that is about 0.5 TB per month. A bootstrap of a few GB (one shard is 800 MiB) gives a few hundred new installs per month before throttling | **Medium**: bulk blockchain data in a public DB is a stretch of "your app's data", and Apple can throttle or disable it | Content-addressed and verified by hash and finality, so a poisoned file only wastes time | Pipln-owned container, and someone must have write rights, which is a Pipln-controlled publisher. **Not worth it**: design 33 BitTorrent already gives a free, permissionless content transport. Do not build |
| 9 | **Mac App Store distribution** | Sparkle, the appcast and GitHub releases | Free | **High for the node.** The MAS sandbox does not fit a pre-login daemon (design 29) or a user-chosen external data disk. App Review decides release timing, which collides with activation heights (design 34) and puts Apple in the validator update path | — | It would make Apple a control point over validator software. **Not acceptable.** The Mac stays on Developer ID + Sparkle + design 19 approval |
| — | **iCloud Private Relay** | Nothing: it does not carry our P2P traffic | — | — | — | — |
| — | **Find My network** | Nothing: hardware accessory program only | — | — | — | — |

**Money and ops saved.** Rows 1-3 remove the only servers a consumer wallet
would otherwise need: push, sync and a private RPC for phones. That is
hosting, on-call and a privacy policy for user data that we would hold. We
avoid them at an effort of M for each. Row 4 removes the need for any backup
service. Rows 8-9 look like savings but cost us more in control and review
risk than they save.

## 10. Genesis impact

| Item | Genesis-required? | Where it lands |
|---|---|---|
| v2 account bytes (`0x…7702`, `0xdeaca4e6…`) | **Keep as is.** No change for anything here | — |
| WebAuthn guardians | **Post-launch.** Account v3 at a new CREATE2 address, together with ERC-7739 | Contract, audit, wallet redelegation |
| Indexer knows more than one account target | Post-launch | Node release (`chain.rs:2674-2687`) |
| `ffi` account target from config | Post-launch (could land before beta as pure refactor) | Wallet release |
| CloudKit relay, iCloud sync, App Intents, Handoff, lost-device flow, Watch | Post-launch | Wallet |
| Hardware binding of node keys, key dir apart from data dir | **Before beta is better, but not genesis** | Node + wallet release. It protects the testnet now |
| Local layers (a)-(c), (e) | Post-launch | Wallet + node RPC transport |
| `valid_until` in `TxHeader` for (d) | Post-launch protocol upgrade, only if (d) is built | Consensus (signing bytes), committee-signed activation |

**Genesis-required items: none.** One thing must stay true at genesis: v3
needs the predeployed Arachnid CREATE2 proxy (already in the reviewed genesis)
and a storage layout that only appends to v2's. Both are true today.

## 11. Lane plan

### Before beta (no genesis change; protects testnet validators now)

1. **N1. Hardware binding for node keys.** `crates/node/src/supervisor.rs`
   (create the binding with the key), new `crates/node/src/key_binding.rs`
   (IOPlatformUUID through IOKit, a hash, check), `main.rs` (exit code
   `EXIT_KEY_ELSEWHERE`), `apps/wallet/Sources/NodeController.swift` (a
   plain message). Tests: unit (mismatch refuses; missing binding on an
   existing key is created once and logged; a corrupt binding refuses);
   integration (copy a data dir to a fake UUID → the node refuses to vote).
   Fault test: binding check before the first vote of each epoch, run with a
   UUID that changes in the middle. **Every check must fail once** (lead
   rule).
2. **N2. Key dir apart from the data dir.** `--key-dir` default
   `~/Library/Application Support/…`. The external-disk mover never moves
   keys. Refuse keys found on a removable or external volume. Tests:
   `supervisor` unit tests for `KEEP_ACROSS_NETWORKS` under a split layout;
   e2e of the external-disk move with keys left behind.
3. **N3. Backup and sync exclusion.** `tmutil addexclusion` on the key files,
   `NSURLIsExcludedFromBackupKey`, refusal under an iCloud-synced Desktop
   or Documents. Test: pure-Swift policy test.
4. **N4. `ffi` account target from config** (refactor, no behavior change).
   Tests: existing ffi tests, plus one asserting that all delegate sites read
   the same value.

### After launch: wallet (ordered by value per effort)

5. **A1. Capability check (half a day).** On our team `LBKUT88RTX`, with a
   Developer ID provisioning profile: CloudKit, push, associated domains and
   the local network prompt on macOS 15+. Write the results into §12. Nothing
   else in this list starts until this is done.
6. **A2. AirDrop / share-sheet payment requests** (§8.2 c).
   `apps/wallet/Sources/PaymentRequest.swift` (new), the `eastsea:` URL
   parser and the receive screen. Tests: parser fuzz, a malformed request
   refused, the name-service name displayed, the e2e "share → open → confirm
   screen".
7. **A3. CloudKit relay** (§4). Mac: `InboxWriter.swift` (user-session
   agent). iPhone: subscription, Notification Service Extension,
   `InboxCrypto` (ECDH + AES-GCM). Tests: crypto vectors; a forged note
   (valid ciphertext, an event not on chain) is never shown as received; a
   replayed note is dropped; rate limit (merging); the relay off → nothing
   breaks. Fault test: CloudKit unavailable or throttled for 24 h.
8. **A4. iCloud sync of non-secret data** (§5). `SyncPolicy.swift` with the
   allowlist and the never-sync test. Contact-change banner. Tests: the
   allowlist test fails when a forbidden type is added; conflict and
   tombstone tests.
9. **A5. Lost-device flow + App Intents "Send"** (§7). Reuse the recovery
   FFI. Tests: intent always requires user presence; the F-05 text appears
   when the lost key is the original.
10. **A6. iPhone ↔ own Mac over LAN/AWDL** (§8.2 a). Node: an RPC listener
    bound for paired peers, TLS-PSK. iPhone: `NWBrowser` with
    `includePeerToPeer`. Tests: pairing, a wrong PSK refused, a lying Mac
    (a bad proof) is caught by the light client, and no advertising off
    home networks.
11. **A7. BGTaskScheduler catch-up** on iPhone. Test: the task handler is
    idempotent and bounded in time.
12. **A8. LAN block sharing between Macs** (§8.2 b): iroh local discovery in
    `crates/net`, gated by "home network". Test: two nodes on one LAN sync
    without internet. A peer serving a bad block is dropped.
13. **A9. Handoff** (§7). **A10. Local Push** (after the entitlement).
    **A11. Watch session key.** **A12. Offline intents** (only with a
    decision on `valid_until`).

### After beta: contract

14. **C1. Account v3 = ERC-7739 + WebAuthn guardians** (§3.3, §3.6). Port
    OZ `WebAuthn`. Append-only layout. Deploy through CREATE2. Audit with the
    list in §3.6. Node indexer for several targets (`chain.rs`). Wallet
    redelegation inside the next batch, only to a pinned code hash. Tests:
    the audit list, plus a live v2 → v3 redelegation drill on testnet with
    a pending recovery.
15. **C2. Passkey guardian UI**: setup ("Recover with iCloud"), Stolen
    Device Protection and ADP suggestions, the shared-passkey warning, and
    recovery on a new device. Test: the full loss drill: wipe every device,
    recover from a new iPhone with only iCloud, cancel from a surviving
    device in a second drill.

## 12. Facts to verify first (task A1 and C1)

| Fact | Used in | Status |
|---|---|---|
| Data protection keychain unavailable to launchd daemons | §6 | Documented in TN3137 and by Apple DTS. Re-check on the target macOS |
| CloudKit, push, associated domains for Developer ID Mac apps | §4, §5, §3 | verify (A1) |
| Local network prompt for a non-sandboxed Developer ID app and its helper on macOS 15+ | §8 | verify (A1) |
| iCloud Keychain PRF stable across synced devices | §3.7 | Reports say **no** (forum threads 774111, 822523). Re-test per major OS |
| OZ `WebAuthn` version and audit | §3.3 | verify (C1) |
| CloudKit public DB quotas | §9 row 8 | verify (only if row 8 is ever revisited) |
| Wi-Fi Aware platform coverage | §8 | verify (only if Android interop matters) |
| watchOS Secure Enclave keys for our target | §7 | verify (A11) |

## 13. Red-team cases

1. Hijack the user's Apple ID, take the passkey, propose recovery. Expect: a
   48 h window, a CloudKit note to every paired device, and a one-tap cancel.
2. Hijack the rpId domain, phish a passkey assertion through a web page.
   Expect: the same as case 1. Bound by `rpIdHash`, UV and the delay.
3. Forge a CloudKit note ("you received 1,000,000"). Expect: a banner at
   most, with no activity row and no balance change. The app later shows
   "this notice could not be confirmed".
4. Swap a contact's address through a hijacked iCloud. Expect: the "changed on
   another device" banner, and the on-chain name in the send screen.
5. Restore a Time Machine backup of a validator Mac onto a second Mac while
   the first one still runs. Expect: the copy refuses to sign
   (`EXIT_KEY_ELSEWHERE`). No equivocation.
6. Plug the external chain disk into another Mac. Expect: no keys on it.
   The other Mac's node starts as its own identity, or refuses.
7. A nearby attacker AirDrops a payment request with a look-alike name.
   Expect: the payer sees the on-chain name or "unknown address".
8. A Shortcut automation tries to send without the user. Expect: the SE's
   user-presence prompt. No payment without Touch ID / Face ID.
9. A LAN peer advertises `_eastsea._tcp` and serves bad proofs to the
   iPhone. Expect: the light client rejects it, and the pairing PSK refuses
   unpaired peers.
10. An offline intent is double-signed with the same nonce to two people.
    Expect: both receivers see "pending, not received" until finality; one
    gets nothing, and the wallet said so up front.
