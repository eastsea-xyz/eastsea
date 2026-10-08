# Registered app content (0.7.4)

The Mac wallet resolves `sea://demo.sea` through the configured Names and
AppRegistry contracts, fetches the active bundle from its local node, and
verifies the canonical index and every file before opening a page. It shows
the name as the origin; the WebKit document uses an isolated
`eastsea-app://<base32-appId>/` origin internally. Registration and matching
content hashes do not establish a publisher's safety or a finality proof for
the local node's registry reads.

## Build and pin content

Choose a static output folder containing `index.html` and separate script
files. The transport is deterministic ustar: relative ASCII paths in byte
order, mode 0644, zero uid/gid/mtime, no links, directories or duplicate names.
The archive includes canonical `bundle.json`. Its SHA-256 is `bundleHash`;
the archive's SHA-256 is not the registry identity. Every indexed file has
its own size and SHA-256. The complete archive, including padding and headers,
is limited to **20,000,000 bytes**; there may be at most 2,000 files, each
at most 10 MiB. The index may be at most 1 MiB.

Use an owned development directory:

```sh
target/debug/aether app-bundle build --folder ./tmp/my-app --out ./tmp/my-app.tar
target/debug/aether app-bundle pin --data ./tmp/my-node --archive ./tmp/my-app.tar --hash <bundleHash>
target/debug/aether app-bundle configure --data ./tmp/my-node --seed true
```

Publish the returned **index hash** in the app's active registry record and
bind that app ID in the name's `app` text record. Registry deployment pins
are per chain in `apps/wallet/Resources/name-sources.json`: each of `names`
and `apps` supplies an `address` and a lowercase `code_sha256` of its runtime
bytes. The resource currently has no deployment addresses; the wallet refuses
unconfigured networks instead of guessing them. The `sea-names` source and
ContentSource protocol were imported unchanged from that lane's draft;
`NodeAppContentSource` implements the protocol using the verified snapshot.

## Node cache and transport

`aether_appBundle` takes `[bundleHash, path]`, where `bundle.json` requests the
index. Its result is `{bundleHash, path, sha256, size, data}`, with base64
bytes. It serves indexed paths only. This method is local HTTP RPC;
public iroh RPC and the read-only gateway refuse it. The separate
`aether/apps/1` ALPN transfers bounded archives from verified cache entries.
Nodes use their existing roster and learned wallet-server peer IDs.

The cache lives in `<node-data>/apps`, has a 500 MiB LRU ceiling and a bounded
entry count, and retains explicit pins within that ceiling. Pins never bypass
the node's free-disk floor. Downloads and cache writes check the actual volume
before allocation and again before publishing a cache entry. Verified cached
reads can continue at the floor. A two-bundle immutable memory cache avoids
rehashing the entire archive for each requested asset. It is bounded to
40,000,000 archive bytes. On a disk read, all hashes are checked again.

Seeding starts **off**. It has a bounded peer/request budget and a transfer
deadline. The following settings take effect after restarting the owned node:

```sh
target/debug/aether app-bundle configure --data ./tmp/my-node --seed false
target/debug/aether app-bundle configure --data ./tmp/my-node --enabled false
target/debug/aether app-bundle unpin --data ./tmp/my-node --hash <bundleHash>
```

## Wallet execution and developer mode

Only verified, immutable bundle assets reach the scheme handler. Top-level
documents must be HTML. A CSP header and an HTML meta policy deny remote
scripts, inline scripts, eval, frames, forms and network connections, including
loopback RPC. Scripts must be separate bundle files. SVG stays an image asset.
Permissions and website storage are scoped to chain, registry and app ID.
The privileged provider handler lives in an isolated WebKit content world;
native dispatch also checks the active view, main frame and exact app host.
Navigation, account, network, lock and developer-mode changes cancel pending
requests. Each account connection and transaction uses the existing native
confirmation flow and displays the resolved `sea://` origin.

In Settings, enable **Developer mode**, then use **Open a local app folder**
in the browser address bar. A snapshot of a static folder opens under its own
fresh namespace. The fixed native banner says **개발 중 · 검증 안 됨** in Korean.
The picker is hidden while developer mode is off. Turning the mode off closes
local content. Local files can sign only on the wallet's owned development
chain (7777), with the normal confirmation flow.

Unavailable, missing, oversized or mismatched content displays an error and
never reaches the renderer. Registry changes during a download also refuse
the load. The running view pins the verified release until the next opening.

## Owned-devnet verification

```sh
scripts/test-app-content-devnet.sh
```

The script observes the team's compiler queue before each compiler command.
It builds a standalone WebKit test executable and tests an owned on-disk
Chain with signed, finalized deployment/registration transactions. The test
uses synthetic one-record Names/Registry runtimes, real loopback iroh transfer,
the wallet's actual name reader and ContentSource, all-file hashing, CSP,
private-scheme page loading and an EIP-1193 chain-ID round trip. It also checks
JavaScript modules/imports and a working counter interaction, alongside
corruption, path traversal, offline cache reads and rejection of unverified
local content. It starts no EastSea application or consensus validators and
sends no transactions to a live network. It does not establish production
registry contract coverage, independent-validator finality, manifest
permissions, bidirectional `name_binding` provenance, or public relay reachability.

`scripts/test-app-content-devnet.sh --build-helper` builds just the native
helper. Set `AETHER_APP_PAGE_HARNESS=$PWD/tmp/app-content/page-check` when
running the full node test gate to include the wallet page portion there.
Pure Swift tests cover index/file tampering, traversal, canonical encoding,
size bounds, developer gating and filesystem symlink escapes. Node unit tests
exercise pin/LRU persistence, the disk floor, opt-out and peer fetching.
