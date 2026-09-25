# Aether token-economics research (2026-09-26)

Note on method: WebSearch hit its session budget (200/200) after the first query, so everything here comes from pages fetched directly (WebFetch). Anything marked **(unverified)** is from memory and was not re-checked. Treat those as leads, not facts.

## 1. Ethereum
| Item | Value | Source |
|---|---|---|
| Issuance | ~1,700 ETH/day at ~14M staked; ~0.52%/yr at that stake level. It grows with the square root of total stake, so it is higher today. | ethereum.org/roadmap/merge/issuance (updated 2026-06-06) |
| Supply | ~122.07M ETH; gas ~0.5 gwei. Burn needs ~16 gwei average to offset issuance, so ETH is **net inflationary in 2025-26**. | ultrasound.money (fetched 2026-09-26) |
| Fee handling | EIP-1559 base fee is burned. Priority fee and MEV go to the proposer. | ethereum.org |
| Reward weights | Source 14, target 26, head 14, sync committee 2, proposer 8 (out of 64) | consensus spec constants **(unverified this session)** |
| ePBS (EIP-7732) | Builder bid is deducted from the builder's beacon balance and paid to the proposer trustlessly. In review; no fork named on the EIP page. | eips.ethereum.org/EIPS/eip-7732 |
| FOCIL (EIP-7805) | Draft. **No reward for inclusion-list committee members**; relies on a 1-of-N honesty assumption. | eips.ethereum.org/EIPS/eip-7805 |
| Anti-correlation (EIP-7716) | Penalty factor up to 4x when many validators miss attestations together. **Stagnant.** | eips.ethereum.org/EIPS/eip-7716 |
| Tapered issuance / "EIP-8363", MEV-burn | Status **(unverified)** | — |

## 2. Solana
| Item | Value | Source |
|---|---|---|
| Inflation | Started at 8%, falls 15%/yr to a 1.5% floor **(unverified this session)** | — |
| SIMD-0228 (issuance set by staking rate; aimed at 45-50% staked vs ~65% then) | PR **closed, not merged**. The March 2025 vote failed to reach its threshold **(vote numbers unverified)**. | github SIMD PR #228 |
| SIMD-0096 | **Activated.** 100% of priority fees go to the leader (was 50% burned). Base fee stays 50% burn / 50% leader. Known drawback: leaders can fake priority fees at almost no cost. | SIMD-0096 doc |
| Alpenglow (SIMD-0326) | Merged. Votes move off-chain, so there are no vote-transaction fees; BLS vote aggregates go in block footers. A validator that earns 0 SOL in an epoch is removed from the active set. Tolerates 20% Byzantine stake. VAT amount **(unverified)**. | github SIMD PR #326 |
| Jito tips | Go to leader and stakers through the tip router **(unverified)** | — |

## 3. Other L1s / DA
| Chain | Issuance | Fee handling | Who is rewarded | Source |
|---|---|---|---|---|
| Sui | 10B cap; temporary stake subsidies that phase out | Computation fees go to stakers. Storage fees go into a **storage fund**: only its returns are paid out, and users get a rebate when they delete data. | Stakers, split by stake deterministically. SIP-39 entry is by voting power (≥3 bps). | docs.sui.io/concepts/tokenomics |
| Aptos | `rewards_rate` set by governance | Burn not documented on the page | Reward = stake × rate × (successful proposals / total). Stake between 1M and 50M APT. | aptos.dev staking |
| Monad | 18 MON minted per block, fixed | EIP-1559 base fee (floor 100 MON-gwei), **charged on gas limit** rather than gas used | Leader gets the block reward plus priority fees; commission applies to the block reward only | docs.monad.xyz |
| Celestia | Inflation 8% at launch → ~5% (CIP-29, July 2025) → **~2.5% (CIP-41, Nov 2025)**, then falls 6.7%/yr to a 1.5% floor | Blob fees are paid in TIA; split not documented | Stakers | docs.celestia.org |
| Avalanche | — | ACP-77: L1 validators pay a **continuous fee, at least 512 nAVAX/s (~1.33 AVAX/month)**, rising exponentially above 10k validators | Primary network | ACPs #77 |
| Berachain | Fixed per block: 0.4 WBERA base rate plus 1.305 WBERA to reward vaults. **BGT deprecated 2026-07-08.** | — | Validator operator, and liquidity providers through vaults | docs.berachain.com/learn/pol |
| Near / Sei / Cosmos Hub | 70% burn / 30% to contracts; inflation cut 5%→2.5% in 2025 **(unverified)** | | | — |

## 4. Paying for proofs
| System | Mechanism | Source |
|---|---|---|
| Mina | Block producers **must buy SNARKs** in the "snarketplace" and pay provers out of their block reward. The cheapest bid for a job wins, and entry is permissionless. | docs.minaprotocol.com FAQ |
| Boundless (PoVW) | Provers earn ZKC for **metered proven cycles** per epoch. They must stake first, and per-epoch rewards are capped at stake/15. Work done before staking earns nothing. | docs.boundless.network ZK mining |
| Succinct | Auctions pick the most efficient prover; PROVE token covers payment, staking and governance. Details **(unverified)** (docs returned 404). | docs.succinct.xyz |
| Aleo, Bittensor | Coinbase puzzle, and emissions split among miners, validators and subnet owners **(unverified)** | — |

## 5. Home participation and anti-sybil lessons
- **Lido CSM:** permissionless, but each operator posts a bond that covers all its validators. Rewards are socialized when an operator performs above a threshold. v3 supports 0x01 validators; a 0x02 module is planned (docs.lido.fi).
- **Solana Alpenglow:** removes validators that earn nothing in an epoch, which is a cheap way to enforce liveness.
- **Ethereum:** FOCIL shows that "unpaid duties with a 1-of-N honesty assumption" is an accepted pattern. Anti-correlation penalties, which would favor solo stakers, have stalled.
- **Filecoin / Helium / Chia (unverified):** paying for hardware presence led to fake coverage in Helium, sealing capacity that nobody used in Filecoin, and emissions far beyond real demand. The lesson is to **pay for verifiable work that someone actually demanded**, not for uptime claims.

## 6. Regulatory (headline, unverified)
- US: the SEC has signaled in 2025-26 that protocol staking and mining rewards are generally not securities offerings, and market-structure legislation is in progress.
- Korea: the Digital Asset Basic Act is under discussion.
- Common ways projects de-risk: testnet points that cannot be transferred and promise no conversion, non-transferable reputation, burn-only fee designs with no issuance and no sale, and no pre-mine sales before legal sign-off.

---

## Options for Aether
Roles: **P** = proposer, **C** = committee voter, **W** = prover chunk worker, **V** = verifier, **DA** = Celestia cost.

**A. Burn-only plus mandatory prover payment (Mina-style). No issuance.**
- Base fee is burned (the design notes' intent).
- The proposer's priority fee must pay for proofs: a block is valid only with proof, and the proposer pays each chunk worker the price it bid in a chunk auction.
- A protocol-computed DA fee is deducted from the fee and sent to a DA treasury that pays Celestia.
- C and V get no pay (FOCIL-style honesty); a C member who never votes is dropped from the committee, as Alpenglow does.
- Pros: simplest; no inflation; pays for real work.
- Cons: low fees mean too few provers get paid; P can fake fees (the SIMD-0096 lesson), so burn a minimum share of the priority fee.
- Legal: lowest risk; fits a zero-value testnet.

**B. Small fixed issuance split by role (Monad/Berachain-style constant per block).**
- For example, X per block: 40% to W in proportion to proven cycles (Boundless-style metering), 25% to P, 25% to C, 10% to V.
- V earns only by submitting fraud or "proof-invalid" challenges, or through random spot-checks.
- Base fee is burned; the DA cost is paid from the base fee before burning.
- Pros: can start the network with no fee demand; W pay is metered and verifiable.
- Cons: dilution; Sybil pressure on W. Mitigate with stake-gated caps like Boundless (reward ≤ stake/k).
- Legal: issuance is a "reward" and needs review before tokens gain value. Keep it testnet-only until then.

**C. Stake-gated proof-of-verifiable-work plus a DA reserve fund (Sui storage-fund style).**
- A share of each fee goes to a DA fund; only the fund's yield or allotted budget pays Celestia.
- W must bond to be assigned chunks, and missed chunks are slashed from the bond (bond model like Lido CSM).
- P and C are paid from fees only.
- Pros: long-run sustainable DA costs; strong anti-Sybil protection for W.
- Cons: bonding keeps casual Macs out unless bonds are small or delegated.
- Legal: the bond or stake mechanics need review.

**D. Points now, token later (recommended for the current phase).**
- Keep the chain burn-only (A) with zero-value tokens.
- Record per-role work receipts on-chain: proven cycles, votes, verified proofs, blocks.
- Receipts form a non-transferable reputation that also drives committee selection weight and prover priority. No promise of conversion.
- Pros: gathers real participation data to calibrate B or C later; no securities surface.
- Cons: weak incentive for home users.
- Legal: must avoid implying future value. Get Korea/US counsel before any conversion.

**Implementation note for today's code:** stop sending 100% of fees to the proposer. Burn the base fee, split the priority fee between proposer and provers (with a burn floor), and add a DA-fee deduction hook. This fits A and D and keeps B and C open.
