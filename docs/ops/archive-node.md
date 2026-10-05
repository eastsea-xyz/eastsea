# Archive node (roadmap B6)

A validator Mac prunes: after `--retain-days` it keeps era files' roots but
deletes their blocks, and may drop the era files too. Someone has to keep
everything, forever, and hand old history back on demand. That someone is the
archive node — `aether archive` — by convention on the NAS (`poc-nas`), where
the disk is.

It is a follower with three differences:

- **It never prunes.** Whatever the history flags say, the mode forces
  `HistoryMode::Archive`.
- **It serves old history.** `aether_eraInfo` / `aether_eraChunk` /
  `aether_eraProof` answer from its complete store, and `GET /era/<file>`
  serves whole era files over HTTP (its RPC binds `0.0.0.0`, not loopback —
  it is a server).
- **It exports every completed era** (8192 blocks) into a static file set a
  mirror can serve as-is: the era file, a signed manifest, a `.torrent`, and
  an `index.json`.

No voting, no proving, no DeviceCheck, no candidate beacons: nothing
consensus-side. It verifies everything it accepts — certificates, re-executed
blocks, and (before export) each era file against the history root its own
certified index carries.

## Install (from the Mac, to poc-nas)

```sh
scripts/install-archive-node.sh poc-nas <chain-id> <path/to/network.json>
```

The script copies the source tree over with a tar pipe through ssh (with
`vendor/` for the patched n0-mainline; without `.git`/`tmp` — poc-nas's
rsync refuses server-mode writes into `~/AI`, and the tar overwrite keeps
`target/` intact so rebuilds are incremental), installs rustup in the host
user's home when missing (no sudo — userspace only, toolchain 1.98.1 as
`rust-toolchain.toml` pins; plus zig as the C toolchain when the host has no
`cc` — poc-nas ships none, and `zig cc -target x86_64-linux-gnu` carries its
own libc so build scripts and the linker work without root), copies
`network.json` in as
`~/AI/eastsea-archive/network-<chain-id>.json`, writes the runner
`~/AI/eastsea-archive/run-archive.sh`, and starts the release build in the
background. First build on the NAS CPU takes a while; watch it with:

```sh
ssh poc-nas tail -f ~/AI/eastsea-archive/build.log
```

Layout on the NAS:

| Path | What |
|------|------|
| `~/AI/eastsea-archive/src/` | the source, and `target/release/aether` |
| `~/AI/eastsea-archive/<chain-id>/data/` | the node's data dir |
| `~/AI/eastsea-archive/export/<chain-id>/` | the era export set (what mirrors serve) |
| `~/AI/eastsea-archive/logs/archive-<chain-id>.log` | the node log (trimmed at ~10 MB) |

## Start

**Do not start it on the live testnet on your own** — an archive node
attaching to the chain is an operational decision (the founder's), like
seating a validator. Installing and building changes nothing on the chain.
When it is time:

```sh
ssh poc-nas 'nohup ~/AI/eastsea-archive/run-archive.sh <chain-id> > /dev/null 2>&1 &'
```

The runner restarts the node after any crash (30 s apart), exits when the node
exits 0 (a deliberate stop: `pkill -f 'aether archive'` and the runner gives
up too), and logs beside the data. It passes:

```sh
aether archive \
  --network ~/AI/eastsea-archive/network-<chain-id>.json \
  --data   ~/AI/eastsea-archive/<chain-id>/data \
  --rpc-port 8545 \
  --bind 0.0.0.0 \
  --export-dir ~/AI/eastsea-archive/export/<chain-id> \
  --https-base http://<nas-address>:8545
```

`--https-base` is what lands in the manifests as the Https mirror and the
torrents' first webseed. On the Tailnet it is the NAS's Tailscale address
(`http://100.100.59.78:8545`); if the export set is later served by a real
public mirror instead, rebuild the set with that mirror's base — the files are
deterministic, so the era bytes stay identical.

## How the NAS reaches the testnet (no --from-rpc)

The validators' HTTP RPC binds loopback on the founder's Mac — `poc-nas`
cannot use `--from-rpc`. It does not need it: with `--from-rpc` empty the
node binds an iroh endpoint under its own persisted node key and finds the
validators' endpoints through the Mainline DHT (`network.json`'s node ids are
the DHT keys), with n0 relays as fallback when hole punching fails. That
path is pure Rust and Linux-fine — it is the same path a Mac follower on
another machine uses. So: yes, the NAS can follow the testnet, and iroh/DHT
is the only path it has (loopback RPC is unreachable by design).

## The export set

`--export-dir` (rescanned every 30 s) holds, per completed era:

- `era-XXXXXXXX.aera` — the sealed era file, byte-for-byte;
- `era-XXXXXXXX.json` — the manifest: chain id, era number, block range,
  history root, blake3 and size, mirrors — Ed25519-signed under the namespace
  `b"aether-era-manifest-v1"` with the key at `--export-key`
  (`<data>/archive-export.key`, created on first use; the *export dir is what
  mirrors serve*, so the key lives in the data dir next to it, never in the
  set);
- `era-XXXXXXXX.torrent` — BitTorrent v1 (256 KiB pieces, no tracker: DHT),
  with the NAS HTTP base and any `--webseed` URLs as webseeds;
- `index.json` — the whole set at a glance: version, chain id, era length,
  signer public key, and per era the root, digests and both mirror URLs.

A second run rewrites nothing an era whose blake3 already matches: the set is
idempotent and a mirror can rsync it at any cadence. Era files are re-verified
against the node's own certified history index before export — a corrupt or
foreign era file is never published, even though it sits in the data dir.

`--webseed` takes a bare URL (the file name is appended) or a `~name~`
placeholder. The intended second webseed is a GitHub Releases asset:

```sh
aether archive ... --webseed \
  'https://github.com/<org>/<releases-repo>/releases/download/eras/era-XXXXXXXX.aera'
```

When the placeholder is a fixed redirect-style URL
(`.../download/eras/era-XXXXXXXX.aera` carries its own name), pass it with
the name where it falls; the manifest builder substitutes per era.

## Mirrors

- **NAS HTTP** (built in): the archive node's own `GET /era/<file>` serves
  the data dir's era files; the export set is served by putting the export
  dir behind any static file server. Tailnet-only by default — fine for the
  Macs, invisible to the internet.
- **GitHub Releases** (manual, periodic): upload the era files as release
  assets and pass the asset URL as `--webseed` (see above). Byte-addressable
  and CORS-friendly: a pruned Mac can fetch an era from it with plain HTTP.
- **Torrent** (optional): the `.torrent` files and magnets in `index.json`
  are valid as written — no tracker, DHT discovery, the HTTP mirrors as
  webseeds. Seeding is optional extra redundancy, not a dependency: the
  manifests and files are complete and verifiable without any seeder.

## A pruned Mac fetching an era back

A Mac that pruned old blocks (or dropped era files with `--drop-era-files`)
fetches a missing era over HTTP and verifies it against the history root in
its own certified state — the same path B4 built (`era_net::fetch_into`).
Point it at the archive:

```sh
aether follow ... # era fetch sources; the archive's http://<nas>:8545 is one
```

Nothing trusts the bytes: `era::read(bytes, Some(&root))` re-verifies the
whole file against the root the Mac's certified history index carries, so a
wrong or tampered era file is rejected whatever server handed it over. The
end-to-end path (export set → prune → fetch from the archive's HTTP → verify
→ serve old blocks again) is covered by `crates/node/tests/archive_export.rs`.

## Failure and disk

- The runner restarts the node after a crash; a clean exit (0) stops it.
- `--min-free-disk` (see [resource-limits.md](resource-limits.md)) applies:
  below the floor no new era files are written and the export pauses rather
  than filling the volume. The NAS pool has terabytes; check with
  `ssh poc-nas df -h ~/AI` before pointing a new chain at it.
- Losing the data dir means re-syncing from genesis (or a certified
  snapshot); the export set itself is the durable copy mirrors hold.
