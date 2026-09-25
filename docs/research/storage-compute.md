# Free storage and compute for aether-node (checked 2026-09-25)

## The big 2026 changes

- **IPFS public gateways are gone.** Protocol Labs retired ipfs.io and dweb.link (rate-limited from 2026-09-01, retired 2026-09-21). Both now return HTTP 429, and so do w3s.link and storacha.link. Backend clients are told to run their own node (Kubo, Rainbow, Someguy) or use @helia/verified-fetch.
- **Oracle Always Free A1 was cut in half** on 2026-06-15, to 2 OCPU / 12 GB.
- **Storacha (web3.storage) looks dead.** storacha.network returns a 301 redirect to fil.one, a paid S3 service at $4.99/TB with a 1 TB, 30-day trial. Its gateways return 429. I could not confirm an official shutdown notice, so treat the exact status as unverified, but don't build on it.
- **Other free tiers are thinner:** AWS gives new accounts only credits for 6 months; Fly.io and Storj have no free tier; Pinata's free plan is 1 GB.

## A. Storage

| Option | What you get free | Blockchain use / ToS risk | Status |
|---|---|---|---|
| **GitHub Releases** | Each file under 2 GiB, up to 1000 files per release, no total size or bandwidth cap | Low: shipping your own software is the intended use | Verified in docs |
| **Git LFS** | 10 GB storage and 10 GB bandwidth per month (Free/Pro), metered billing beyond that ($0.07/GiB storage, $0.0875/GiB bandwidth) | Bandwidth runs out quickly. Don't use it for snapshots | Verified |
| **Cloudflare R2** | 10 GB-month, 1M Class A and 10M Class B operations, **no egress fees at any volume** | Low for static files. Needs a card | Verified 2026 |
| **Backblaze B2** | First 10 GB free, egress free up to 3x stored data (then $0.01/GB), standard API calls free since May 2026 | Low | Verified (3rd-party summary of a Mar 2026 update) |
| **Storj** | No free tier since 2024. 30-day trial, then $5/month minimum | n/a | Verified |
| **Oracle Object Storage** | 20 GB Always Free (from memory, not re-checked) | The Acceptable Use Policy bans crypto mining. A non-mining node is a grey area | Unverified |
| **Hugging Face Hub** | Public repos: "best-effort", usually up to about 5 TB, but content must be "useful to the community" | Good for **AI model weights and datasets**. Using it as a blockchain snapshot mirror is a likely ToS problem | Verified |
| **Zenodo** | 50 GB and 100 files per record, unlimited records, gets a DOI, more quota on request | Only fits benchmark datasets or research artifacts. Records are permanent and can't be changed | Verified |
| **GitHub Pages** | Small static site (from memory, not re-checked) | Fine for docs or a snapshot manifest, not bulk data | Unverified |
| **Internet Archive** | Free and permanent, but slow | Only for archival history | Unverified |
| **Pinata** | 1 GB, 500 files, 10 GB bandwidth, 60 requests/min | Too small to matter | Verified |
| **Filebase** | 5 GB, 1000 files, a dedicated IPFS gateway with about 5 GB bandwidth, no card | Fine for small items like manifests | Verified |
| **Self-run IPFS (Kubo/Helia/Rainbow)** | Free apart from your own bandwidth | This is now the only IPFS option that works | Verified |
| **iroh-blobs (Rust)** | Hash-verified transfer of large blobs between peers | Iroh 1.0 released 2026-07-09. I couldn't confirm the iroh-blobs version (last seen around 0.9x) | Mostly verified |
| **BitTorrent + webseed (librqbit)** | Peers seed snapshots to each other. The webseed can point at R2 or GitHub Releases | Mature Apache-2.0 library, about 18k downloads/month. Latest version unverified | Verified |
| **Arweave** | Pay once, store permanently. Not free | n/a | Note only |
| **Walrus** | About $0.023 per GB-month, paid in WAL (USD-pegged). Tusky, a service built on Walrus, closed in Jan 2026 | Not free | Verified |
| **Filecoin free programs, Celestia/Avail data availability** | Paid per blob. No free programs found | Not a fit | Unverified |
| **Users' nodes with erasure coding** | Free and scales with the network | You build it yourself. Simplest to add chunked snapshots over iroh-blobs or BitTorrent | Design choice |

## B. Compute

| Option | What you get free | Blockchain use / ToS risk | Status |
|---|---|---|---|
| **GitHub Actions** | Public repos are free on all hosted runners, **including arm64 and macOS**. The planned fee for self-hosted runners was postponed indefinitely | Low for CI, benchmarks and short testnets. Not for long-running bootnodes (jobs time out after 6 h) | Verified (Jan 2026 price change) |
| **CodSpeed** | 600 macro-runner minutes/month on bare-metal ARM64 machines (16 cores, 32 GB) for wall-time benchmarks. Open-source projects can ask for more | Low | Verified (Sep 2025 changelog) |
| **Bencher.dev** | Free for public projects (tracking and regression alerts). Its bare-metal runners are paid per minute | Low | Verified |
| **Oracle Always Free A1** | **2 OCPU / 12 GB** (1,500 OCPU-h and 9,000 GB-h per month). Pay-as-you-go accounts reportedly still get 4/24 at no charge. A terminated instance may not be recreated above the new limit | The Acceptable Use Policy bans "crypto mining" with no exception for proof-of-stake. Free accounts get reclaimed often. **Medium-High** | Verified (InfoQ, Jul 2026) |
| **GCP e2-micro** | 1 VM in us-west1, us-central1 or us-east1, 30 GB standard disk, always free | Mining is banned without written approval. Proof-of-stake-style nodes reportedly don't need verification. **Medium**: fine for a light bootnode | Mostly verified |
| **AWS** | Accounts created since 2025-07-15 get $100 + $100 in credits and a free plan lasting 6 months or until credits run out | Temporary only | Verified |
| **Azure** | $200 credit for 30 days, then 750 h/month of B1s or B2pts/B2ats VMs for 12 months | Temporary. Mining banned | Verified |
| **Fly.io** | No free tier for new accounts. Trial is 2 VM-hours or 7 days | n/a | Verified |
| **Kaggle** | About 30 GPU-hours/week (varies with demand), 12 h sessions | Fine for training a small classifier | Verified |
| **Google Colab (free)** | T4 GPU, sessions up to 12 h, no fixed quota, availability not guaranteed | Fine for training | Verified |
| **Hugging Face ZeroGPU** | About 3.5-5 minutes/day (sources disagree) | Only useful for demos | Partially verified |
| **Lightning AI** | About 15 credits/month, roughly 22 T4-hours | Fine for training | Figures inconsistent across sources |
| **Modal** | $30/month in credits (about 50 T4-hours) | Fine for training and batch jobs | Verified |
| **Cloudflare Workers AI** | 10,000 neurons/day, hard cap on the free plan | Only suits small inference, e.g. spam scoring through a Worker. Your model would need to be one Cloudflare hosts | Verified |
| **Volunteer compute (users' nodes)** | Free | No mature Rust BOINC-style framework found. You'd build task distribution on your own P2P layer | Unverified |
| **Open-source credit programs (AWS/GCP)** | Application-based, not guaranteed. Equinix Metal is shut down | Unverified | Unverified |

**On the ToS question:** every major cloud bans mining. Only GCP's wording hints that non-mining, proof-of-stake-style nodes are OK. Oracle's wording is blanket and free accounts get reclaimed. A validator or bootnode is not mining, but automated abuse detection can still flag heavy peer-to-peer traffic.

## Recommended free stack

**Storage, in priority order:**

1. **GitHub Releases** for binaries and DMGs, plus snapshot chunks under 2 GiB (no bandwidth cap).
2. **Peer-to-peer snapshot distribution between nodes**, using iroh-blobs or librqbit. Put a content hash in the chain so peers can verify what they download.
3. **Cloudflare R2** as the webseed or HTTP fallback (egress is free). Keep the latest snapshot there, under 10 GB.
4. **Hugging Face Hub** for AI model weights and datasets only.
5. **Zenodo** for frozen benchmark datasets, with a DOI.
6. **Backblaze B2** as a paid-overflow backup.

Skip public IPFS gateways, Pinata and Storacha.

**Compute, in priority order:**

1. **GitHub Actions** (public repo) for CI, including arm64 and macOS builds, and short multi-node testnets.
2. **CodSpeed** for benchmark regression checks, with **Bencher** as a free tracker.
3. **Kaggle**, then **Colab**, then **Modal** credits for training the classifier.
4. **GCP e2-micro** as a free, always-on bootnode.
5. **Oracle A1 (2/12)** as a second bootnode, accepting the reclaim and ToS risk.

Your own poc-cuda and poc-nas machines are a safer home for anything you need to keep running permanently.

## Not confirmed

- Exact ZeroGPU and Lightning AI numbers (sources disagree).
- The latest librqbit and iroh-blobs versions.
- Oracle's free object storage limit; GitHub Pages and Internet Archive limits.
- Storacha's official shutdown notice.
- Any Rust framework for volunteer computing.
- Cloud open-source credit programs.

## Sources

- [InfoQ - Oracle A1 halved](https://www.infoq.com/news/2026/07/oracle-cloud-free-tier-limits/)
- [IPFS blog - beyond sponsored gateways](https://blog.ipfs.tech/2026-08-beyond-sponsored-gateways/)
- [orbitdb-storage-bridge #111 (gateways retired 2026-09-21)](https://github.com/NiKrause/orbitdb-storage-bridge/issues/111)
- [fil.one](https://fil.one/)
- [Pinata limits](https://docs.pinata.cloud/account-management/limits)
- [Filebase](https://filebase.com/)
- [R2 pricing](https://developers.cloudflare.com/r2/pricing)
- [B2 pricing](https://www.backblaze.com/cloud-storage/pricing)
- [Storj free tier discontinued](https://forum.storj.io/t/discontinuation-of-the-storj-free-tier/25332)
- [GitHub Releases docs](https://docs.github.com/en/repositories/releasing-projects-on-github/about-releases)
- [Git LFS billing](https://docs.github.com/billing/managing-billing-for-git-large-file-storage/about-billing-for-git-large-file-storage)
- [HF storage limits](https://huggingface.co/docs/hub/en/storage-limits)
- [Zenodo quota](https://help.zenodo.org/docs/deposit/manage-quota/)
- [Walrus costs](https://docs.wal.app/docs/system-overview/storage-costs)
- [Tusky shutdown](https://docs.tusky.io/)
- [librqbit](https://lib.rs/crates/librqbit)
- [Iroh 1.0](https://www.iroh.computer/blog/the-road-to-iroh-1-0)
- [GitHub Actions pricing change](https://github.blog/changelog/2026-01-01-reduced-pricing-for-github-hosted-runners-usage/)
- [CodSpeed minutes](https://codspeed.io/changelog/2025-09-08-more-free-macro-runners-minutes)
- [Bencher pricing](https://bencher.dev/pricing/)
- [AWS free tier 2025](https://aws.amazon.com/about-aws/whats-new/2025/07/aws-free-tier-credits-month-free-plan/)
- [Azure free services](https://learn.microsoft.com/en-us/azure/cost-management-billing/manage/create-free-services)
- [GCP free-tier guide](https://agentdeals.dev/gcp-free-tier-2026)
- [GCP trial terms](https://cloud.google.com/terms/free-trial)
- [Hivelocity - cloud ToS vs validators](https://www.hivelocity.net/blog/cloud-tos-crypto-when-your-validator-is-not-welcome/)
- [Kaggle GPU docs](https://www.kaggle.com/docs/efficient-gpu-usage)
- [Colab FAQ](https://research.google.com/colaboratory/faq.html)
- [ZeroGPU docs](https://huggingface.co/docs/hub/en/spaces-zerogpu)
- [Modal pricing](https://modal.com/pricing)
- [Lightning pricing](https://lightning.ai/pricing/)
- [Workers AI pricing](https://developers.cloudflare.com/workers-ai/platform/pricing/index.md)
- [Fly.io free tier 2026](https://www.saaspricepulse.com/blog/flyio-free-tier-2026)
