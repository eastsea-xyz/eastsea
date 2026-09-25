# Free content-addressed distribution alternatives to IPFS (verified 2026-09-25)

Context: ipfs.io / dweb.link returned 429 + Sunset header from 2026-09-21 (now "service-worker gateway only", i.e. inbrowser.link); Shipyard ends all IPFS work 2026-09-30 after Protocol Labs cut funding. cloudflare-ipfs.com died 2024-08-14. Fleek IPFS hosting ended 2026-01-31. nft.storage Classic uploads off since 2024-06-30. Storacha/web3.storage: gone. "Rust" column = usable from a Rust macOS app (client crate or plain HTTP/S3).

## 1. IPFS ecosystem survivors

| Service | What's free | Exact limits | Alive? | ToS risk (chain data) | Rust |
|---|---|---|---|---|---|
| Filebase | 5 GB pin + dedicated gateway | 5 GB storage, 1000 pins, 5 GB/mo gateway egress, no S3 egress fee | Yes, 2026-09 (blog reaffirms free tier) | Low | S3 crates |
| 4EVERLAND | 5 GB IPFS + free dedicated gateway | 100 GB/mo gateway data; public gateway 300 rpm | Yes, 2026 | Low | HTTP/S3 |
| Pinata | 1 GB | 500 files, 10 GB/mo bandwidth, 10k req/mo | Yes, 2026-09 | Low | HTTP |
| Lighthouse.storage | 5 GB (Filecoin + Walrus backends) | 5 GB; paid $12/500 GB | Yes, 2026-08 | Low | HTTP |
| Crust Files | "unlimited" promo storage | promo via Discord; retrieval via IPFS public gateways (now gone) → effectively needs own node | Alive but unverified quotas | Medium | HTTP |
| Aleph Cloud | credits via accelerator only | none general | Alive; no general free tier | — | unverified |
| Swarm (bee) | none; postage stamps cost xBZZ on Gnosis | — | Alive (gateway repos active 2026-09) | Low | no crate; HTTP |
| Autonomi | none; pay ANT per upload | — | Alive (stable-2026.2.3.2) | Low | native Rust (`autonomi` crate) |
| Public gateways | inbrowser.link (browser only); gateway.pinata.cloud / *.4everland.io / *.myfilebase.com for own pins | rate-limited | — | — | — |

## 2. Non-IPFS content-addressed P2P

| Service | What's free | Exact limits | Alive? | ToS risk | Rust |
|---|---|---|---|---|---|
| iroh 1.0 + iroh-blobs (n0) | Public relays free (relay only; n0 hosts no blob storage) | relay bandwidth "starts free", unstated cap; you seed | Yes: iroh 1.0 2026-06-15, iroh-relay 1.1.0 2026-08-25 | Low | Native Rust, BLAKE3 verified streaming — best fit |
| Hypercore/Hyperdrive (Pear 3.5.0, 2026-09-22) | Free DHT, self-seed | none | Yes | Low | No official crate (JS); unverified Rust ports |
| Veilid 0.5.6 (2026-07) | Free DHT/network | DHT records small; not a blob store | Yes | Low | Native Rust (`veilid-core`) |
| Hyphanet 1507 (2026-09-13) | Free, anonymous | slow; large-file churn | Yes | Low | Java node only, FCP over TCP |
| Nostr Blossom (blossom.nostr.build, blossom.band) | Free SHA-256 blob hosting | 20 MiB/file free (100 MiB paid), unlimited count/retention | Yes, 2026 | Medium (media-oriented) | HTTP + nostr crates |
| Gun.js / Dat | legacy; Dat succeeded by Hypercore | — | Dat effectively dead | — | no |

## 3. BitTorrent as infrastructure

| Service | What's free | Exact limits | Alive? | ToS risk | Rust |
|---|---|---|---|---|---|
| Public trackers | opentrackr.org:1337, opentracker.io, exodus.desync.com, demonii.com (~80% of public traffic) | none; use ngosang/trackerslist (updated 2026-09-24) + DHT (BEP 5 you already have) | Yes | Low | librqbit / cratetorrent |
| Internet Archive | Unlimited upload, auto-generates .torrent with IA as webseed | no documented hard cap; huge items via email | Yes (1.4M torrent items) | Low-medium (must be archival, not app CDN) | S3-like API |
| Academic Torrents | Unlimited size datasets, tracker + webseed mirror | "no limit on size" | Yes | Medium (academic scope) | HTTP |
| Webseeds | Any HTTP host below (R2, B2, GitHub Releases) works as BEP 19 webseed | per-host | — | — | librqbit supports url-list |

## 4. Centralized-but-free CDN/object storage

| Service | What's free | Exact limits | Alive? | ToS risk | Rust |
|---|---|---|---|---|---|
| GitHub Releases | Unlimited assets, unlimited bandwidth | 2 GiB/file, 1000 assets/release | Yes | Low | HTTP/`octocrab` |
| GHCR (OCI artifacts) | Public packages: storage + bandwidth "currently free" | soft fair-use; no pull limits public | Yes 2026-04 | Low-medium (fair-use team) | `oci-distribution` crate |
| Cloudflare R2 | 10 GB-mo, 1M Class A, 10M Class B, zero egress | Standard class only | Yes | Low | S3 crates |
| Backblaze B2 | 10 GB storage, egress free to 3x stored, API calls free (2026-05) | $0.01/GB beyond; free egress unlimited via Cloudflare (Bandwidth Alliance) | Yes 2026-03 | Low | S3 crates |
| Cloudflare Pages | Unlimited bandwidth, 500 builds/mo | 25 MiB/file, 20k files (docs) | Yes | Low (small files only) | HTTP |
| Netlify / Vercel Hobby | 100 GB/mo bandwidth each | non-commercial (Vercel) | Yes | Medium | HTTP |
| jsDelivr | GitHub repo/release CDN | 50 MB/file (20 MB some types), pkg 50–150 MB | Yes | Low for small | HTTP |
| npm registry | packages | practical ~50–100 MB tarball; ToS forbids non-package blobs | Yes | High | — |
| Hugging Face Hub | Public repos best-effort up to 5 TB, 300 GB/repo | must be "useful to community"; abuse mitigations | Yes 2026 | Medium for non-ML data (chain snapshots plausibly OK as dataset) | `hf-hub` crate |
| Zenodo | 50 GB/record, unlimited records, +150 GB extra | 100 files/record, DOI, immutable | Yes | Medium (research scope) | HTTP |
| OSF.io | 50 GB public project | 5 GB private | Yes | Medium | HTTP |
| Codeberg | releases | 100 MB/attachment | Yes | Low | HTTP |
| Sourcehut pages | 1 GiB tarball | paid account required | Yes | — | HTTP |
| GitLab.com | releases/packages | 5 GB package file (unverified 2026) | unverified | Low | HTTP |

## 5. Decentralized storage with real free GBs

| Service | What's free | Exact limits | Alive? | ToS risk | Rust |
|---|---|---|---|---|---|
| Filecoin via Lighthouse | 5 GB | see §1 | Yes | Low | HTTP |
| Arweave via Irys/ar.io Turbo | Files < 100 KiB free, no account | ~$8/GB above | Yes | Low | HTTP |
| Walrus | Testnet only (free WAL faucet); mainnet $0.023/GB-mo, foundation subsidy contract | testnet wiped periodically | Yes | Low | Rust SDK (`walrus` crates) |
| Storj | Free tier ended 2024-04-01; $5/mo minimum | 30-day trial | Yes | — | S3 |
| Sia (renterd 2.9.1) | Skynet dead 2022; must pay SC | — | Yes | Low | S3 API |
| Shadow Drive | last release 2024-07; pay SHDW | — | unverified/stale | — | — |
| BNB Greenfield | pay BNB | — | Yes (2026 roadmap) | — | Go SDK only |
| Autonomi | pay ANT | — | Yes | Low | Rust native |
| Celestia / Avail DA | pay TIA/AVAIL; 8 MiB / ~4 MB per blob | pruned after ~30 days, not storage | Yes | — | Rust clients |

## Recommended free stack

**(a) Chain history chunks (100 MB–2 GiB, hash-verified):**
1. GitHub Releases (2 GiB/file, no bandwidth cap) — primary HTTP source and BitTorrent webseed.
2. BitTorrent (your BEP 5 DHT + opentrackr/opentracker.io) with GitHub/R2/B2 as BEP 19 webseeds; librqbit in-app.
3. Internet Archive item (auto-torrent, webseed) as durable third mirror; Hugging Face dataset repo (hf-hub) as fourth if chunks > 2 GiB.
4. iroh-blobs peer swarm between wallets (BLAKE3 verified; n0 relays free) for hot recent chunks.

**(b) State snapshots (large, rotate frequently):**
1. Cloudflare R2 (10 GB, zero egress) + Backblaze B2 (10 GB, 3x egress; front with Cloudflare for unlimited) — rotate, keep latest 2.
2. Hugging Face dataset repo (300 GB/repo) for archival snapshots.
3. iroh-blobs seeding from full nodes; fall back to Zenodo (50 GB, immutable) for milestone snapshots.

**(c) Release binaries (tens of MB):**
1. GitHub Releases (Sparkle/appcast points here).
2. GHCR as OCI artifact (`oci-distribution` crate) — independent auth/CDN path.
3. jsDelivr mirror of the GitHub release asset (< 50 MB) + Nostr Blossom (< 20 MiB per blob, SHA-256 addressed) + Filebase/4EVERLAND IPFS pin with their dedicated gateway.

Rule: never depend on a single IPFS gateway again; embed a manifest listing all mirrors with the content hash and let the client race them.
