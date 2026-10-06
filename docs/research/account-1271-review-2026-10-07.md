**EastSeaAccount ERC-1271 and genesis delta — security review**

Date: 2026-10-07 KST. Reviewed detached HEAD `befc8c42e90b0e37b5b8bd6e781decd6392c68b9`, against merge parent `6e8b587069d61d534b76bf917aaf9f046b0feaeb`. Scope: the merged `glm/account-1271` commits `6257061`, `ff6b31a`, `bbc6ff6`, `55042de`; adjacent wallet, execution, and ceremony code was read to establish boundaries. Legacy comparison also used `consensus-freeze-7` (`f31bc8fee0c30a7e1752028165a0458de9b746af`).

**Verdict: safe to keep for a fresh genesis.** No confirmed Critical or High defect requiring a change to the reviewed account runtime or predeploy bytes was found. There is a **conditional High signing-integration risk**, a **Medium extension of accepted F-05**, and three Low observations. This is not approval to expose blind hash signing or to apply these genesis artifacts to an initialized chain. Before launch, verify the final binary and identical genesis root across validators; before enabling owner message signing, enforce the signing requirements below. No source edits, commits, Cargo builds, or tests outside `contracts/` were performed.

| ID | Severity | Scope | Result |
|---|---|---|---|
| R-01 | High, conditional | v2 signature integration | Opaque hash signing can conceal a different application's authorization; shipped providers currently refuse signing. |
| R-02 | Medium | v2 runtime; F-05 extension | Recovery cannot revoke original-key permits/authentication; relayers can act without an account-originated transaction. |
| R-03 | Low | v2 receiver hooks | Unwanted NFTs/ERC-1155s and activity spam are accepted; their incoming state fees are paid by the transaction sender. |
| R-04 | Low | Genesis only | Ceremony JSON/record checks do not establish that all launch binaries derive the same new genesis. |
| R-05 | Low | Documentation/integrations | Low-s removes a malleable twin, not all distinct signatures for one message. |

**Findings and scenarios**

**R-01 — High, conditional: the wrapper prevents account/chain replay but does not establish informed application consent.** Locations: `contracts/src/EastSeaAccount.sol:102`, `:515`, `:518`, `:554`, `:562`; signing boundary: `apps/wallet/Sources/BrowserPolicy.swift:223`, `:229`, `apps/extension/src/lib/methods.js:5`, `:11`, `:12`, and `docs/design/09-wallet.md:202`.

The account signs an arbitrary `bytes32` under its own domain. It cannot recover the application name, contract, typed fields, or purpose from that hash. A malicious site could describe a request as login while supplying the digest of a real spending authorization for another application. If an owner signs that opaque digest's account wrapper, the intended spending application receives a valid ERC-1271 answer for its own digest. The prerequisite is a signing path that accepts the supplied hash without independently checking what it represents, plus whatever permissions the spending application requires. No cryptographic collision, account replay, session-key privilege escalation, or malicious transaction from the account is necessary.

This is **custom defensive rehashing, not ERC-7739's readable TypedDataSign workflow**. ERC-7739 retains the application domain and a user-defined contents type, carries reconstruction data in the signature, and checks that it reconstructs the caller's original hash. This account instead accepts exactly 128 signature bytes, provides no `eip712Domain()`, and has no ERC-7739 detection response or readable nested contents. Its final SHA-256 is also a custom P-256 convention, rather than ERC-7739's prescribed final Keccak-256. [ERC-7739 specification](https://eips.ethereum.org/EIPS/eip-7739)

If two applications compute genuinely different, properly domain-separated inner hashes, a signature for one will not validate for the other. If they reuse the same raw hash, the account accepts the same signature for either: `msg.sender`/application identity is not an additional binding. The phishing scenario above asks the user to sign the target application's hash in the first place; it does not transform an honestly signed different hash.

**Required signing boundary:** obtain the complete inner typed data or complete canonical login message; independently reconstruct its digest and compare it with `contents`; validate the active chain/account; show the requesting website origin separately from the actual application contract/domain. Show the action, token/asset and contract address, amount or unlimited allowance, spender and any bound recipient, nonce, expiration/deadline, and all material restrictions. Display unbound fields as unbound. A dApp-provided title or "EastSeaAccount / Contents / 0x…" is insufficient. Refuse hash-only requests that cannot be reconstructed; do not silently fall back to raw signing. Never expose this through session/agent keys.

The current browser/extension method allowlists exclude `personal_sign`, `eth_sign`, and `eth_signTypedData_v4`; the wallet routes unsupported methods to refusal. Thus this is a **release gate for future signing support**, not a demonstrated currently reachable wallet exploit. A custom wallet can preserve the present runtime if it enforces that boundary. For interoperable readable signing through ordinary typed-data clients, implement/review the ERC-7739 flow and the necessary P-256 adapter before advertising compatibility.

**R-02 — Medium: original-key ERC-1271 authority survives recovery, extending F-05 to off-chain approvals and identities.** Locations: `contracts/src/EastSeaAccount.sol:563`, `:565`, `:299`, `:306`, `:327`; baseline: `docs/research/contracts-audit-2026-10-05.md:105`, `:109`; existing user guidance: `apps/wallet/Sources/KeyExposureNotice.swift:20`, `:24`.

Scenario: guardians recover the account by adding a new owner and removing old registered owners, but someone retains signing access to the original P-256 device key. `_p256Address(k) == address(this)` still succeeds without consulting owners or recovery state. That key can now authorize ERC-1271 permits, orders, and application login challenges. A relayer can submit them using its own transaction and gas. Increasing `ownerNonce`/`recoveryNonce`, revoking sessions, or removing registered owners does not invalidate original-key message signatures. The account supplies no message nonce, expiry, or signature revocation mechanism; each consuming application must enforce its own.

**Added risk versus today's F-05:** original-key compromise already gives unrestricted top-level account transactions and redelegation, so the worst-case asset authority was already 100% of assets the original signer could move. ERC-1271 does not raise that ceiling or give sessions that power. It adds immediate permit/order and authentication support through the pinned runtime, sponsored execution, and a monitoring/revocation gap: draining an approved token need not advance the account's transaction nonce or appear as an outgoing account transaction. Future deposits may remain exposed while the application's authorization remains usable. For a permit-based transfer, exposure is bounded by the application's signed amount, available balance, required token/operator approvals, nonce, and deadline; authentication exposure depends on the relying service.

Current registered owners are independently sufficient signers, not a quorum. Removing every registered entry for a non-original owner key makes subsequent ERC-1271 checks fail. The original key and any remaining duplicate owner entry still validate (`addOwner` permits duplicates). Re-adding the same key can make previously unused signatures valid again. An allowance/order already materialized while a signature was valid may survive owner removal because its later execution need not recheck ERC-1271.

**Containment:** retain F-05's fresh-address migration guidance and expand it to application allowances/orders, sign-in sessions, and future deposits; monitor token/operator events and application activity, not just outgoing native transactions. Treat recovery as lost-key recovery, never stolen-original-key revocation. Optional contract signature epochs/disable flags could provide defense for cooperating applications, but cannot solve original-key top-level execution/redelegation. This remains an accepted architectural risk, not a complete contract-revocation fix.

**R-03 — Low: receiver hooks accept unsolicited assets and metadata/activity spam, without charging incoming fees to the recipient.** Locations: `contracts/src/EastSeaAccount.sol:583`, `:587`, `:591`; payer evidence: `crates/execution/src/block.rs:233`, `:373`, `:380`, `:386`, `:432`; pricing: `crates/execution/src/fees.rs:38`, `:39`.

Scenario: a third party safe-mints or safe-transfers dust NFTs/ERC-1155s to a delegated account, creating unwanted assets and history entries. All hooks return acceptance for any caller/operator/token; they neither authenticate the asset nor guarantee it can later be transferred or burned. Malicious token metadata can become a wallet phishing surface if trusted automatically. ERC-20 dust and non-safe ERC-721 delivery were already possible, so restricting these hooks would not eliminate unsolicited assets.

The hooks are pure, write no account storage, make no external calls, and grant no spending authority. Token ownership/balance writes reside in token storage. The executor reserves and settles growth/archive fees against `tx.header.sender`, even for changes involving another address. Therefore an unsolicited incoming transfer charges **zero additional fee directly to the receiving user's native balance**. The sender/relayer pays its transaction's gas, newly occupied token slots, and archived bytes. At the floor, each new occupied slot costs 100 × 10^12 wei = 0.0001 DBLN, before receipt/calldata and execution costs. A user who voluntarily sends/burns dust later pays that cleanup transaction; cleanup is not mandatory. Hide/filter unsolicited assets and avoid treating arbitrary metadata as trusted. No mandatory on-chain receiver restriction is warranted here.

**R-04 — Low, genesis only: stale ceremony binaries/records can validate JSON while deriving different genesis code.** Locations: `crates/node/src/chain.rs:179`, `:182`, `:195`; `crates/node/src/mainnet.rs:525`, `:552`, `:619`; `scripts/mainnet-genesis.sh:52`, `:54`, `:549`, `:550`.

The added predeploys and replaced v2 runtime change the computed new-genesis state root without changing the network JSON schema. The ceremony record's immutable fields and SHA-256 cover configuration/file bytes, not the computed genesis root, binary revision, or account/utility hashes. The launch script can select an existing binary and treats a successful checker exit as success while merely printing its rule count. A pre-delta binary can consequently validate the unchanged JSON with its older rule set, while the merged binary derives a different genesis. This is an inherited coordination gap exposed by the post-freeze delta, not a newly introduced attacker-controlled code update.

**Required launch action:** use an explicitly identified final binary on every launch validator; verify identical computed genesis roots and the runtime hashes below; expect 21 genesis rules, 26 ceremony rules, and 27 bundled rules. Reissuing a same-format record alone does not prove binary/root agreement. Consider pinning the root and artifact/binary identities in the ceremony evidence, and failing on an incomplete checker rule set. Never use this merge as an in-place migration of an initialized history-v2 chain. Existing stores fail closed on a different genesis (`crates/node/src/chain.rs:762`, `:764`), preventing silent state replacement; it does not replace prelaunch agreement. No initialized new-genesis deployment or mixed-binary launch was demonstrated in this review.

**R-05 — Low: low-s is not signature uniqueness or application replay prevention.** Locations: `contracts/src/EastSeaAccount.sol:557`, `:558`; `docs/design/09-wallet.md:112`.

The comments claim one signature per message. Low-s correctly rejects the `(r, n-s)` twin; different ECDSA signing nonces can still produce different valid low-s signatures for the same message. Scenario: an integrating application interprets those comments as permission to deduplicate signature bytes instead of consuming a signed nonce, allowing repeat authorization. Correct the wording and require message/application nonce handling. No reviewed consuming application was shown to make that error, and the account appropriately does not consume state during ERC-1271 verification.

**Checks that passed**

| Check | Evidence and boundary |
|---|---|
| P-256/low-s | `EastSeaAccount.sol:104`, `:181`, `:554`: half-order constant is exactly floor(n/2); blob length is exactly 128; P256VERIFY must return exactly 32 bytes containing 1. Zero/out-of-range scalars, invalid/off-curve/infinity points are rejected by the precompile. The low-s twin rejection passed on the real Osaka precompile. No absent-precompile empty-return bypass exists. |
| Cross-account/cross-chain replay | `EastSeaAccount.sol:515`, `:518`, `:525`: signed domain contains runtime `block.chainid` and `address(this)`, hence the delegating account, not shared target `0x…7702`. Tests cover a common owner on two accounts and chain-ID changes. Chains/forks sharing both account and chain ID are not distinguished; launch must use its intended distinct chain ID. |
| Signer authorization | `EastSeaAccount.sol:562`–`:568`: verified original address-derived key or a current registered owner only. Guardian/session arrays are not authorization sources. Session limits, expiry, and agent status grant no ERC-1271 permission. Test with an unlimited session still fails. A key deliberately also installed as an owner/original has full owner authority; there is no disjoint-role prohibition. Keep session/agent keys distinct from owner keys. |
| Address derivation | `EastSeaAccount.sol:573`, `crates/crypto/src/lib.rs:115`: scheme byte 0x01 plus compressed SEC1 prefix determined by y parity and x, Keccak-256's low 160 bits. Point validity is checked before this comparison. No uncompressed/compressed mismatch was found. |
| Both selectors | `EastSeaAccount.sol:541`, `:549`: standard magic 0x1626ba7e, legacy magic 0x20c13b0b; exactly 32 bytes of legacy data become the same inner hash; other data/blob lengths return invalid. No alternative bare-hash acceptance. Legacy support is deliberately limited to hash data, not arbitrary Safe legacy messages. |
| Digest convention | P-256 signs SHA-256 of the 66-byte 0x1901/domain/struct envelope. This is sound custom domain separation, but differs from ordinary EIP-712's final Keccak-256. A signer adapter must pass the 66-byte message to a SHA-256-signing Secure Enclave API, or sign its SHA-256 digest through a prehash API; avoid double hashing or substituting the ordinary typed-data digest. |
| ERC-165/receivers | `EastSeaAccount.sol:601`: accepts ERC-165, ERC-1271, ERC-721 receiver, aggregate ERC-1155 receiver interface 0x4e2312e0; refuses reserved 0xffffffff/unknown IDs. Receiver selectors and ERC-165 cases passed. Hooks cannot reenter spending because they perform no calls/writes. |

Precompile invalid-input rules were checked against [EIP-7951](https://eips.ethereum.org/EIPS/eip-7951) and the locally installed `revm-precompile-43.0.3/src/secp256r1.rs:86`, `:124`, `:128`, `:130`. The custom signing convention is distinguished from [EIP-712](https://eips.ethereum.org/EIPS/eip-712); ERC-1271 permits contract-defined validation and requires read-only verification, which this path satisfies. [ERC-1271](https://eips.ethereum.org/EIPS/eip-1271)

**Storage compatibility**

`contracts/src/EastSeaAccount.sol:109`, `:111`, `:128`, `:136`, `:154`: `STATE_SLOT`, `_state()`, and the complete `Call`, `Key`, `State`, `TokenLimit`, and `Session` declarations are byte-identical to the merge parent. New constants occupy no storage; new signature views and receiver hooks introduce no writes or initialization. Existing v2 delegated-account guardians, pending recovery, owners/nonces, sessions/serials and token limits retain their slots and packing.

The legacy artifact's last regeneration (`308861d`, then named `AetherAccount.sol`) also used the same namespace, `Key`, and `Session` layout. Its `State` is the current prefix through `sessionSerial`; the token-limit mapping was appended later, before this delta. This delta adds no layout migration. This compatibility observation is not permission to replace the immutable 7780 artifact or arbitrarily redelegate to unrelated code. No live account upgrade/migration was run.

**Runtime and predeploy evidence — genesis-only artifacts**

| Artifact/address | Runtime bytes | Computed Keccak-256 |
|---|---:|---|
| Legacy `aether_account.bin.hex`, target 0x…7702 on 7780 | 12,156 | `0x0b9ba7215eb4aee404f884d94f74c5d5279cdfa5dc7cb85b028c045cf17a9433` |
| New `aether_account_v2.bin.hex`, target 0x…7702 | 17,480 | `0xdeaca4e6cc9787c233aeec5034a8884cf5ced4c6899299b3e76027a03f85288a` |
| Arachnid CREATE2 proxy, `0x4e59b44847b379578588920cA78FbF26c0B4956C` | 69 | `0x2fa86add0aed31f33a762c9d88e807c475bd51d0f52bd0955754b2608f7e4989` |
| Multicall3, `0xcA11bde05977b3631167028862bE2a173976CA11` | 3,808 | `0xd5c15df687b16f2ff992fc8d767b4216323184a2bbc6ee2f9c398c318e770891` |

The reproduced account build used `solc 0.8.19+commit.7dd6d404`, optimizer enabled/200 runs, no remappings/libraries or via-IR, compiler target Paris. Although Foundry executes tests as Osaka, it normalizes the older compiler's target to Paris. **The entire runtime, including its 53-byte metadata trailer, is byte-identical to the reviewed v2 pin**; this is stronger than an executable-core-only match. It also matches `fixtures/artifacts.json`'s `core/EastSeaAccount.runtime`; source/config SHA-256 and fixture-manifest SHA-256 match `fixtures/provenance.json`. The account is below the 24,576-byte runtime limit.

Independent upstream-byte verification extracted the utility runtimes from their published deployment transactions: Arachnid's 83-byte initcode returns 69 bytes; Multicall3's 3,840-byte initcode returns 3,808 bytes. Both match the local files exactly, including Multicall3 metadata; all computed hashes match `crates/execution/src/predeploys.rs:24`, `:29`. [Arachnid canonical deployment](https://github.com/Arachnid/deterministic-deployment-proxy/blob/master/README.md), [Multicall3 canonical deployment](https://github.com/mds1/multicall3/blob/main/README.md#new-deployments)

Both utilities are constructor-storage-free and permissionless, with no owner privilege to transplant. Genesis begins from an empty world and allocations set balances only (`chain.rs:172`); `WorldState::set_code` preserves any allocated balance (`world.rs:248`, `:252`). Addresses do not collide with the system targets. Nonempty code prevents CREATE/CREATE2 replacement even though genesis does not recreate the original deployment nonce. Multicall3 is a public call aggregator, not a wallet custodian: integrations must not grant it token allowances or assume calls originate from the user's account. Its upstream deployer-key compromise does not give authority over this directly installed runtime.

**Genesis/mainnet boundaries and legacy immutability**

`crates/execution/src/lib.rs:40` selects v2 when `node_rewards || history_v2`; the new utility installation at `crates/node/src/chain.rs:182`, `:195` requires **both** flags. This distinction is intentional: strict mainnet checks require the complete new-genesis configuration. `mainnet.rs:88` recomputes actual installed utility code hashes; rehearsal does not exempt that check. Rule counts and script-test expectations consistently advance to 21/26/27. The new rule checks utilities, while the account pin is asserted separately in `crates/execution/tests/delegation.rs:24`; this audit independently checked that pin and its build provenance.

The shipped 7780 file has neither feature flag, selects the unchanged legacy account, and installs neither utility. Binary and text-file equality were checked against **both** the merge parent and freeze-7:

- `crates/execution/src/aether_account.bin.hex`: byte-identical, including whitespace; file SHA-256 `64f1dbc64ba849b45f396a8c775c1aa13cce986bb9b78108a279ea2db8353076`.
- `apps/wallet/Resources/network.json`: byte-identical; SHA-256 `26faa6bca43e2c1f458ccd4051edb92665efe8939a3ef60652ab92c528b7b9cc`.

**Executed verification and limits**

From `contracts/`, with all temporary/compiler output redirected under this worktree's `tmp/`:

```sh
env TMPDIR="$REVIEW_ROOT/tmp/acct-verification" FOUNDRY_PROFILE=default \
  forge test --root . --offline --threads 4 \
  --out "$REVIEW_ROOT/tmp/acct-verification/forge-out" \
  --cache-path "$REVIEW_ROOT/tmp/acct-verification/forge-cache" \
  --build-info --build-info-path "$REVIEW_ROOT/tmp/acct-verification/build-info"
```

`REVIEW_ROOT` denotes `/Volumes/workspace/aether-node/.claude/worktrees/codex-acct-review`; outputs include the rebuilt account artifact and compiler input/output. Foundry `1.6.0-nightly` (`5e88010a83d1b87b8f4d13058e42a2949d3e9dc0`) compiled 27 files successfully. **172 tests passed across 14 suites, zero failed/skipped**, including all ten new ERC-1271 tests with real P256VERIFY, session-payment tests and receiver/ERC-165 assertions. Existing compiler warnings concern unrelated test fixtures (fallback/unused variable/mutability).

Additional read-only checks used Git object/file equality, `cast keccak`, Python artifact/provenance comparison, source-layout comparison, and independent code review. No new attack harness was created. Rust genesis/mainnet/delegation and contracts-onchain tests were inspected but **not executed**, per the no-Cargo/contract-only constraint. The external toolbox's actual OZ safe-transfer tests were likewise inspected, not run. No live genesis-root computation, launch rehearsal, Secure Enclave signing/UI flow, or deployed-app permit/login flow was exercised; those limits bound the verdict. Shell DNS prevented direct Jina/RPC fetching, so upstream provenance used primary repository documents through the web tool rather than claiming fresh multi-chain `eth_getCode` observations.

**Required follow-through:** keep the verified bytes for fresh genesis; complete final-binary/root agreement before ceremony/launch; keep opaque signing refused until a reconstructing, informative owner-signing flow exists; preserve original-key compromise/migration warnings and application-level nonce/revocation handling. No mandatory Solidity or predeploy-byte replacement was established by this review. The only authored report is `tmp/acct-review.md`; source simplifications: none (read-only audit).
