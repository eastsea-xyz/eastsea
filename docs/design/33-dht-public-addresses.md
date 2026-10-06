# 33. DHT public addresses

Date: **2026-10-06**. Status: **design only; nothing here is implemented by this document**.
Scope: EastSea public entry points in the Aether codebase. Native currency: DBLN.

Founder: “우리는 노드 주소를 토렌트에 무임승차하는데, 공개주소도 무임승차하면 안됨?”
and “한계를 우회 가능?” Addendum: “많은 지갑앱들이 브라우저를 가지고 있는걸 알고 있지?”

## 0. Decision and boundaries

**Make the EastSea wallet's in-app browser the primary entrance.** Its installed
native layer resolves names, discovers peers, verifies chain state and app bytes,
then serves a private `eastsea-app://` origin to WKWebView. Opening an app needs
no HTTP landing page, Pipln domain, Pipln RPC, or Pipln content server.

Use Mainline DHT as a replaceable locator and BitTorrent as a content transport.
Chain state authorises names and active releases; cryptographic hashes select
bytes. DHT signatures authorise a locator update under one key, never a release,
a payment, a committee, or a change to the chain's trust anchor.

There are three entrances with different capabilities:

| Entrance | App entry and content | Chain transport | Initial trust |
|---|---|---|---|
| **EastSea wallet, primary** | Native `eastsea://name` resolution → verified peer/torrent cache → `eastsea-app://appKey/` | Native iroh, direct when possible; replaceable relays when necessary | Installed verifier and authenticated network configuration |
| EastSea extension / verified explorer | Packaged verifier; content from peers or any mirror | iroh-wasm through WebSocket relays; pkarr HTTP lookup of known keys | Extension package or independently authenticated loader |
| **No-install web, onboarding** | Any community HTTPS mirror | Same browser transports after authenticating the loader | Ordinary first-load web origin unless an independent loader check is performed |
| Standard wallet, future compatibility | Publisher/community HTTPS entrance to a dApp | Standard Ethereum JSON-RPC to an independent operator | Wallet's own key handling; RPC reads ordinarily remain trusted |

“Any mirror works, none is trusted” is achievable for **data and bundles behind an
already authenticated verifier**, including the native wallet. A fresh ordinary
browser cannot authenticate its first JavaScript using that JavaScript alone.
Section 6 makes that limitation and the optional no-install verification procedure
explicit. There is no promise that all Internet servers or all domains disappear.

**Consensus impact:** public discovery, names delegation, torrents, scheme serving,
and browser transports require **none**. The standard-wallet transaction lane in
§10 is a separate **consensus/protocol upgrade**, deferred from this work.

This document supplies networking mechanisms, not new app eligibility rules.
[31-app-registry](31-app-registry.md) remains authoritative for release delays,
permissions, classification, and legal release gates. The founder addendum makes
the existing native browser the primary transport path; it does not approve an
iOS app catalogue or override that document's iOS pure-wallet boundary.

## 1. Read-first evidence and current gaps

Read against this worktree on 2026-10-06:

| Evidence | What exists; what this design adds |
|---|---|
| [Public read research](../research/public-read-access-2026-10-05.md), [WASM measurements](../research/wasm-speed-2026-10-05.md) | Replaceable proof providers; roughly 11.7 ms isolated BLS WASM verification on M1 Max/Node 22. These are neither browser latency measurements nor end-to-end targets. |
| [App discovery research](../research/app-launch-discovery-2026-10-05.md), [31](31-app-registry.md), [26](26-name-service.md) | 31's 2026-10-06 revision supersedes the research's older Pipln-domain landing pages, recommendations, deposits, and default lists. Registry is a design, not deployed code. Names has four bounded text slots. |
| [crates/net](../../crates/net/src/lib.rs), [p2p.rs](../../crates/node/src/p2p.rs) | iroh 1.2.0, mainline address adapter 0.5.0, known EndpointId → unsalted pkarr addresses; authenticated `aether/p2p/1`, bounded `aether/rpc/1`. No public per-chain provider enumeration yet. |
| [Wallet-server admission](../../crates/net/src/lib.rs), [announce.rs](../../crates/node/src/announce.rs) | Registered candidate signature bound to EndpointId; finalized-registry check; 15-minute list TTL, 4,096 retained entries, 64 sampled across operators. Open DHT discovery must not weaken this separate admission rule. |
| [Read gateway](../ops/read-gateway.md), [archive node](../ops/archive-node.md) | Read-only allowlist/caps already exist. Archive exports verified 8,192-block eras, signed manifests and v1 torrents with 256 KiB pieces. Exporting a torrent does not start a seeder. Tailnet HTTP hints are not public reachability. |
| [Explorer RPC](../../apps/explorer/js/rpc.js), [extension RPC](../../apps/extension/src/lib/rpc.js), [extension network](../../apps/extension/src/lib/network.js) | Explorer currently defaults to loopback plus `https://rpc.eastsea.xyz`; extension defaults to loopback and probes `eth_chainId`. HTTP endpoint selection is not DHT discovery or chain-proof verification. |
| [BrowserController](../../apps/wallet/Sources/BrowserController.swift), [BrowserOriginPolicy](../../apps/wallet/Sources/BrowserOriginPolicy.swift) | Merged WKWebView supports HTTPS and bundled `eastsea-page://` explorer. Bare names become HTTPS hosts. `eastsea-app://`, native name resolution and torrent fetching are proposed additions. Current bridge injection uses the page world. |
| [light verifier](../../crates/light/src/lib.rs), [WASM bindings](../../crates/wasm/src/lib.rs), [FFI verified_release](../../crates/ffi/src/lib.rs) | Storage/code-hash proofs exist. ReleaseLog reads already require all slots at one state height under one certified anchor. Use this precedent for Names/Registry reads. |
| [Block payload](../../crates/light/src/block.rs), [explorer verify](../../apps/explorer/js/verify.js) | New code has optional `receipts_root` and `verifyReceipt`; legacy blocks lack commitments and some browser UI paths still say “not committed.” Receipt inclusion does not prove current Names/Registry state or log-range completeness. |

The current network configuration pins chain ID, a 96-byte MinSig BLS group
identity, and initial endpoint IDs. It has no explicit genesis-digest field.
Do not hash its mutable JSON bytes to identify a chain, replace its identity from
a DHT answer, or treat a successful `eth_chainId` response as authentication.

## 2. Keys and record formats

All formats below are proposals. Binary integers are big-endian; hashes are raw
bytes on the wire and lowercase hex in diagnostic JSON. No private key, account
secret, or payment argument belongs in discovery records.

### 2.1 Chain namespace and public topic

The authenticated client configuration adds a public genesis digest:

    chain_id : u64
    identity : canonical MinSig group public key, 96 bytes
    genesis  : canonical genesis block digest, 32 bytes
    C = SHA256("eastsea-chain-v1\0" || u64(chain_id) || identity || genesis)
    g = consensus group, u16; initially 0

`C` is 32 bytes. Genesis is checked against the installed configuration or
authenticated release, not first learned from a gateway. Publishing this extra
client configuration field is **not a genesis or consensus change**. A different
genesis or group key requires explicit network selection. Future group identities
must come from authenticated chain configuration, not an advertisement.

Use a real, deterministic, tiny rendezvous torrent rather than pretending an
iroh UDP port speaks BitTorrent:

    payload = "eastsea-rendezvous-v1\0" || C || u16(g)
    info = { length: len(payload), name: "eastsea-rendezvous-v1",
             "piece length": 16384, pieces: SHA1(payload) }
    T = SHA1(canonical_bencode(info))                    // 20 bytes

`info` keys use canonical bencode ordering. This single-file torrent has one
piece and no tracker requirement. `T` is the per-chain/group BEP-5 topic for read
gateways, archives, app seeders, and service introducers. Announcers seed this
small payload and implement the introduction extension in §3.1. Roles filter
the verified contact records; adding a role does not create a founder-owned list.
BitTorrent v1 uses the encoded `info` dictionary's SHA-1 as its infohash
([BEP-3][b3], checked 2026-10-06).

### 2.2 Endpoint addresses and chain-specific contact records

Keep the existing **unsalted pkarr slot**:

    N = iroh EndpointId / Ed25519 public key             // 32 bytes
    A = SHA1(N)                                         // 20-byte BEP-44 target
    value = existing iroh DNS address packet

Do not overwrite that slot with an EastSea-specific binary descriptor.

A separate **raw BEP-44 mutable item** advertises services:

    salt = "eastsea-contact-v1\0" || C || u16(g)           // 53 bytes
    target = SHA1(N || salt)                            // 20 bytes
    k = N; sig = Ed25519 signature (64 bytes)
    seq = positive i64, persisted monotonically per (N, salt)
    v = bencoded byte string containing ContactV1
    signable = bencode(salt field) || bencode(seq field) || bencode(v field)

The signature follows BEP-44's field encoding, without an enclosing dictionary.
Verify both the key-derived target and signature. BEP-44's portable limit is
1,000 bytes for encoded `v`; salt is at most 64 bytes. It may expire after about
two hours without republication ([BEP-44][b44], checked 2026-10-06).

**ContactV1 layout, in order:**

| Field | Bytes / bound |
|---|---|
| Magic `ESCA`, version `1`, roles | 4 + 1 + u16 |
| `C`, chain ID, group, node key `N` | 32 + 8 + 2 + 32 |
| `issued_s`, `expires_s`, `height_hint` | 8 + 8 + 8; height is a hint |
| ServiceIndex SHA-256 and byte length | 32 + u32; both zero when absent |
| Direct addresses | u8 count ≤4; each family:u8, IP:4/16, port:u16 |
| iroh relay URLs | u8 count ≤2; each u8 length + ≤128 ASCII bytes |
| Optional HTTPS service base | u8 count ≤1; u8 length + ≤192 ASCII bytes |
| Candidate binding | u8 flag 0/1; if 1, voting key32 + existing wallet-server signature64 |

Role bits: `0x0001 read/proofs`, `0x0002 archive`, `0x0004 app-content`,
`0x0008 native-tx-submit`, `0x0010 service-introducer`. Unknown bits confer
no capability. A server without a candidate binding is allowed to offer public
data; it is not thereby a registered wallet server or a validator.

Fixed overhead is **145 bytes**, including counts and binding flag. Worst case:
`145 + 4×19 + 2×129 + 193 + 96 = 768 bytes`. The byte string's encoded
`v` is **772 bytes** (`768:` plus the payload). Reject raw values above 900 bytes
as an early bound, then
require the exact v1 layout and the tighter field limits. Test the entire KRPC
packet against a 1,200-byte budget; `k/sig/salt/seq` and KRPC overhead are not
part of `v`. Do not insert a full pkarr signed envelope inside `v`.

ServiceIndex is a separate UTF-8 JSON document, **≤8 KiB**, fetched from the
advertiser through iroh or its optional HTTPS base. Check its exact byte hash
against ContactV1 before parsing. It describes RPC capabilities, served era
ranges, up to eight relay/lookup/tracker/mirror URLs, and up to 32 peer IDs.
It conveys locations and claims, not permissions or certificate trust. Large
lists, torrent metadata, and bundles do not go into Mainline storage.

The current vendored [mutable decoder](../../vendor/n0-mainline/src/common/mutable.rs)
verifies signatures but accepts a caller-provided target; the caller must still
recompute it. [Server limits](../../vendor/n0-mainline/src/core/server.rs) are not
a substitute for these client checks.

### 2.3 Owner-delegated pkarr locator

An EastSeaNames owner is an **account address**, potentially a P-256 passkey
account or a contract. It is not intrinsically an Ed25519 pkarr key.
Use existing owner-only text writes, consuming two of four available slots:

    text["app"]   = "0x" + hex(appId)                     // 66 ASCII bytes
    text["pkarr"] = z-base-32(K)                         // 52 ASCII bytes

`K` is a separate Ed25519 locator key generated by the owner/publisher.
The owner authorises it with an ordinary on-chain `setText` transaction.
This is a client convention, **no Names contract change**. Never derive the
locator secret from a public address or assume Secure Enclave stores Ed25519.
Store/wrap the local signing key using existing protected-key mechanisms.

Use standard pkarr's unsalted target `SHA1(K)`. Its DNS response contains one
TXT RR at `_eastsea.<z-base-32(K)>.`, TTL 300 seconds:

    TXT = "es1=" + base64url_no_padding(LocatorV1)

Split it into consecutive DNS character strings of at most 255 bytes.
No other answer, authority, question, or additional records in this v1 packet.
The record is application TXT data, not a DNS authority or CA credential.
Pkarr distributes signed DNS records through BEP-44 and exposes HTTP lookup
relays for browsers ([pkarr][pkarr], checked 2026-10-06).

**LocatorV1 layout:**

| Field | Bytes |
|---|---|
| Magic `ESLN`, `C`, group, name node, appId | 4 + 32 + 2 + 32 + 32 |
| Active registry seq | u32 |
| manifestHash, bundleHash | 32 + 32 |
| Torrent v1 infohash, metainfo SHA-256 | 20 + 32; zero means absent |
| Metainfo bytes, archive bytes | u32 + u32 |
| `issued_s`, `expires_s` | u64 + u64 |
| Locations | u8 count ≤3; each type:u8, length:u8, value ≤128 bytes |

Location type 1 is an iroh EndpointId (exactly 32 bytes); type 2 is an HTTPS
content base (≤128 ASCII bytes). Reject other types in v1. Fixed size is
**247 bytes**, maximum **637 bytes**. Base64url is at most 850 characters;
with `es1=`, TXT length octets, a 63-byte uncompressed RR owner name, 12-byte
DNS header and 10-byte RR header, the DNS packet is **943 bytes**, encoded BEP-44
`v` **947 bytes**. A complete pkarr packet adding key32/signature64/timestamp8
is **1,047 bytes**; that outer packet is not the DHT value. These are worst-case
format calculations, to become encoder golden tests.

The pkarr timestamp/sequence and registry release sequence are different
counters. The former only orders locators. A locator is usable for an app only
when its chain/group/name/app/release hashes match independently verified chain
state. Its torrent fields must agree with the authorised manifest's descriptor
if present. Mismatches trigger another locator/path, never an app update.

Name transfer preserves existing text records in
[EastSeaNames.sol](../../contracts/src/EastSeaNames.sol). A new owner should
clear/rotate `pkarr` and review `app`; otherwise delegation is inherited.
During rotation only the key in current certified Names state is authorised.
Even an inherited/stolen locator key cannot replace the registry's active bytes.

## 3. Gateway and archive discovery

### 3.1 Enumerating unknown keys: the missing step

BEP-5 returns peer IP/port contacts, not iroh endpoint keys or URLs
([BEP-5][b5], checked 2026-10-06). BEP-44 resolves a known key; it cannot enumerate
all publishers by a shared topic. A singleton mutable “gateway list” would have
one signing-key controller and is rejected.

Publicly reachable introducers announce **their TCP BitTorrent listener port**
under `T`, obtaining a normal BEP-5 write token first. Do not announce the
iroh QUIC port or use `implied_port` for an unrelated TCP listener. Proposed
flow, using [BEP-10 extensions][b10] (checked 2026-10-06):

1. Native client `get_peers(T)` obtains bounded untrusted socket contacts.
2. TCP BitTorrent handshake must name `T`; exchange extension handshake
   advertising `es_contact_v1`. Negotiated message IDs, not fixed IDs.
3. One `es_contact_v1` response carries that peer's BEP-44 contact envelope:
   `k, salt, seq, sig, v`. Max **1,200 bytes**, handshake deadline 2 seconds.
   The small rendezvous file remains seedable to ordinary torrent clients.
4. Verify §2.2, then dial iroh using `N` and signed addresses; authenticate the
   remote EndpointId. A malicious introduction can only add an untrusted candidate.
5. Request live proofs and bounded peer exchange through `aether/rpc/1`.

The extension is new networking work. Ordinary torrent peer IDs are not
EndpointIds. A peer without the extension is skipped, without probing its HTTP
or arbitrary ports. Public introducers may also return other contacts after
an authenticated iroh connection, under the peer-exchange caps below.

The current Mainline API is IPv4-only for standard `get_peers`
([vendor API](../../vendor/n0-mainline/src/dht.rs)). CGNAT/relay-only and IPv6-only
EastSea providers publish their signed contact item but do not advertise an
unreachable IPv4 TCP listener. Reachable introducers, existing registered
wallet-server lists, known IDs, and gossip introduce their keys. Their own
service can remain entirely behind NAT and be reached through iroh.

The vendored `unstable_signed_peers` extension can optionally introduce
`(key32, timestamp8, signature64)` directly; then use normal pkarr address
resolution. It is labelled `BEP_????`, its public API is not enabled by the
current adapter dependency, and adoption is not established. The all-legacy
test network returns no peers. It is an optional optimisation, **never the
baseline or sole recovery path**.

### 3.2 Publication and expiry

| Item | Proposed cadence | Acceptance/cache limit |
|---|---|---|
| BEP-5 topic announcement | Every 15 minutes, ±20% jitter; immediate after reachable-port change | Contacts are hints; probe live |
| Signed ContactV1 / LocatorV1 | Re-sign every 20 minutes and on changes; re-put same latest item every 30 minutes, ±20% | `expires_s ≤ issued_s + 7,200`; future issue skew ≤120 seconds |
| Existing iroh unsalted address packet | Keep adapter behaviour initially; publish promptly on address/relay change | DNS TTL and successful EndpointId-authenticated connection |
| DHT retry | 5 s exponential backoff to 5 min, with jitter and bounded lookups | Use alternate paths during retry |
| Last-known-good peer | Keep up to 128 contacts for 7 days as reconnection hints | Expired contact never proves current reachability or chain freshness |

BEP-44's approximate two-hour expiry is storage policy, not an availability SLA
or app-release expiry. Re-putting identical signed bytes refreshes DHT storage,
not `issued_s/expires_s`. Subscribers may re-put valid records; only the
key holder can renew signed time. Keep one writer and persist seq high-water
marks. A lost writer state requires recovery of the latest seq or key rotation,
not publishing a lower sequence.

The pinned address adapter currently uses a short DNS TTL and hourly re-put of
the same signed packet. Its address packet is distinct from our expiring
service descriptor. Do not apply ContactV1's signed-time requirement blindly to
unchanged iroh address records.

### 3.3 Lookup, selection, and verification flow

Native wallets and nodes race three paths, starting with a usable cache:

- Mainline topic lookup through routing-table contacts and at least three
  operator/network-diverse bootstrap routes; standard BEP-5 introductions.
- Unsalted pkarr lookup of bundled validator IDs, user-supplied IDs and previously
  successful providers; request registered wallet-server lists from multiple nodes.
- Bounded `aether_publicPeers` exchange from the first connected EastSea node,
  then gossip from additional nodes. These are proposed read methods.

Browser/extension clients cannot send Mainline UDP. They use at least two
independently operated pkarr HTTP relays to resolve **known keys** from the
packaged configuration, cache or user contact capsule, then use iroh-wasm.
Ordinary pkarr relays do not enumerate BEP-5 topics or necessarily support raw
salted ContactV1. Fetch those envelopes from the authenticated EastSea peer.
An optional community DHT-to-HTTP topic bridge may return bounded candidates,
but it is not required when a known peer works and never decides authority.

For each candidate:

1. Enforce record bounds, chain/group, key/target, signature, seq and signed time.
   Persist high-water seq only for correctly scoped, verified records.
2. Enforce public network policy. Reject loopback, private, link-local,
   unspecified, multicast, CGNAT/Tailscale and reserved destinations in public
   advertisements; LAN discovery is separate and opt-in. Current net filtering
   permits some RFC1918 addresses, so this stricter filter is new work.
   For advertised HTTPS/relay hostnames, validate resolved destinations and
   every redirect against the same policy; permit at most three HTTPS redirects.
   Signature validity does not permit DNS rebinding or probes of local services.
3. Prefer a responsive proof-capable follower, spread across verified candidate
   operators when available and across address prefixes/relays otherwise.
   Roles, IP diversity and low RTT are not proof of independent ownership.
4. Dial two providers concurrently; authenticate iroh EndpointId. Fetch a live
   finalization, verify the installed group key/group and expected chain context,
   freshness and persistent height/digest floor, then verify the requested proof.
   On cold start, also authenticate the configured genesis digest in the
   certified history (or an already authenticated checkpoint/ancestry proof);
   a claimed chain ID cannot distinguish chains that reuse a group identity.
5. A certificate at height H authenticates `parent_state_root` for H−1.
   Use one state height and one anchor for all Names/Registry slots. The RPC
   string “latest” does not make separately sampled answers coherent.
   Authenticate the H−1 block's timestamp through parent/history linkage and
   use that timestamp for the state view, not certificate H's later time.
6. Use the first cryptographically valid fresh answer. Retain another provider
   for failover; cross-check heads for freshness. Three agreeing RPCs cannot
   substitute for a proof. Conflicting valid certificates at one height are a
   chain safety failure and stop signing; do not vote on them.

Read gateways keep the [existing allowlist and cost caps](../ops/read-gateway.md).
Add only bounded discovery/proof methods needed by this design. An archive's
era range is tested with a requested era proof and certified history root.
Archive chunks are a separate bounded content capability, not permission to
turn the read-only gateway into an unrestricted archive RPC.

For payments, discover a provider with `native-tx-submit`, submit the already
signed native envelope through the existing native RPC, and verify inclusion
where supported. Read-only endpoints remain ineligible. Receipt status must be
distinguished from transaction inclusion on legacy networks. Never enable
faucets, owner operations, snapshots or unrestricted node-local methods through
public discovery.

## 4. Content distribution: apps, intro site, explorer, eras

### 4.1 Authority hashes and torrent hashes are different

[31 §4](31-app-registry.md) specifies:

    manifestHash = SHA256(exact manifest bytes)
    bundleHash   = SHA256(canonical eastsea-bundle/1 index bytes)
    fileHash     = SHA256(each file)
    torrentId    = SHA1(bencoded torrent info dictionary), 20 bytes

Neither a tar hash, an iroh BLAKE3 blob ID, nor a v1 infohash is `bundleHash`.
Transport hashes help transfer; the chain-authorised index and file SHA-256
checks remain mandatory before execution.

Put a transport descriptor in the existing extensible manifest, rather than
adding an on-chain torrent field:

    "x-torrent-v1": {
      "infohash": "<40 lowercase hex>",
      "metainfo_sha256": "<64 lowercase hex>",
      "metainfo_bytes": 2450,
      "archive_bytes": 812032,
      "container": "ustar-deterministic"
    }

These are illustrative sizes. The manifest hash is pinned in the registry, so
this pins the torrent identity transitively. Old consumers ignore `x-*`; new
consumers check this extension's bounds. This needs a **client/CLI format
convention**, not a registry ABI or consensus change.

Avoid a hash cycle: the torrent contains `bundle.json` and its files,
**not the outer manifest containing the torrent descriptor**. Fetch the manifest
first by its known SHA-256 from the peer content protocol. The torrent may be
a single deterministic tar; lazy individual-file requests can concurrently use
the proposed `aether/apps/1` SHA-256 protocol from 31.

Limits: manifest ≤64 KiB; bundle index ≤1 MiB; preserve 31's 2,000 files,
10 MiB/file and 25 MiB total content; deterministic tar ≤32 MiB including
index/padding. Metainfo ≤64 KiB, 256 KiB pieces. At the tar ceiling the v1
piece-hash table is `128×20 = 2,560 bytes`. All limits apply before allocation
and extraction; reject symlinks, path escapes, case-fold duplicates and
undeclared files. Hash the extracted index and every served file.

Use magnets and bounded [BEP-9 metadata exchange][b9] to obtain `info` without
a metainfo server. Verify the infohash before requesting pieces. Full metainfo
outer tracker/webseed fields are not committed by the infohash; authenticated
manifest hashes or local policy select them. Added community locations are
always hints. SHA-1's legacy strength is not the application's authorisation
boundary because index/files additionally require SHA-256.

The intro site and explorer follow the same content format and transport rules.
After AppRegistry deployment their release manifest hashes can be registered
like any other publisher's app, with no discovery preference. Before that,
authenticated client releases pin their manifests, or an independent user
pins a publisher-signed manifest/key out of band. A mirror's self-chosen key
does not authenticate an “official” release. The bundled explorer remains a
usable starting point even if all remote bundles disappear.

### 4.2 Seeders, WebTorrent and optional webseeds

Desktop nodes that use/add an app keep and seed its verified active bundle,
subject to an explicit bandwidth policy and an off switch. Preserve 31's staged
seeding policy: no surprise public uploading in the first implementation;
default seeding of added apps belongs to its later phase.

Proposed limits: content cache **500 MiB**, active/previous versions pinned within
that ceiling, eight upload peers, aggregate 1 MiB/s and 256 MiB/day by default.
Resource configuration can lower these; no pin is allowed to bypass the disk
budget. A new pin that does not fit needs eviction/user storage choice.
Wallet mobile devices need not remain background seeders. Archives use their
own disk-retention policy for era torrents.

Browsers use **WebTorrent/WebRTC**. A normal TCP/uTP torrent seed cannot serve
a web peer unless it also supports WebRTC; a WSS tracker/signalling service
introduces WebRTC peers ([WebTorrent FAQ][wt], checked 2026-10-06).
Ship a separately bounded WebRTC-capable seed bridge for willing node operators
or a supported hybrid seeder. Merely enabling native BitTorrent in Rust is
insufficient. Start with multiple independent WSS trackers; later authenticated
iroh gossip can carry signalling if implemented. ICE may need independent
STUN/TURN services; successful peer discovery does not imply successful NAT
traversal. WebTorrent and iroh-wasm are distinct transports.

Publish HTTPS range/CORS webseeds as optional fallbacks
([BEP-19][b19], checked 2026-10-06). Native clients can fetch solely from torrent
or iroh peers. Browser fallbacks are WebRTC seeders, iroh peers through relays,
then available webseeds. Do not execute any downloaded script until the
authorised SHA-256 checks pass. Browser cache target is 128 MiB with quota-aware
eviction; tab memory is bounded separately. No forced browser uploading.

Era torrents reuse [archive export](../ops/archive-node.md): 8,192-block eras,
signed location manifests, and history-root/MMR validation under a certified
anchor. The export key's signature authenticates a location manifest; only the
chain proof authenticates era history. Seeding and public publication are
explicit additions to the current export-only path.

## 5. Primary entrance: the wallet's native in-app browser

### 5.1 End-to-end `eastsea://name`

    user/QR → native scheme resolver
      → proof-capable EastSea peer (DHT / known IDs / cache / peer exchange)
      → certified Names + Registry reads at a coherent state height
      → delegated K → pkarr locator (where, not what is authorised)
      → manifest / bundle index / files from torrent or iroh peers
      → native SHA-256 verification + release/permission policy
      → eastsea-app://<base32(appId)>/<entry> in WKWebView
      → native provider for reads/signatures

1. Validate 26's lowercase ASCII name grammar, length and release/grace rules.
   The name is not a DNS hostname. Reject transaction parameters in name links.
2. Prove `ownerOf`, expiry, `text["app"]` and `text["pkarr"]` against the
   pinned Names runtime code hash. Pin the chosen Registry address/code hash.
   Decode Solidity mapping and short/long-string slots with bounded reads.
3. Prove Registry current and pending fields. Evaluate effective release state
   using the **authenticated state-block timestamp**: for a root in certificate
   H, evaluate the H−1 state at H−1's proven timestamp. Handle delayed unlist,
   and require explicit transfer acceptance; delay expiry alone does not change
   the publisher. Plain remote `eth_call`, indexer events or account
   balance proofs do not establish these facts. Alternatively, a user's fully
   verifying local node executes these reads.
   For 31's narrowing-release override, obtain the predecessor manifest and
   authenticated queue time/declaration as well. After `settle` clears pending
   fields, current storage alone may no longer contain that evidence. A cold
   light resolver needs authenticated historical snapshots or certified
   block/receipt history replay with a complete relevant range; an isolated
   release-event inclusion proof does not establish the absence of later
   cancellation/requeue. Use the verified current seq/hashes to cross-check the
   reconstruction. A fully verifying local node with retained history can do
   this first. If evidence is unavailable, do not enable the new release or
   signing bridge; keep an explicitly historical prior view if allowed.
4. Resolve owner-authorised `K`; compare locator values to that state. If the
   locator is absent, stale or malicious, request the chain-selected hashes
   directly from peers/cache. DHT disappearance never unlists an app.
5. Verify manifest and reciprocal `name_binding`, index and files. First opens
   retain 31's native app information/permission step. Handle name rebinding,
   expiry and update activation before reconnecting a signing session.
6. Serve only verified bytes via `WKURLSchemeHandler`; no HTTP page navigations
   or `fetch` to an HTTP entry point are needed. Native iroh relay transport may
   use TLS infrastructure; that does not introduce an HTTP application origin.

Reuse the [verified ReleaseLog read](../../crates/ffi/src/lib.rs) pattern,
extended with an explicit state-height read/batch capability so tip movement
does not cause indefinite mismatched-slot retries. Storage/code-hash proof
primitives already exist; exposing coherent bounded proofs is RPC/client work,
**not a new state root or consensus rule**. Never upgrade an unproved result
because several peers agree.

### 5.2 Scheme and provider security

Implement the isolation requirements of 31 §5.4 before loading third-party
bundles: `appKey = lowercase base32(appId)`, 52 characters; separate app data
stores; permissions keyed by chain/registry/appId; verified cache only; native
identity/status strip outside page control. Add a chain/registry namespace to
the data-store UUID as well as appId, so identical IDs on different networks
cannot inherit cookies or permissions. Pin one release for the running view.

The isolated `WKContentWorld` holds privileged bridge logic; a minimal page
facade passes requests. Native dispatch independently checks main frame,
expected scheme/appKey, view/release identity, declared method permissions,
account lock/connection and pending-request limits. Isolation alone does not
authenticate a page request. Cancel requests on navigation, account/network
change, release invalidation or lock.

CSP denies remote scripts, eval, frames and loopback RPC; allow only verified
same-app resources and declared network destinations. Chain access uses the
native provider allowlist. Test WebKit custom-scheme origin, module, CSP,
storage, cancellation and MIME behaviour on supported OS versions; if origin
separation fails, do not ship the signing bridge with that scheme.

Current OS `eastsea://pay|call|connect|tx` links already have action meanings
([WalletModel](../../apps/wallet/Sources/WalletModel.swift)). Preserve those exact
legacy routes. Introduce unambiguous `eastsea://name/<name>` for every colliding
name, including `name`, `app` and `follow` as well as the legacy action words;
other bare valid names can resolve normally. Also retain `eastsea://app/<appKey>`
and `eastsea://follow/<appKey>` from 31. Do not reserve those names in the
on-chain name service. External schemes remain interceptable by another app;
links carry identity only, never secrets or automatic signing instructions.

## 6. Browser without installation: onboarding and replaceable mirrors

### 6.1 Authenticated loader → any mirror

Package a small loader/verifier with the chain tuple, BLS identity, genesis
digest, contract pins and an initial intro/explorer manifest SHA-256. The
extension or native wallet authenticates these at installation/update.
A previously authenticated browser loader can also fetch data from any origin.

After that trust step:

1. Fetch candidate manifests as **data**, race community mirrors and peer paths.
2. Require the pinned manifest hash, or prove its active registry release under
   the BLS light client. Verify bundle index and every asset before execution.
3. Run the same packaged `crates/light` logic through WASM. Reuse a verified
   anchor for account/storage/receipt reads; persist chain-specific height floors.
4. Build `crates/net-wasm` (proposed) around pinned iroh with browser-compatible
   features. Native transport code and the current mainline UDP adapter do not
   compile unchanged into a browser.
5. Resolve known EndpointIds and `K` through replaceable pkarr HTTP relays;
   verify returned signed bytes locally. Connect `aether/rpc/1` through
   configurable, independently run iroh WebSocket relays.

Iroh's documented browser path is relay-only and end-to-end encrypted; the relay
cannot decrypt peer traffic ([iroh browser docs][iroh-browser], checked
2026-10-06). Browser clients cannot “fall back to direct UDP” when relays fail.
For production, community operators must supply capacity and relay failover;
n0's free relay preset is documented for development/hobby use, not a production
availability guarantee ([iroh relay limits][iroh-limits], checked 2026-10-06).

Moving from relay A to B requires the **provider** to attach to B and publish its
new address; a client cannot arbitrarily substitute B into A's URL and reach the
same provider. Maintain providers spread across at least two independent relay
operators, or prototype multi-relay provider attachment explicitly. A signed
record can carry two URLs, but that does not prove both currently work.

A vanilla HTTPS mirror cannot serve a browser's torrent content directly over
Mainline UDP. It can host bytes, webseed, or provide a replaceable DHT/HTTP
bridge. CORS and WSS/HTTPS certificate usability are separate reachability
requirements. Privacy remains limited: lookup relay sees queried keys, iroh
relay sees endpoint/IP/timing, provider sees requested chain data.

### 6.2 Cold browser trust and the blocked-mirror procedure

**Do not claim that mirror-delivered HTML verifies itself.** A malicious mirror
can replace the loader, hash comparison, BLS key, verifier and success badge
together. SRI inside that same document, a same-origin service worker, or a
“trusted hash” fetched through the same loader does not fix first execution.

The default no-install page is therefore **onboarding/read-only**, with no key
import, signing or native-verification assurance. It offers an identity capsule
and a handoff to the installed wallet. It can perform real data verification
when its loader is independently authenticated, but must state the initial
web-origin dependency.

Optional strict no-install mode: distribute a single self-contained
`bootstrap.html` capsule including verifier JS/WASM and network anchors. Obtain
its expected SHA-256 independently (authenticated release, existing verified
wallet/extension, or a trusted offline contact), download it from **any** mirror
without executing it, compare the file hash with an OS/local verifier, then open
the checked local file. This adds no installed EastSea application, but requires
an independent verification action and initial anchor. File-origin CORS,
WASM/WSS/WebRTC behaviour must pass Chrome/Firefox/Safari testing before
advertising this mode; no service-worker/COOP assumption hides in the capsule.

Discovery capsule, copyable as text/file/QR:

    C32 + group_u16 + desired appId32 + expected manifestHash32
    up to 4 EndpointIds32; up to 8 HTTPS/WSS location hints

Locations are replaceable; the identity/hash values need an authenticated
source. Split large capsules into bounded QR/file attachments rather than
assuming every scanner accepts an 8 KiB QR.

Concrete recovery when a mirror is blocked:

- A loaded authenticated loader starts cache and peer paths immediately, tries
  two mirror origins in parallel, and accepts pasted/exported contact capsules.
  Fetch timeouts do not send the user back to a single “official URL.”
- A returning user has local content/peer hints; alternate origins cannot read
  another origin's browser storage, so export/import the capsule or use the
  extension/native store. A service worker on the blocked origin is not a
  universal cross-origin bootstrap.
- A first-time user gets a working mirror/capsule from another user, publisher,
  community documentation, printed QR, ordinary search, or any available content
  host. No Pipln redirector is required. The native wallet can retrieve a current
  capsule through DHT/peers even when every listed HTTP mirror is blocked.
- If the user has no working URL, no cached loader, no peer/contact route and
  no independent anchor, an ordinary browser cannot discover or authenticate
  the system from nothing. Offer another contact channel or the native wallet;
  do not fabricate a verification result.

Thus **mirror replacement restores availability; independent loader
authentication establishes integrity**. These are separate acceptance tests.

## 7. Anti-eclipse, Sybil resistance and cache safety

[BEP-42][b42] binds DHT routing IDs to an IP-derived prefix, making targeted
placement harder; it is not an identity system or complete Sybil defence
(specification dated 2014, last modified 2016; checked 2026-10-06).
Retain the vendored secure-ID generation and secure-node preference. Add tests
that compliant storage/query candidates count toward lookup termination;
non-compliant closest replies must not prematurely end a secure lookup.
Private-LAN exceptions remain outside public discovery.

The chain's committee key and certified state are the integrity boundary.
Registered-candidate/operator proofs can weight provider diversity, while open
archives/mirrors need not become registered candidates. No DHT role, node count,
WebRTC tracker response or publisher locator grants voting authority.

Proposed client bounds and policy:

- Cache key: `(C, group, registry address+code hash, appId, active seq/hash)`;
  name delegation cache additionally includes Names address+code hash and
  name node. Network changes cannot reuse authority/permission caches.
- Race lookup paths with different bootstrap operators, last-known DHT nodes,
  known endpoint IDs and user-imported contacts. Repeating a lookup on the same
  closest-node region is not independent consensus evidence.
- `aether_publicPeers`: up to 32 signed contacts/answer, stopping before the
  **actual serialized reply** exceeds 32 KiB (JSON/base64 means fewer maximum-size
  envelopes fit);
  at most one request/30 s/connection, pool ≤128, no recursive unbounded
  fetching. Gossip uses the same caps and expiration validation; dedupe by key
  and cap unverified contacts per address prefix/relay.
- Prefer distinct proven operators where possible. Do not sell prefix diversity
  as ownership diversity. Transport rate limits keyed only by self-created
  endpoint IDs also need global work/byte budgets.
- Before signing, require a fresh certified head and proven nonce/balance;
  engineering target ≤30 s head age, hard stale ceiling no looser than the
  current verifier's 10 minutes. If current state cannot be proved, cached apps
  can display an explicitly historical offline snapshot with signing disabled.
  Eligibility/classification gates still apply; cached name ownership and
  delegation are labelled unrefreshed.
- Keep verified heights/digests and seq high-water marks across restarts.
  Expired service records may be reconnection hints; cached proof state cannot
  quietly become “latest.” High seq alone cannot select an app version.
- Preserve old verified app bytes for rollback/display, subject to 31's release
  and eligibility policy. A cancelled update, new owner or expired name is
  re-evaluated from chain state, not cached locator text.

Residual attacks: censoring every lookup/path, concentrating seeders/relays,
fresh-but-withheld updates within the freshness window, key compromise,
traffic correlation and committee compromise. Proof checking prevents data
forgery under its assumptions; it does not force delivery or prove the freshest
possible head. Clock error and offline rollback require explicit UI states.

## 8. Latency and bandwidth targets

These are engineering **targets**, not measured performance claims. Measure
median/p95 on macOS native, iPhone WKWebView/native path, Chrome/Firefox/Safari,
good broadband and 100 ms RTT mobile links. Count DNS/TLS/relay setup and proof
verification; publish cold/warm results separately.

| Operation | Warm target median / p95 | Cold target median / p95 |
|---|---|---|
| First usable provider + certified head | 250 ms / 1 s | 2 s / 8 s |
| Native name + effective-release proof | 300 ms / 1 s | 3 s / 10 s, including discovery |
| Cached app first paint | 100 ms / 300 ms | — |
| 1 MiB app, first paint after verified entry assets | 500 ms / 2 s with known peers | 3 s / 10 s at ≥10 Mbit/s and a responsive seeder |
| Browser relay-based verified read, established session | 500 ms / 1.5 s | 4 s / 12 s with loader already authenticated |

A 25 MiB bundle over 10 Mbit/s alone takes roughly 21 seconds; never promise a
3-second complete download. Show verified progress and serve entry assets lazily
through the index; a single tar torrent may require extra pieces before entry
assets become available. Small apps and warm peers are the first-paint targets.

Use content-addressed persistent caches, short authority cache revalidation,
pooled iroh sessions, parallel two-provider queries and 300 ms hedging. Start
independent DHT/known-key/cached paths together rather than serial 8-second
HTTP timeouts. Refresh a live descriptor in the background; do not block a
working peer on a DHT refresh.

Prefetch only the user's added/followed apps and publisher manifests, at most
two downloads / 8 MiB ahead by default, respecting network/power policy.
No popularity-driven catalogue crawl. Verify one certificate per coherent
anchor and reuse it for multiple state proofs. Native BLS and browser WASM
costs differ; [the WASM research](../research/wasm-speed-2026-10-05.md) supports
budgeting tens of milliseconds, not speculative wide-arithmetic/WebGPU gains.
Move browser verification off the UI thread where supported; MV3 lifecycle and
WebKit foreground/background suspension are acceptance cases.

## 9. Irreducible dependencies and founder control

| Remaining dependency | Who can run/provide it | Failure / control risk |
|---|---|---|
| Initial verifier, chain/genesis/group identity | Authenticated software releases, independently chosen distribution/contact channels | Compromised software or wrong first anchor; DHT cannot repair this trust choice |
| Mainline bootstrap/routing reachability | Existing BitTorrent ecosystem, cached routing tables, user contacts | UDP blocking, bootstrap/DNS outage, hostile closest nodes |
| Live chain data and consensus quorum | Independent EastSea validators/followers/archives | Availability and quorum concentration; no transport can create missing finality |
| Content retention | Publishers and consenting consuming nodes/archives | An unseeded/unmirrored uncached bundle is unavailable even if its hash survives |
| Browser iroh relay / pkarr HTTP bridge | Independent node/community/hosting operators | Egress filtering, traffic metadata, rate limits; at least one reachable route is essential |
| Browser WebRTC signalling/STUN/TURN | Independent consenting operators | NAT/mobile/browser restrictions; sometimes a bandwidth-carrying relay is unavoidable |
| First browser page and usable web TLS | Any publisher/community mirror | First-load code trust and domain/CA dependency unless independently authenticated |
| Native app distribution and OS browser policy | Platform and independent distribution channels where permitted | Distribution removal, suspension, custom-scheme interception, WebKit policy |

The current vendored [bootstrap configuration](../../vendor/n0-mainline/src/actor/config.rs)
names third-party routers. “No founder server” does not mean “no bootstrap,
no DNS anywhere, or free bandwidth forever.” Support saved routing contacts,
user configuration, and address literals where usable.

**Operational decision:** Pipln runs **none of the required production
relays, lookup bridges, trackers, gateways or content mirrors**. Independent
operators may provide them without a Pipln permit, fee, operator signing key,
allowlist or privileged API. Their resource budgets and content policies are
their own. At least two independent browser relay/lookup routes and non-Pipln
content seeds must exist before claiming founder-independent browser service.
Free n0 capacity is not a substitute for that acceptance evidence.

Cloudflare Pages may remain an **optional, replaceable static onboarding or
explorer mirror**. If Pipln elects to run it, its outage, domain revocation or
content deletion cannot change registry state or prevent the native primary
path. No Pipln-hosted automatic redirector, latest-hash API, index service,
RPC proxy or relay is hidden behind the mirror. Third-party dApp hosting and
standard-wallet RPC operation belong to independent publishers/operators.

Pipln still controls its authored wallet releases and any mirror it chooses to
operate; app publishers control their releases, and the committee retains its
existing quorum-based upgrade powers. This design removes founder infrastructure
as a compulsory public entry, not every form of software or governance power.

Legal check: preserve [31's 2026-10-06 decisions and gates](31-app-registry.md),
including no Pipln recommendation/ranking/operator payments and no reintroduced
financial execution entrance in the beta. A publisher's torrent signature is
not a licence; seeders/mirror operators still choose retention and respond to
their own obligations. Public names/topics reveal interests and IPs, so no
private query/contact data is published there.

Moving bytes from HTTPS to torrents does not establish a legal exemption.
The current [Apple guidelines §4.7][apple] continue to assign responsibilities
to a mini-app host and require additional controls (checked 2026-10-06).
This proposal does not silently move the Mac reader into the iOS shipping
binary. Required legal review concerns actual shipped UI, seeding, mirror
operation and RPC submission roles; no “no founder control” compliance badge
or assertion of immunity is warranted.

## 10. Feasibility: standard-wallet compatibility entrance

### 10.1 Recommendation and distinct scope

**Feasible as a later protocol upgrade; reject an RPC-only conversion.**
First ship native wallet/public discovery. Then prototype a separate type-2
lane on a private devnet, review fees/metering and historical replay, and test
actual MetaMask/Trust/Rabby versions before advertising compatibility.
No wallet-specific support or market-share claim is assumed here.

These wallets' ordinary in-app browsers do not gain `eastsea://`, pkarr,
native torrent fetching, or the private `eastsea-app://` scheme from an RPC
change. A publisher/community entrance resolves an app and serves HTTPS.
Users choose an independent standard RPC endpoint/custom network. A wallet
integration could later implement native content resolution; a Snap/SDK is a
different compatibility option, not standard type-2 support.

Compatibility therefore increases reach while retaining HTTP/TLS and RPC
operator dependencies for those users. It can be **Pipln-independent**, but
cannot deliver the native path's no-HTTP entry using unmodified standard wallets.

### 10.2 Transaction bytes, hashes and accounts

Current [TxHeader](../../crates/types/src/envelope.rs) signs native
`aether/tx-header/v2|v3` fields, including separate exec/prove/state budgets,
fee caps, payload commitment, scheme and group. Secp256k1 signs Keccak of those
native bytes ([crypto](../../crates/crypto/src/lib.rs)); its Ethereum-compatible
address derivation already exists. [tx.rs](../../crates/execution/src/tx.rs)
encodes a custom EvmCall and hashes the canonical envelope with BLAKE3.
Having that signer does not validate an Ethereum transaction signature.

Proposed new consensus variant retains exact signed bytes:

    0x02 || RLP([chainId, nonce, maxPriorityFeePerGas, maxFeePerGas,
                 gasLimit, to, value, data, accessList, yParity, r, s])
    signingHash = Keccak256(0x02 || RLP(the first nine fields))
    txHash      = Keccak256(all signed bytes)

That is [EIP-1559][e1559]'s type-2 format under [EIP-2718][e2718]
(checked 2026-10-06). Consensus requires canonical RLP, minimal unsigned
integers, exact field counts/widths, valid `to`, bounded data/access list,
`yParity ∈ {0,1}`, valid secp signature and low-s
([EIP-2][e2], checked 2026-10-06). Recover sender, execute the supplied access
list, and keep native envelopes' existing bytes/signatures/hashes unchanged.
No RPC-added unsigned budget/group/delegation may change the signed meaning.

The compatibility EOA is the ordinary 20-byte secp address. No passkey
registration, implicit `EastSeaAccount` deployment or EIP-7702 delegation.
Existing absent-account handling can supply zero nonce/balance; account
materialisation must still pay state costs. Test the current native u128 stored
balance limit against standard U256 RPC/signing values; reject overflow, never
truncate. Reject unsupported types 0/1/3/4 explicitly until separately specified.

Use one account nonce across native and type-2 transactions. Check chainId at
consensus admission and replay; configure a unique production chain ID, not
dev/test IDs. Type-2 has no EastSea group field: initially support **group 0
only**, or later allocate distinct authenticated chain IDs per group.
Never inject a destination group after signing. Domain-bind typed-message
signatures to the selected chain and application; personal signatures do not
become transaction authorisation.

Every standard RPC transaction/receipt/log lookup for a **type-2 transaction**
reports the raw Keccak transaction hash consistently. Native transactions retain
their existing BLAKE3 identities in receipts/logs and documented native rendering;
do not manufacture Ethereum raw bytes or rehash historical native records.
A node may retain an internal content digest for type-2 bytes, but must not
return that digest as if it were the signed Ethereum hash.
Version receipt commitments and proof logic rather than rewriting historical
native receipt roots.

### 10.3 Paid-state budget and fee-vector mapping

Current [execution](../../crates/execution/src/block.rs) reserves execution,
proving and paid-state budgets separately, and charges new accounts, slots,
code, archived transaction bytes and receipts/logs. Type-2 signs one gas limit
and one maximum price. Mapping its gas only to `gas.exec` while inventing
state/proving budgets would permit debits beyond the wallet's signed fee
ceiling. A gateway subsidy would add an operator dependency and is rejected.

A **candidate** future compatibility model is scalar resource gas:

    e = actual EVM execution units (including standard intrinsic/access-list gas)
    p = actual proving units
    s = actual paid-state units, including bounded tx/receipt archive costs
    wP, wS = positive integer conversion weights fixed by the protocol
    Q = e + wP*p + wS*s
    B = max(baseExec, ceil(baseProve/wP), ceil(baseState/wS))
    L = signed gasLimit; Cmax = signed maxFeePerGas
    Pmax = signed maxPriorityFeePerGas
    F = min(Cmax, B + Pmax)
    require Cmax >= B and Pmax <= Cmax
    require Q <= L
    upfront fee reserve = L*Cmax
    final fee = Q*F
    refund = L*Cmax - Q*F

The maximum debit is **value + L×Cmax**; no independent state/proving debit.
Because `Q×B ≥ e×baseExec + p×baseProve + s×baseState`, the scalar reserve
can cover each resource. Allocate actual proving costs to the existing proof
escrow, burn exec/state costs plus conservative base surplus, and specify
the priority-fee split explicitly (native split currently 60/20/20).
Neither the weights nor surplus accounting can be chosen by an RPC operator.

This overcharges some mixes compared with exact vector pricing. It also has
**EastSea-specific base dynamics**: the maximum of vector bases does not
automatically follow Ethereum's one-dimensional EIP-1559 update formula.
Market it as type-2 transaction/wallet compatibility, not identical Ethereum
economics. Full EIP-1559 fee-market behaviour would require an additional
scalar market design and analysis of the paid-state/proving floor.

Weights, scalar block capacity, failure reserve and opcode behaviour
(`GAS`, `GASPRICE`, `BASEFEE`, refunds) require a separate normative
protocol specification and calibration; no safe numeric weights are measured
in this design. Account/receipt fixed costs must be reserved before execution.
Meter proof work and state allocation before exhausting the shared signed
budget. Exhaustion canonically rolls back user effects, consumes nonce and
bounded fees, and produces a bounded failed receipt within `L`.
Do not reuse today's “state budget exceeded → invalid transaction” behaviour
without resolving inclusion/failure semantics. Failed/reverted transactions
must not create unpaid archive growth.

If preserving exact native vector economics is mandatory, continue native
transactions and require a custom wallet integration. There is no honest
unmodified standard-wallet gas answer that exposes three separately signed
resource budgets.

### 10.4 RPC answers and network configuration

Current node implements some real `eth_*` reads and `eth_call`; it lacks
`eth_sendRawTransaction`, `eth_estimateGas`, `eth_feeHistory`, standard
transaction/receipt/block lookups and several wallet probes. Some current reads
ignore block selectors/pending semantics. The public read-only allowlist even
excludes `eth_chainId`, balance, nonce and code. Therefore the existing gateway
cannot be advertised as a standard-wallet RPC.

Required proposed RPC behaviour:

| Method / surface | Answer and boundary |
|---|---|
| `eth_chainId`, `net_version`, client version | Authenticated selected network's ID (hex quantity for chainId); usable probes. A matching answer still does not prove honest state. |
| `eth_getBalance/Code/StorageAt`, block selectors | Correct selected-height semantics, canonical quantities/bytes; explicit errors for unsupported historical state. |
| `eth_getTransactionCount(...,"pending")` | Account nonce plus executable local contiguous pending sequence; replacing/dropping txs is handled consistently. |
| `eth_call` | Selected-state execution, requested bounds, revert data, access-list semantics; no zero-fee private shortcut reused as a paid-state estimate. |
| **`eth_estimateGas`** | Run the same compatibility executor and search for sufficient **scalar L**, including p/s, account creation and archived tx/receipt/log bytes. Quote at a stated state/base context; estimate is advisory, consensus enforces the cap. |
| **`eth_feeHistory`** | `oldestBlock`; N+1 actual scalar `baseFeePerGas` values including next-block B; N scalar `gasUsedRatio` values; requested gas-weighted priority reward percentiles. No exec-only numbers hiding state fees. |
| `eth_gasPrice`, `eth_maxPriorityFeePerGas` | Consistent with scalar B and recent priority policy; no guarantee of inclusion after fee/state movement. |
| `eth_sendRawTransaction` | Validate raw signed type-2, selected chain, bounds/activation/mempool policy; return Keccak(raw). Public submit-only service, never arbitrary owner/node RPC. |
| Standard block/tx/receipt/log methods | Stable hashes and indices, status, type, scalar gasUsed/cumulativeGasUsed, effectiveGasPrice, contract address and logs/bloom; additional native resource metrics may be explicit extension fields. |

The [Ethereum execution fee API][fee-api] defines the fee-history response
shape (checked 2026-10-06); scalar ratios must correspond to a protocol-defined
scalar block limit. Count the same projected resources from native transactions
if they share that capacity. Update the three native congestion markets using
actual resources, not fabricated RPC gas. State price/weights and pending-state
changes can invalidate estimates, but can never lift the signed cap.
The scalar `gasUsed × effectiveGasPrice` billing invariant applies to type-2
transactions. Native vector debits require explicitly documented native receipt
rendering and resource/fee extensions; do not disguise them as identical type-2
billing or change their historical receipt commitments.

An illustrative simulation with `e=30,000, p=2,000, s=100` and **fixture-only**
`wP=1, wS=100` gives Q=42,000. An estimator might return 50,400 after a
20% margin; it must not return 30,000 and debit state separately. Wallets may
change estimates/fee caps manually; a low limit fails canonically. These
numbers choose no production fee schedule.

The dApp obtains the user-selected independent RPC URL and requests a custom
network with `chainId`, name and `nativeCurrency={name:"Doubloon",
symbol:"DBLN",decimals:18}`. Validate RPC chain ID and require consent.
`wallet_addEthereumChain` and `wallet_switchEthereumChain` are wallet methods,
not node methods ([EIP-3085][e3085], [EIP-3326][e3326], checked 2026-10-06).
HTTPS endpoints and a normal HTTPS app origin are the baseline. Multiple
`rpcUrls` are suggestions, not a guarantee of automatic wallet failover.
Do not hardcode a Pipln RPC or promise every wallet accepts arbitrary chains.

### 10.5 Protocol activation, capabilities lost, security

This is **not inherently genesis-only**. Current
[upgrade.rs](../../crates/node/src/upgrade.rs) implements protocol 3 and
committee-signed height activation; old nodes stop at an unsupported upgrade.
Schedule a later version only after clients, validators and provers implement:

- A versioned transaction discriminator and block codec, preserving historical
  `Vec<TxEnvelope>` decoding and all old hashes.
- Activation-aware admission/mempool and dual historical replay; bounded raw
  type-2 validation at both RPC and consensus.
- Versioned storage/snapshots, receipt roots/history proofs and updated
  [prover input/guest commitments](../../crates/node/src/prover_input.rs).
  A serde/postcard change is consensus-relevant here.
- Scalar metering/caps/refunds, block capacity and fee history, account/nonce
  cross-lane behaviour, matching native/guest execution.

Genesis-bound `history_v2`, rewards and group settings are not silently flipped
to turn this feature on. Existing paid-state activation is not already a type-2
upgrade gate. On a legacy chain, any needed paid-state rule activation needs its
own specification; no chain reset follows merely from adding a transaction
format. Keep the existing committee approval and notice rules; this document
does not authorise an upgrade.

Standard EOAs lose the native account's passkey/Secure Enclave custody, owner
policies, delayed recovery, scoped sessions, recipient/day limits, expiry,
native self-delegation and account batch features unless they separately adopt
a supported smart-account flow. They also do not automatically verify EastSea
proofs. Their wallet's RPC can misreport balances, fees, simulation or inclusion;
the chain still enforces the signed transaction and fee ceiling. A malicious
HTTPS dApp retains ordinary approval/phishing risks. Never ask users to import
passkey material or enable EIP-7702 as a compatibility shortcut.

Compatibility gate: raw vectors from each intended wallet, chain/group replay
rejection, low-s/canonical RLP, access lists, create/call/revert/OOG, initial
account costs, state/archive exhaustion, cap/refund conservation, cross-lane
nonce replacement, hash/receipt/log consistency, height-boundary old/new replay,
and native/prover equivalence. Test endpoint replacement and custom network
addition on actual supported wallet versions; do not equate an EIP-1193
provider with verified working compatibility.

## 11. Small implementation steps and shipping sequence

Each step is independently reviewable. Names/Registry deployment and app
eligibility remain subject to 31; none is silently completed by this document.

| Step | Crate/app ownership | Deliverable / test gate | Timing / consensus |
|---|---|---|---|
| 1 | `crates/net`, network-config consumers in `crates/ffi`, extension | Chain fingerprint; strict public-address filter; persist peer/seq floors; configurable independent bootstrap/relay routes, retain existing pkarr addresses | **Right after beta; none** |
| 2 | `crates/net`, `crates/node/src/p2p.rs` / `announce.rs` / `rpc.rs` | Contact encoder/verifier, known-key service lookup, bounded peer exchange and service methods; preserve registered wallet-server filtering/read gate | **Right after beta; none** |
| 3 | `crates/ffi`, `crates/light`, node RPC | Coherent certified Names/Registry storage reads and state-block time, pinned code hashes, delegated locator, effective-release resolver; retained-history path for settled narrowing metadata and cold light-client witness work | **Right after beta on dev/testnet; none** |
| 4 | Wallet `Browser*`, `WalletModel.swift`, proposed `AppBundleScheme` | Native name link dispatch, verified cache/provider isolation, `eastsea-app://`; start with existing peer cache/direct hashes and bundled explorer | Mac allowed scope after beta; iOS scope gate; **none** |
| 5 | Publisher CLI from 31, `crates/net` app-content protocol, node archive/export | Deterministic bundle torrents/manifest extension, native downloader/seeder and bounded cache; era seeding; BEP-5 rendezvous TCP introduction | Subsequent small slices; **none** |
| 6 | Proposed `crates/net-wasm`, `crates/wasm`, extension RPC/offscreen lifecycle, explorer RPC/verify | iroh-wasm transport, multiple pkarr/iroh relays, proven reads, remove compulsory `rpc.eastsea.xyz`; retain loopback/user-selected endpoints | After native route; **none** |
| 7 | Independent operator tooling/docs, browser content loader | WebRTC-capable seed bridge, replaceable WSS signalling/STUN/TURN, authenticated loader capsule, community mirror export/import | Later; **none** |
| 8 | Node/wallet/extension/browser test harnesses | Full founder-outage acceptance below; document independent capacity and cold-start evidence | Gate for “no founder endpoint required”; **none** |
| 9 | `crates/types/execution/crypto/light/wasm`, node codecs/RPC/upgrades, prover/guest | Separate reviewed type-2/scalar fee specification and devnet prototype; no changes mixed into discovery PRs | **Later; consensus/protocol upgrade** |

Reuse pinned libraries/current patterns first. Choosing a torrent/WebRTC engine
and adding its dependencies requires a separate implementation review; no
package is introduced by this design. Release native resolver/fetch/isolation
in slices; do not hold DHT failover hostage to standard-wallet fee-market work.

## 12. Tests and completion evidence

These are proposed tests, not tests run by writing this document.

### 12.1 Format, proof and policy tests

- Independent encoder/decoder vectors for fingerprint/topic, 53-byte salt,
  ContactV1 max 768 bytes, encoded v 772, complete packet budget, LocatorV1
  max 637 / DNS 943 / encoded v 947 / outer pkarr 1,047. Reject truncation,
  bad key/target/salt/signature, oversized strings/counts, invalid DNS chunks,
  stale/future timestamps and seq rollback after restart.
- Standard Mainline without signed-peer support still discovers a TCP
  introducer; BEP-5 contacts become authenticated iroh IDs only after
  `es_contact_v1`. Test CGNAT/IPv6-only provider introduced by an independent
  reachable peer. Private/reserved-address hints do not trigger network probes.
- BEP-42 generation/check vectors and hostile non-compliant closest replies;
  simulated UDP blackout, one poisoned bootstrap, empty/poisoned HTTP relay,
  mixed operators and bounded malicious peer exchange. Use a local DHT fixture,
  not unsolicited public-DHT load tests.
- Names owner/delegation rotation/transfer/grace/re-registration; reciprocal
  app binding; wrong contract code, mixed state heights, cancelled release,
  narrowing declaration violation and premature activation. Remote `eth_call`
  alone cannot satisfy the resolver. Test H−1/H timestamp boundary crossings,
  delayed transfer without acceptance, and a settled narrowing-policy violation
  with empty caches and cancellation/requeue history. URI dispatch covers every
  routing-word collision without reserving an on-chain name.
- Wrong torrent/infohash/metainfo, valid transport but wrong authorised index,
  file corruption, malicious tar paths/symlinks/duplicate names, cache/disk
  limits, partial-download recovery, last seeder gone, era MMR/root mismatch.
- WKWebView and extension main-frame/origin/isolation/permission tests,
  network/account switches and navigation cancellation; no loopback privilege
  inherited by a third-party explorer bundle; no arbitrary code before hashing.
- Provider proof freshness/digest floors, stale cached UI with signing disabled,
  failed gateway policy remaining a policy error; valid legacy receipts marked
  according to actual commitment availability, not a blanket old UI assumption.
- Browser first-load negative test: malicious mirror replaces loader+key+badge.
  The product must not classify that unauthenticated page as independently
  verified. Checked capsule rejects a byte-modified loader before execution.

### 12.2 Required founder-outage integration test

Build a staging chain with sufficient independently operated validators to
retain quorum after the Pipln operator disappears (for example, four validators,
threshold three, at most one Pipln-operated validator). Community introducer,
archive, content seeder, pkarr HTTP relay and browser relay providers use
independent operator identities/infrastructure. The authorised app is published
and seeded by a non-Pipln publisher. A test-only synthetic deployment is enough
for automated coverage; production claims also require an actual operator drill.

**Deny every Pipln-run endpoint**, including domains and known IPs, Pages,
Tunnel/RPC, founder-owned validator/follower listeners, mirrors, release
sidecars, redirects, relay/lookup/tracker and health endpoints. Prevent hidden
fallback requests using an egress allowlist and capture attempted traffic.
Bring down those services, not merely change the visible URL.

Run both empty operational caches and last-known-good caches. Retain only the
authenticated software/network trust anchors needed by any light client:

1. Native wallet opens `eastsea://name`, discovers non-Pipln providers, proves
   current name/release state, fetches via native torrent/iroh with **all HTTP
   content mirrors additionally disabled**, and serves `eastsea-app://`.
   Read a balance; submit a permitted native payment to a non-read-only peer;
   verify fresh finality and supported inclusion/receipt proof.
2. Extension, with no Pipln RPC/default URL and no local node, uses independent
   pkarr HTTP + iroh relay paths, proves balance/nonce, and submits a native
   signed transaction through a permitted provider.
3. Browser loads from community mirror B while A and all Pipln mirrors are
   blocked. With an independently authenticated loader/capsule it verifies
   explorer/intro content and certified reads through independent relays,
   then hands off to the wallet. In ordinary first-load mode it remains
   onboarding with the explicit web-origin trust boundary.
4. Stop the chosen community lookup relay, iroh provider/relay pair, WebRTC
   tracker and seeder one at a time; recover through another independently
   attached provider/path. Exercise content from WebRTC and from iroh separately.
5. Measure §8 medians/p95 and list every reached/attempted host/operator.
   No successful or attempted compulsory dependency on a Pipln endpoint is
   acceptable. Warm-only success does not satisfy cold-start independence.

If all validators/seeders of the beta happen to be founder-operated, this test
**cannot pass honestly** until independent quorum and data retention exist.
Separately take away quorum or all reachable relays/seeders: clients must stop
signing or show unavailable/offline data, never invent fresh proofs. “Founder
outage works” cannot mean “the chain stopped, but a cached page still paints.”

Add the standard-wallet transaction/activation suite from §10 only when that
separate upgrade is proposed. This document creates no consensus test fixtures
or implementation code.

## 13. Dated sources

External specifications/documentation were checked **2026-10-06**. Source
facts above are distinguished from EastSea proposals and unmeasured targets.

- [BEP-3: BitTorrent protocol][b3] — created 2008-01-10; last modified 2017-02-04.
- [BEP-5: Mainline DHT][b5] — created 2008-01-31; last modified 2020-01-21.
- [BEP-9: metadata exchange][b9] and [BEP-10: extension protocol][b10] — specification pages checked 2026-10-06.
- [BEP-19: HTTP webseeds][b19] — specification page checked 2026-10-06.
- [BEP-42: DHT security extension][b42] — Draft; created 2014-01-15; last modified 2016-07-21.
- [BEP-44: mutable/immutable DHT items][b44] — Draft; created 2014-12-19; last modified 2017-02-01.
- [Pkarr upstream README][pkarr] — signed DNS/BEP-44 and browser HTTP relays; checked 2026-10-06.
- [Iroh WebAssembly/browser support][iroh-browser], [relay rate limits][iroh-limits] — checked 2026-10-06. Worktree pins iroh **1.2.0**, mainline adapter **0.5.0**; validate APIs against those versions before implementation.
- [WebTorrent FAQ][wt], [API documentation][wt-api] — checked 2026-10-06; WebRTC peer compatibility and browser transport.
- [EIP-1559][e1559] — created 2019-04-13; [EIP-2718][e2718], [EIP-2][e2], [Ethereum JSON-RPC][eth-rpc], [execution fee API][fee-api] — checked 2026-10-06.
- [EIP-3085][e3085] — created 2020-11-01; [EIP-3326][e3326] — both wallet-interface proposals checked 2026-10-06, not universal wallet support guarantees.
- [Apple App Review Guidelines][apple] — checked 2026-10-06; platform release boundary, not a legal opinion.

[b3]: https://www.bittorrent.org/beps/bep_0003.html
[b5]: https://www.bittorrent.org/beps/bep_0005.html
[b9]: https://www.bittorrent.org/beps/bep_0009.html
[b10]: https://www.bittorrent.org/beps/bep_0010.html
[b19]: https://www.bittorrent.org/beps/bep_0019.html
[b42]: https://www.bittorrent.org/beps/bep_0042.html
[b44]: https://www.bittorrent.org/beps/bep_0044.html
[pkarr]: https://github.com/pubky/pkarr
[iroh-browser]: https://docs.iroh.computer/languages/wasm-browser
[iroh-limits]: https://docs.iroh.computer/relays/rate-limiting
[wt]: https://webtorrent.io/faq
[wt-api]: https://webtorrent.io/docs
[e1559]: https://eips.ethereum.org/EIPS/eip-1559
[e2718]: https://eips.ethereum.org/EIPS/eip-2718
[e2]: https://eips.ethereum.org/EIPS/eip-2
[e3085]: https://eips.ethereum.org/EIPS/eip-3085
[e3326]: https://eips.ethereum.org/EIPS/eip-3326
[eth-rpc]: https://ethereum.org/developers/docs/apis/json-rpc/
[fee-api]: https://github.com/ethereum/execution-apis/blob/main/src/eth/fee_market.yaml
[apple]: https://developer.apple.com/app-store/review/guidelines/
