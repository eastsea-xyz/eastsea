# Free-riding on public network infrastructure for aether-node (checked 2026-09-25)

**Main finding:** Metered Open Relay (free TURN) is effectively dead. Its DNS name still resolves (openrelay.metered.ca -> 37.27.44.221), but it gave no STUN reply on UDP 80 or 3478 in a live probe. A GitHub PR merged 2026-09-24 reports the same. Google and Cloudflare STUN both answered in the same test. Any "free TURN" fallback should point at a relay you run yourself.

**How this was checked:** web search, DNS lookups, STUN binding probes, and the crates.io API for crate versions, all on 2026-09-25. "Probed" = tested live today. "Docs/press" = only what a source says. "Unverified" = nothing to go on.

## 1. Discovery
| Name | Gives | Cost/limits | Rust crate | ToS/ethics risk | Status |
|---|---|---|---|---|---|
| Mainline DHT BEP 5 (current setup) | Peer lists by info_hash | Free; records must be re-announced periodically | `mainline` 8.0.0 (also published as `dht` 7.0.0, same description) | **Med**: BitTorrent protocol; corporate/university firewalls and some ISPs block DHT/UDP; can look bad to IT | Probed: router.bittorrent.com and dht.transmissionbt.com resolve |
| BEP 44 / **Pkarr** | Signed, mutable DNS records keyed by an ed25519 key (bootnode list you can update without a server) | Free; ~1000-byte limit per record; must be republished every few hours | `pkarr` 8.0.2 (2026-09-23), `n0-mainline` 0.6.0 | **Low-Med**: small signed records; established use (Pubky, iroh) | Crates current |
| IPFS Amino DHT + `bootstrap.libp2p.io` | Large public Kademlia network | Free | `libp2p` 0.57.0 | **Med**: IPFS docs say nothing about non-IPFS use; Amino peers are expected to implement `/ipfs/kad/1.0.0`. Run your own DHT protocol ID and use their bootstrappers only as entry points | Probed: `_dnsaddr` TXT records live |
| Hyperswarm / HyperDHT | Topic-based discovery plus hole-punching | Free | `peeroxide` 1.7.3 (claims wire-compatibility with Node.js network), `hyperdht` 0.1.1 | **Low**: generic, app-neutral network | Crates exist; interop not tested by me |
| Nostr relays | Seed lists as replaceable (kind 30078) or ephemeral (20000-29999) events | Free relays rate-limit and increasingly block bot-like traffic from throwaway keys | `nostr-sdk` 0.45.4 | **Low-Med**: use a stable key, low frequency | Probed: relay.damus.io, nos.lol, relay.nostr.band resolve |
| Ethereum discv5 (piggyback via ENR) | Encrypted discovery; ENRs can carry custom fields | Free | `discv5` 0.12.0 (sigp) | **Med-High**: filling Ethereum routing tables with foreign nodes is parasitic. Use the protocol on a separate network instead (Portal/trin does this) | Crate current |
| Public BitTorrent trackers (e.g. opentrackr) | HTTP/UDP announce for a fixed info_hash | Free | small; hand-rolled | **Med**: content-neutral, but trackers act on DMCA hash takedown lists; same BitTorrent optics | Probed: tracker.opentrackr.org resolves |

## 2. NAT traversal and relaying
| Name | Gives | Cost/limits | Rust crate | Risk | Status |
|---|---|---|---|---|---|
| Google STUN (stun.l.google.com:19302) | Public address discovery | Free, no SLA | `stun-rs`, `str0m` | Low | **Probed OK** |
| Cloudflare STUN (stun.cloudflare.com:3478) | Public address discovery | Free, "unlimited" per docs | same | Low | **Probed OK** |
| Cloudflare Realtime TURN | TURN relay | 1,000 GB/mo free (shared with SFU), then $0.05/GB; account + API key needed | `webrtc` 0.21.0 | Low | Docs |
| Metered Open Relay | Free TURN (was 20 GB/mo) | n/a | n/a | n/a | **Effectively dead** (see top) |
| iroh n0 public relays | QUIC relay + hole-punching, dial by public key | Free, rate-limited, "development/testing" only, no SLA; paid Pro (shared) / Enterprise (dedicated) tiers | `iroh` 1.2.0, `iroh-relay` 1.2.0 | Low for dev; do not depend on them in production | **Probed**: use1-1/euc1-1/aps1-1.relay.n0.iroh.link resolve, HTTP 200 |
| Self-hosted `iroh-relay` | Same, on your own box | VM cost only | `iroh-relay` | Low | Open source |
| libp2p Circuit Relay v2 + DCUtR | Relay + coordinated hole-punch | Needs relays you run; measured hole-punch success ~60-70% | `libp2p` | Low if self-hosted | Docs/papers |
| Tailscale DERP / `derper` | Relay | Tailscale's own DERPs serve Tailscale clients only; self-hosted `derper` free | Go only (no Rust) | **High** if using Tailscale's DERPs; Low if self-hosted | Docs |
| Tor onion services via Arti | Reachability without port forwarding, even behind CGNAT | Free; high latency; load on volunteer relays | `arti-client` 0.46.0, `tor-hsservice` 0.46.0 | **Med**: optics and volunteer burden; last resort only | Arti 2.x stable with onion services |
| WebRTC (ICE/STUN/TURN) | Browser-compatible NAT traversal | Needs STUN (free) + TURN (see above) | `webrtc` 0.21.0, `str0m` 0.23.1 | Low | Crates current |

## 3. Free hosting for bootnodes, seeds, relays
| Name | Limits (2026) | Fit | Risk | Status |
|---|---|---|---|---|
| Oracle Cloud Always Free Ampere A1 | **Halved 2026-06-15** to 2 OCPU / 12 GB (1,500 OCPU-h, 9,000 GB-h/mo); idle reclaim if CPU/net/mem all <20% over 7 days | Best free always-on VM; UDP works; good for relay + bootnode | Med: limits changed without notice | Docs + InfoQ |
| GCP e2-micro | 1 VM, only us-west1/us-central1/us-east1; 30 GB disk; **1 GB/mo egress** | Bootnode yes; relay no | Low | Docs |
| Fly.io | No free tier (2 VM-hour / 7-day trial) | No | n/a | Press |
| Railway | No free tier ($5 one-time trial, then $5/mo) | No | n/a | Press |
| Render free | 750 h/mo; sleeps after 15 min idle; HTTP only | No for P2P | n/a | Press |
| Cloudflare Workers + Durable Objects | ~3M DO requests/mo free; incoming WebSocket messages billed 20:1; 1 GB SQLite storage free | Good for WebSocket rendezvous/signaling or HTTPS seed API; no UDP | Low for signaling; don't stream bulk data | Docs |
| Cloudflare Tunnel | Free | Exposes a TCP/WebSocket service without port forwarding | Low-Med: ToS §2.8 bars non-HTML bulk content | Docs |
| GitHub Pages / raw | Free | Static signed seed lists | Low | Known |
| Cloudflare free DNS (TXT/A seeds) | Free | DNS seeds | Low | Known |

## Recommended layered stack (solo developer, priority order)
1. **Discovery, primary:** publish a signed bootnode list via Pkarr/BEP 44 on Mainline; keep the existing BEP 5 announce. No more hand-editing `peers.json`, and nobody can forge the list.
2. **Static seeds:** GitHub raw `peers.json` plus DNS TXT seeds on Cloudflare free DNS, signed with the same key.
3. **Firewall-proof discovery:** Cloudflare Worker/Durable Object rendezvous over HTTPS/WSS (port 443 passes networks that block DHT/UDP), plus one Nostr replaceable event as another channel.
4. **Public address:** Google + Cloudflare STUN (both verified).
5. **Connectivity:** adopt iroh. n0 public relays during development; self-hosted `iroh-relay` on Oracle A1 in production (keep it above 20% use to avoid reclaim).
6. **Backup node:** GCP e2-micro as a second bootnode (not a relay: 1 GB egress cap).
7. **Last resort:** Arti onion service for nodes behind CGNAT that cannot hole-punch; off by default.

**Avoid:** discv5 on Ethereum mainnet's network, Tailscale's own DERPs, Metered Open Relay.

## Sources
- openrelay PR #12: https://github.com/jamesroblarsen/the-republic/pull/12
- iroh relays: https://docs.iroh.computer/concepts/relays
- InfoQ, Oracle cut: https://www.infoq.com/news/2026/07/oracle-cloud-free-tier-limits/
- Oracle Always Free: https://docs.oracle.com/en-us/iaas/Content/FreeTier/freetier_topic-Always_Free_Resources.htm
- GCP free tier: https://cloud.google.com/free/docs/compute-getting-started
- Render free tiers 2026: https://render.com/articles/platforms-with-a-real-free-tier-for-developers-in-2026
- Fly.io free tier: https://www.saaspricepulse.com/tools/flyio
- Workers/DO pricing: https://developers.cloudflare.com/workers/platform/pricing/
- Cloudflare TURN FAQ: https://developers.cloudflare.com/realtime/turn/faq/
- IPFS public utilities: https://docs.ipfs.tech/concepts/public-utilities/
- Amino DHT spec: https://specs.ipfs.tech/routing/kad-dht/
- pkarr: https://github.com/pubky/pkarr
- peeroxide: https://github.com/Rightbracket/peeroxide
- sigp/discv5: https://github.com/sigp/discv5
- DCUtR measurement: https://arxiv.org/pdf/2604.12484
- Arti 2.5.1: https://blog.torproject.org/arti_2_5_1_released/
- Tailscale DERP: https://tailscale.com/docs/reference/derp-servers
- Nostr relay directory: https://d-central.tech/nostr-relay-directory/
- OpenTrackr hash blocks: https://torrentfreak.com/bittorrent-tracker-blocks-thousands-of-infringing-hashes-240103/
