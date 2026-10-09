# Should EastSea sell AI inference from people's Macs?

**Red-team review · 2026-10-09 · recommendation: NO-GO for a paid public marketplace; LATER for optional inference on the wallet owner's own machine.**

Founder instruction: “그냥 아이디어일 뿐인데, 레드팀 의견없이 받아들이지는 마.” This is an unadopted idea. This document is a review, not an implementation decision, launch plan, or authorization to add inference to EastSea.

Scope: unrelated Mac owners serve AI jobs; customers pay through the chain. Reviewed against `lead-merge` / lane baseline `fc75af2c969af9884d997e60bcc2abf2f40e5489`. No code, dependencies, model downloads, runtime experiments, builds, live-node access, deployments, or payments were performed.

## 1. Decision and the strongest case against

EastSea would enter a low-price inference business while assuming the additional obligations of an untrusted compute marketplace. It would need to sell reliability, validate computation, protect prompts, manage malicious model artifacts, settle failed jobs, and support household hardware. The chain solves payment authorization and settlement; it does not supply those missing guarantees.

The existing promise is **“Your money, checked by your own Mac.”** It already has an agent wallet without an inference marketplace. Mainnet has not launched, and the repository discloses that no independent security audit has occurred. Diverting attention or shared hardware to a new service before that core product is established is a poor trade. These are current repository facts, not a forecast about AI demand. [Site copy](../../site/index.html), [README](../../README.md).

| Proposal | Verdict | Reason / boundary |
|---|---|---|
| EastSea-operated public discovery, job routing, verification and payment marketplace | **NO-GO** | No demonstrated home-Mac paying niche, delivered-cost advantage, fleet-wide integrity contract, or mapped operating responsibilities. |
| Existing owner-limited agent payment tools | **Keep existing scope** | An agent can use a wallet without EastSea becoming its inference supplier. This review authorizes no new tools. |
| Owner's own local model, or their iPhone using their own trusted Mac | **LATER / separate decision** | Fits the privacy and ownership promise; validate utility and resource isolation after mainnet priorities. No inference-selling network is needed. |
| Privately managed cluster owned by one user or organization | **LATER / external experiment** | Can address model capacity without anonymous sellers. Its operator still owns its security and legal duties. |

The bear case survives even if decentralized inference is technically possible and some services have paying customers. EastSea must establish that **its particular fleet, workload, customer and operating model** have an advantage. “Idle Macs exist,” “AI is growing,” and “tokens can pay for it” do not establish that advantage.

### Evidence rules

All external observations are as of **2026-10-09**; publication/version dates appear where relevant. **Observed** means a source or exposed mechanism was inspected. **Reported** means a paper's experiment or a vendor's statement, with its scope preserved. **Inference** and **proposed gate** mean this review's reasoning or decision threshold. Missing disclosures mean **unknown**, not zero.

Keep five different quantities separate: requests/tokens processed; external customer payments; gross supplier settlements; platform revenue after payouts/refunds; and token emissions/subsidies. Funding, registered GPUs, users, stars, contract bookings and annualized run rates are not realized customer revenue. The review found no inspected dataset reconciling unrelated customers' payments with inference delivered on strangers' home Macs.

## 2. Demand: the buyer can already choose cheap APIs or their own Mac

### Today's price floor

USD per **1 million tokens**, input and output separately; standard synchronous uncached text rates displayed on the retrieved official pages. The example is **2,000 input + 1,000 output tokens**. Taxes, tools, reasoning tokens beyond this assumed output count, long-context uplifts and negotiated terms are excluded. The last column is arithmetic, not an observed bill.

| Provider / listed model | Input / 1M | Output / 1M | Example job |
|---|---:|---:|---:|
| OpenAI GPT-6 Luna | $0.10 | $0.50 | $0.000700 |
| OpenAI GPT-5 nano, still listed | $0.05 | $0.40 | $0.000500 |
| OpenAI GPT-6.1 Sol | $2.00 | $10.00 | $0.014000 |
| Anthropic Haiku 5.5, prompt ≤100K | $0.10 | $0.50 | $0.000700 |
| Anthropic Sonnet 5.5 | $2.00 | $10.00 | $0.014000 |
| Groq GPT-OSS 20B | $0.075 | $0.30 | $0.000450 |
| Groq GPT-OSS 120B | $0.15 | $0.60 | $0.000900 |
| Together Llama 3 8B Instruct Lite | $0.14 | $0.14 | $0.000420 |
| Together Qwen3.5 9B | $0.17 | $0.25 | $0.000590 |
| Together Llama 3.3 70B | $1.04 | $1.04 | $0.003120 |

Sources: [OpenAI official API pricing](https://developers.openai.com/api/docs/pricing), [Anthropic pricing](https://claude.com/pricing), [Groq production-model price table](https://console.groq.com/docs/models), [Together inference pricing](https://www.together.ai/pricing). OpenAI's quoted current-model rows use its short-context tier. Anthropic lists a higher Haiku tier above 100K. Groq now labels its Llama 8B/70B rows **Contact Sales**; old $0.05/$0.08 Llama quotes cannot be presented as today's public tariff. OpenAI's published batch/cached rates can lower costs further.

These models are not interchangeable in quality, language coverage, quantization or tool reliability. The test must compare the **same task at an accepted quality and latency**, preferably the same model/version, against its credible incumbent. Comparing a small quantized Mac model's price with a frontier API's price would invent a competitive advantage.

**Inference:** at $0.000450, saving even half the incumbent bill saves $0.000225 per job. Token acquisition, approvals, routing, cold starts and uncertainty about refunds can outweigh that saving. A buyer's own already-owned Mac has no per-request supplier bill and keeps data under the owner's control, although its electricity, setup and device opportunity cost remain real.

### What the named networks actually establish

| Network | Evidence inspected, with date and unit | What it does not establish |
|---|---|---|
| **Bittensor / Chutes** | Bittensor documents protocol emissions, not customer receipts. OpenRouter exposes priced Chutes models on the observation date. Chutes' 2026-03-20 update reports a 45% token-volume drop versus about 25% revenue drop after removing subsidized/loss-making workloads, including one at a 9:1 cost-to-revenue ratio. The percentages are vendor disclosures. | Emissions are not sales. A listed paid endpoint is not a reconciled paid invoice. The disclosure concerns a GPU service, including H200 capacity, and is not a home-Mac contribution-margin study. [Emissions](https://www.bittensor.com/docs/concepts/emissions), [OpenRouter listing](https://openrouter.ai/provider/chutes), [Chutes disclosure](https://chutes.ai/news/from-volume-to-value-building-a-sustainable-ai-inference-platform-2). |
| **Petals** | A primary paper, 2023-12-13, reports a real distributed BLOOM-176B experiment on 14 heterogeneous GPU servers, approximately 0.83 inference steps/s at sequence length 128 and 0.79 at 2,048. | A technical experiment is not current paid network utilization. No reconciled customer-revenue dataset was established. The live health dashboard was unavailable during retrieval, so this review reports no current swarm count. [Paper, Tables 2–3](https://arxiv.org/html/2312.08361v1), [Health dashboard attempted](https://health.petals.dev/). |
| **Exo** | Its local/offline cluster offering and repository support the relevance of owned Mac clusters. An undated historical M4 Pro 3B benchmark reports single-request throughput of 49.3, 44.4 and 39.7 tokens/s on one, two and three devices, respectively; multiple-request throughput improves. | This is a vendor benchmark, not paid customer demand. Current tensor-parallel/RDMA capabilities should not be reduced to that old pipeline result. A LAN/Thunderbolt cluster does not establish household WAN economics. [Site](https://exolabs.net/), [Benchmark](https://blog.exolabs.net/day-1/), [Repository](https://github.com/exo-explore/exo). |
| **Hyperbolic** | Its 2025-04-28 recap claims 112B+ tokens and 127,000+ GPU-hours, alongside free-credit programs. Billing documents expose purchased credits and hourly charging. Its 2026-01-21 dedicated-hosting announcement addresses shared-endpoint latency, noisy neighbors and isolation. | The totals are unverified vendor usage claims with unclear time bases, not independently reconciled paid volume; credits prevent multiplying usage by tariff to infer revenue. Dedicated capacity addresses requirements home Macs would have to satisfy. [Recap](https://www.hyperbolic.ai/blog/monthly-recap-april-2025), [Billing](https://www.hyperbolic.ai/docs/general/billing-payments), [Dedicated hosting](https://www.hyperbolic.ai/blog/dedicated-inference-hosting). |
| **io.net** | Its 2025-10-21 announcement claims >$20M annualized on-chain revenue; its 2026-06-12 article claims $8M in Q1 enterprise deals. Both are vendor statements. Its 2025-05-19 TNE explanation says it purchased IO to backdate historical revenue on chain. | Annualized rate ≠ collected annual revenue; bookings ≠ completed service; operator-origin chain transfers do not prove the original external payment. GPU-cluster business is not Mac-served inference demand. [Annualized claim](https://io.net/blog/io-net-20m-in-annualized-on-chain-revenue), [Bookings claim](https://io.net/blog/three-years-of-building-the-future-of-ai-compute), [TNE mechanism](https://io.net/blog/introducing-total-network-earnings-transparent-trust). |

Independent publication is not necessarily independent underlying evidence. The inspected [DefiLlama Chutes adapter](https://github.com/DefiLlama/dimension-adapters/blob/master/fees/chutes-ai.ts) obtains vendor API fields for subscriptions, pay-as-you-go, pending instances and sponsored inference, and reports the same sum as fees/revenue/protocol revenue. It does not reconcile customer receipts, refunds or subsidies. Its series starts 2025-12-24. The underlying [revenue endpoint](https://api.chutes.ai/daily_revenue_summary) returned HTTP 401 without authentication in this review; no numeric revenue snapshot was independently reproduced.

Likewise, [Syndica's October 2025 report, p.21](https://blog.syndica.io/content/files/2025/11/Deep-Dive---Solana-DePIN-October-2025-2.pdf) reports $931,000 for io.net but names io.net's website as its source. [CoinDesk Research's IDE report](https://www.coindesk.com/research/the-incentive-dynamic-engine-a-new-era-for-io-net-tokenomics) discloses that io.net commissioned it. These figures remain outside **proven independent customer spending**.

**Strongest counterargument:** Chutes' external distribution and candid operating disclosures make “nobody pays for decentralized inference” an unjustified claim. Real GPU-compute demand is credible. The missing bridge is evidence that unrelated home Macs can meet a paying customer's requirements more cheaply after verification, outages and support.

**Strongest Mac-specific defense, still a hypothesis:** suppliers already own high-memory desktop Macs; buyers need a licensed custom/large model unavailable from a suitable commodity API; jobs use non-personal, non-confidential data and tolerate interruption. Such buyers might pay for model access rather than the cheapest tokens. This avoids several privacy/battery/latency objections and deserves a fair test. It still needs evidence of enough suitable independent Macs, useful memory-bandwidth/throughput, paid utilization, safe fixed model profiles, verification and renewed customers. An owned cluster or contracted specialist host is a competing way to meet the same need. No inspected paid cohort establishes why EastSea's marketplace/payment layer is necessary for this niche.

### Candidate buyers, challenged

| Proposed buyer | Strongest objection | Evidence needed to overcome it |
|---|---|---|
| Privacy-conscious wallet owner | Strangers' Macs see the prompt; the owner's own Mac is the better privacy baseline. | A workload that is non-sensitive, or independently established confidential execution; explicit willingness to outsource. |
| Small app developer seeking cheap tokens | Existing API jobs already cost fractions of a cent and require no DBLN procurement. | Existing invoices plus repeat purchases at an all-in price, quality and p95 latency that overcome switching cost. |
| Buyer needing a large open/custom model | Many consumer Macs cannot fit the weights/KV cache; distributing a request adds communication and availability dependencies. | Same-model fleet measurements and a paid niche not served adequately by GPU rental or an owned cluster. |
| Enterprise, clinic or financial agent | Requires accountable suppliers, access controls, deletion, incident response and predictable service. | Real procurement acceptance of this supplier/data-flow model, not a general “interested in AI” interview. |
| Crypto-native autonomous agent | A payment mechanism is not a need for a new model host; approvals and budget loss still matter. | Unrelated agents' recurring paid workloads with no token subsidy or circular project funding. |
| Research hobbyist / buyer seeking unmoderated service | Volunteer interest need not convert to payment; an abuse-oriented niche increases operator exposure. | Lawful retained customers and positive margins with an enforceable acceptable-use scope. |

None of those buyer hypotheses is validated for EastSea by the inspected material.

## 3. Verification reality: define the guarantee before pricing it

### Bit-exact Metal/MLX replay is an unproven fleet assumption

Apple exposes Metal math modes: fast mode permits lossy transformations; safe mode restricts unsafe transformations. That is a compiler contract, not a documented guarantee of identical LLM results across all chip generations and OS releases. MLX documents hardware/backend-dependent numerical precision, and its random API has both implicit state and explicit keys. A seed does not pin kernels, floating-point reductions or random-state consumption. [Apple math modes](https://developer.apple.com/documentation/metal/mtlmathmode), [Safe mode](https://developer.apple.com/documentation/metal/mtlmathmode/safe), [MLX precision](https://ml-explore.github.io/mlx/build/html/usage/precision.html), [MLX random API](https://ml-explore.github.io/mlx/build/html/python/random.html).

There is bounded positive evidence: [MLX-LM discussion #1017](https://github.com/ml-explore/mlx-lm/discussions/1017), 2026-03-18, reports identical text over repeated prompts on a fixed M4 Max setup while excluding batch-invariant determinism. [Issue #1470](https://github.com/ml-explore/mlx-lm/issues/1470), 2026-07-04, reports temperature-zero divergence between plain and speculative generation on one M5 Max configuration. Both are contributor reports; neither establishes a cross-generation matrix or universal failure.

**Inference:** a nearly tied token can change after small logit drift, then alter the entire later context. Pinning weights and temperature zero is insufficient. A meaningful job specification would also pin tokenization/chat templates, adapters, quantization, engine/compiler versions, cache configuration, permitted math profile, sampling and RNG rules. Constraining batching and execution paths may improve reproducibility while sacrificing throughput. This is not proof that bit-exact execution is impossible; it is a reason not to assume it exists.

Four promises must be distinguished:

1. A provider signed a receipt.
2. A named model was evaluated on particular inputs or supplied tokens.
3. That model generated the completion under the agreed decoding policy.
4. The completion is factually correct, safe and useful.

A signature or model-file hash establishes neither 2 nor 3. A computation proof can establish a specified computation, not 4. A chain payment proof establishes payment, not successful service.

### Fast-verification research is real, with narrower guarantees

**TopLoc v2, 2025-05-30:** reports compact activation commitments, using tolerant comparison rather than bit-exact whole-output replay. Its §6.2 excludes verification of sampling; it leaves KV-cache compression discrimination untested and flags unstable-prompt false rejection in §6.3. An activation fingerprint must not be marketed as proof of the authorized generation policy. [Primary paper](https://arxiv.org/html/2501.16007v2).

**VeriLLM v4, 2026-01-22:** reports a 10-node 1-Gbps LAN experiment including two M4 Macs, six RTX 5090s and two A100s; claims approximately 1% verification cost under a global honest-majority assumption and calibrated numerical drift. Table 3's Mac comparison uses an **M4 CPU**, not a cross-Metal/MLX fleet. This contradicts a blanket dismissal of Mac verification research. However, Table 4 lists 33 ms verifier prefill versus 1,278 ms inference: **2.58% on that latency denominator**, or **3.76% including its 15 ms on-chain component**. The prose also gives 0.78%; reconcile sampling/amortization and cost denominators before borrowing the headline. Its reported approximately $0.004 L2 verification cost alone exceeds several example API job prices above; that is its environment, not an EastSea fee estimate. [Primary paper, §§6–7](https://arxiv.org/html/2509.24257v4).

**SVIP v3, 2026-01-31:** reports a learned proxy verifier, under 0.01 seconds per prompt, with average false-negative rate 3.49% and false-positive rates below 3% across its tested smaller models. A useful statistical model-identity classifier is not a cryptographic execution proof or a justified automatic slashing rule for arbitrary customer prompts. [Primary paper](https://arxiv.org/html/2410.22307v3).

Teacher-forced verification can evaluate supplied completion tokens in parallel instead of serially generating each token. It still needs the appropriate weights, memory and forward computation. Long prompts, short outputs, loading, concurrent requests and low utilization can erase much of the latency advantage. Splitting checking among devices can reduce elapsed time without reducing aggregate paid accelerator work. Report wall time, accelerator-seconds and actual cost separately.

### Relevant threat classes, not an attack implementation

These are threats to the proposed marketplace, not claims of reproduced EastSea vulnerabilities:

| Threat | Why a naive spot check fails | Required evidence / limitation |
|---|---|---|
| Lazy supplier or lazy checker | Correct receipts and occasional successful audits do not show every purchased job was computed or checked. | Demonstrate detection and rational incentives on actual billable jobs; include checker cost. |
| Model substitution / undisclosed precision | Plausible text and a claimed model hash do not prove use of the agreed weights, adapter or quantization. | Test identity detection on the supported profiles; state whether decoding is also covered. |
| Colluding provider/checkers | Multiple agreeing identities may share control, incentives or computation. | Establish the assumed independence/honest fraction; chain validator honesty does not automatically imply compute-provider honesty. |
| Prompt-dependent deviation | Calibration on ordinary prompts may not predict behavior on customer-selected distributions. | Held-out task families, length ranges and targeted failure classes; no general security claim from average accuracy. |
| Honest numerical drift | Equality checks can penalize honest providers; wider tolerance can accept deviations. | Fixed, preregistered thresholds with honest-rejection and bad-work-acceptance measurements; an appeal policy. |
| Correct computation, bad advice | A faithful model can hallucinate or issue unsafe financial instructions. | Separate model integrity from answer quality and wallet authorization. |

The [Petals security discussion](https://github.com/bigscience-workshop/petals/wiki/Security,-privacy,-and-AI-safety), last edited 2022-12-09, explicitly lacks a default correctness guarantee and proposes multiple-peer comparisons or a trusted private swarm. Replication buys evidence under a trust assumption; agreement does not create that assumption.

### Verification economics

Use an all-in cost per **accepted job**, not “verification takes 1% of generation time.” First estimate operating cost per submitted request:

    E_cost_per_request = C_provider + q × m × C_checker
        + C_commitment + C_transport + C_settlement
        + p_dispute × C_dispute + p_retry × C_retry

    C_operating_per_accepted = E_cost_per_request / p_accept

    E_buyer_harm_per_request = p_accepted_bad × L_bad

Here q is the audited fraction of initial requests, m the paid checkers per audited request, and the p terms are probabilities per submitted request. p_accept includes final rejection, outage and exhausted retries; bad work can still be accepted. C_retry includes all incremental retry computation, checking and transport, avoiding double counting. Amortize loading, idle availability, calibration, support and storage. For measured results use total operating spend divided by the number of accepted jobs.

Choose an accounting perspective explicitly: platform operating cost includes provider/checker payouts; supplier margin separately deducts electricity, equipment and its own support from payouts. Internal payouts are not additional external customer revenue. Buyer harm from accepted bad work is a **separate risk measure**, not supplier/platform operating cost or a substitute for contribution-margin accounting. L_bad can dwarf the token price for a paying agent.

Illustrative sensitivity, not a measured EastSea result: q=10%, m=2 and each checker costing half a supplier execution adds **10%** compute cost; auditing every job with two full-cost checkers adds **200%**. A checker costing 1% would make the first case 0.2%, but would not remove settlement, privacy, retries or the security assumptions.

For a rational supplier a necessary, insufficient deterrence condition is:

    q × d_effective × (recoverable stake + forfeited reward)
      > gain from deviation

Detection must include checker honesty/availability and the covered deviation class. Expensive collateral creates onboarding cost; numerical false positives create dispute cost. Lowering audit rates preserves margin only by accepting a changed integrity guarantee.

## 4. Security and privacy: whose computer sees whose data?

### Ordinary remote inference exposes the prompt to the supplier

Encryption in transit protects a network hop. A conventional MLX process must process usable prompt data on the host, where the host's owner can inspect or retain it. Multiple verification peers expand that exposure. A deletion promise or a valid computation result does not prove deletion, prevent a screenshot, or establish confidential execution.

Petals' security documentation warns about input recovery/modification on public peers and recommends a private trusted swarm for sensitive workloads. Apple's Secure Enclave is an isolated security subsystem; its existence on a Mac does not mean ordinary Metal inference runs inside it. Apple's Private Cloud Compute uses a purpose-built system with attested software, restricted access and transparency mechanisms. That evidence cannot be transferred to a household Mac running an ordinary application. [Petals security](https://github.com/bigscience-workshop/petals/wiki/Security,-privacy,-and-AI-safety), [Secure Enclave architecture](https://support.apple.com/guide/security/secure-enclave-sec59b0b31ff/web), [Private Cloud Compute](https://security.apple.com/blog/private-cloud-compute/).

**Inference:** confidential wallet history, medical records, corporate code and private documents are poor default workloads for strangers' Macs. A service limited to non-personal, non-confidential public data has a smaller addressable market; public availability alone does not remove PIPA obligations. A trusted supplier registry improves accountability while requiring supplier vetting and undermining the cheap permissionless-availability premise. Do not describe either case as equivalent to the owner's own private inference.

### A model response is untrusted input to a spending agent

Indirect prompt injection is a documented risk in LLM-integrated applications, including manipulation of downstream API behavior. An inference supplier can also return adversarial instructions or misleading “service” claims directly. This is a proposed trust-boundary risk, not an exploit reproduced against the current wallet. [Greshake et al., v2, 2023-05-05](https://arxiv.org/abs/2302.12173).

EastSea's existing protections matter: payments remain disabled until owner approval of a payee; the contract enforces recipient, per-payment/day limits and expiry. Its documentation explicitly says a tricked agent can still spend within those limits; Secure Enclave does not verify intent. The owner can allow any recipient, and revocation cannot reverse an already submitted transaction. [Agent wallet rules](../../agents/skills/aether-wallet/SKILL.md), [README agent policy](../../README.md).

**Inference:** approving an inference supplier does not make every charge it suggests legitimate. A provider response must not expand its own authority, authorize a new payee, change a session policy, or become the trusted price/metering source. Cap losses, bind a purchase to independently authorized terms, and keep results outside wallet-control instructions. Those are review requirements for any later proposal; this document does not claim the current payment tools lack an implemented check or authorize an integration.

### Model distribution adds a new supply-chain boundary

Downloaded model packages can include unsafe serialized artifacts or executable custom modeling code. Hugging Face recommends safetensors to avoid the arbitrary-code-execution risk of unsafe formats and separately warns that remote modeling code must be reviewed and revision-pinned. A safer tensor format does not by itself validate loaders, tokenizers, native libraries, resource use or model behavior. [Transformers security policy](https://github.com/huggingface/transformers/security/policy).

**Inference:** arbitrary buyer-selected model files, scripts, custom kernels, package installs or unrestricted tool execution are incompatible with a consumer wallet/node host. A fixed catalog reduces this risk but creates a curator's security, update, license and support duties. The minimum separation would keep an inference process away from wallet credentials, signing interfaces and node data, with limited filesystem/network access and resource budgets. Even then, memory/thermal/accelerator contention remains.

| Additional risk | Consequence that settlement cannot fix |
|---|---|
| Prompt/output retention, diagnostic logs and backups | Private data can survive a finished or refunded job. The host controls its own OS. |
| Wallet address, job metadata and billing correlation | Chain receipts reveal relationships/timing; publishing prompt text, outputs or sensitive activations makes exposure durable. A hash is not blanket anonymization. |
| Wrong or inflated model/token metering | A valid transfer can pay for a misdescribed service; tokenizer and billing terms need independent definition. |
| Malicious or merely oversized artifacts/requests | Can exhaust shared resources or bring parser/runtime risk into the wallet's host. |
| Abuse disputes and evidence retention | “No logs” conflicts with some support needs; collecting everything creates privacy obligations. Define a narrow lawful scope and accountable incident handling. |

Private local inference removes stranger-host exposure, but does not cure prompt injection, hallucination or malicious downloads. Its value is a smaller trust boundary, not immunity.

## 5. Korea: paid compute changes the facts, not every legal category at once

This is issue-spotting supported by current primary materials, not a formal legal opinion or an agency ruling on EastSea. The role of Pipln, each node seller, the wallet owner and the final AI application must be mapped separately.

### “Non-commercial / no business / no brokerage” is not a blanket exemption

Repeatedly accepting strangers' jobs for compensation can make the node owner a service seller; a platform choosing sellers, publishing prices, taking orders and handling disputes can be a service intermediary. That is this review's application of the facts, not a finding that every occasional Mac user is legally a business. A project charging no platform fee can still operate a continuing marketplace. “We only distribute software” must match what the official app, domain, defaults and support actually do.

Business registration, e-commerce status, AI-provider status and VASP status have different tests. The National Tax Service's general business-registration guidance and VAT Act Article 8 are relevant to recurring independent compute sales. Service compensation in crypto is not automatically tax-free: VAT Act Article 29 addresses non-money consideration. Model serving and tax treatment require their own review; delaying taxation of some crypto disposals does not exempt service income. [NTS guidance](https://www.nts.go.kr/nts/na/ntt/selectNttInfo.do?mi=2448&nttSn=1396), [VAT Act Article 8](https://www.law.go.kr/lsLinkCommonInfo.do?lsJoLnkSeq=1033805913), [Article 29](https://law.go.kr/lsLinkCommonInfo.do?lsJoLnkSeq=1031740189).

**Recommendation:** if the founder maintains a substantive no-business/no-brokerage boundary, an EastSea-operated paid marketplace fails that boundary. Independent third-party services may have different responsibilities, but a different label or a nominal third-party entity does not establish operational independence.

### Paying for compute is not automatically operating a VASP

Virtual Asset User Protection Act Article 2(2) covers specified virtual-asset activities performed as a business, including sale/exchange, certain transfers, custody and intermediation of sale/exchange. Ordinary payment for a compute service does not become virtual-asset exchange intermediation merely because the service price is paid in DBLN. Review together with the AML Act's definition. [Article 2, 2026-10-02 version](https://www.law.go.kr/LSW/lsSideInfoP.do?docCls=jo&joBrNo=00&joNo=0002&lsiSeq=290735&urlMode=lsScJoRltInfoR), [AML Act Article 2](https://law.go.kr/LSW/lsLinkCommonInfo.do?chrClsCd=010202&lsJoLnkSeq=1031812109).

Prepaid customer balances, pooled tokens, operator-controlled escrow/refunds, transfers for others, conversion into won/another token, or integrated token trading materially change that analysis. Platform fees evidence economic involvement but are not alone a VASP classification. The FSC's PayProtocol explanation concerns its actual affiliated coin buy/sell arrangement; it is not a prohibition on every merchant accepting crypto. [FSC, 2022-04-21](https://www.fsc.go.kr/no010101/77703).

**New evidence:** the final FIU/FSS August 2026 reporting manual is now retrievable, closing the source gap recorded in the [10-05 legal paths review](legal-lawful-paths-2026-10-05.md). Printed p.3 (PDF p.5) distinguishes mere technical services and some noncustodial wallets, lists key/control/signing factors, and says every factor need not be satisfied. It also warns that linked services' actual business model can change reportability. This is favorable to a narrow wallet, not an exemption for a future marketplace. [Official 103-page manual](https://www.fsc.go.kr/comm/getFile?srvcId=BBSTY1&upperNo=87521&fileTy=ATTACH&fileNo=5).

### Consumer protection survives noncustodial payment

For consumer online sales of compute services, identify seller and intermediary under the E-Commerce Act. Article 20 addresses notice that the intermediary is not the contracting seller, seller identity information and complaint/dispute handling; Article 20-2 addresses associated liability and an intermediary that also sells. “P2P” and “AS IS” do not replace those duties. Decide responsibility for failed/partial work, wrong models, double charging, cancellation and refunds before selling. [Article 20](https://www.law.go.kr/lsLinkCommonInfo.do?chrClsCd=010202&lsJoLnkSeq=1027063375), [Article 20-2](https://www.law.go.kr/LSW/lsSideInfoP.do?docCls=jo&joBrNo=02&joNo=0020&lsiSeq=282793&urlMode=lsScJoRltInfoR).

The legal research distinguished the **2026-07-21 current version** from search results containing **2027-01-21 future amendments**. Do not retroactively apply future provisions. B2B/private contracts can change the applicable consumer analysis, but still require an identifiable counterparty and enforceable service terms.

### Prompts create data-processing and overseas-transfer questions

Personal data can be processed temporarily in memory without persistent logs. PIPA Article 26 governs processing entrustment, including contractual safeguards, disclosure, supervision and re-entrustment; Article 28-8 governs overseas provision/access/entrustment/storage. Separate consent is not the only lawful route, but each route has conditions and required information. Randomly selected overseas home Macs make identifying recipients/countries and supervising processing difficult. User acceptance does not automatically cover personal data belonging to patients, staff or other third parties. [Article 26, 2026-09-11 version](https://www.law.go.kr/LSW/lsSideInfoP.do?docCls=jo&joBrNo=00&joNo=0026&lsiSeq=283839&urlMode=lsScJoRltInfoR), [Article 28-8](https://www.law.go.kr/LSW/lsSideInfoP.do?docCls=jo&joBrNo=08&joNo=0028&lsiSeq=283839&urlMode=lsScJoRltInfoR).

**Gate:** remove personal/sensitive-data workloads from any launch scope unless the actual processing roles, contracts, overseas-transfer basis, access controls and incident responsibilities can be fulfilled. “Encrypted transport,” “no logs” and a valid inference receipt are insufficient evidence.

### AI transparency and harmful outputs: duties depend on role and use

The AI Basic Act first commenced **2026-01-22**. Article 31 addresses notice/marking for generative or high-impact AI; Article 34 addresses high-impact risk management, explanation, protection, human oversight and records. General-purpose infrastructure is not automatically a high-impact application. [Article 31](https://www.law.go.kr/LSW/lsSideInfoP.do?docCls=jo&joBrNo=00&joNo=0031&lsiSeq=282791&urlMode=lsScJoRltInfoR), [Article 34](https://www.law.go.kr/LSW/lsLinkCommonInfo.do?lsJoLnkSeq=1031809457).

The official transparency guideline allocates duties to the business providing the final user-facing product/service, with an upstream model API versus downstream application example. It also does not automatically exempt free/open-source provision. Do not assign every hardware host the final application's duty, or assume nobody has it. Medical, hiring, lending and educational uses require specific high-impact analysis. [MSIT guideline, official NIA distribution, printed pp.2–3](https://www.nia.or.kr/common/board/Download.do?bcIdx=28987&cbIdx=99835&fileNo=14).

No inspected authority establishes a universal duty for every Mac operator to read/moderate every private inference output. Conversely, no inspected authority gives a blanket AI/neutral-infrastructure immunity. Criminal aiding liability depends on the actual offense, intent and contribution. Illegal-information rules and the Telecommunications Business Act's illegal-recording provisions depend on covered services, knowledge and public distribution; do not equate every private generation session with a public hosting platform. [Criminal Act Article 32](https://www.law.go.kr/lsLinkCommonInfo.do?lsJoLnkSeq=1005640169), [Network Act Article 44-7](https://www.law.go.kr/LSW/lsLinkCommonInfo.do?ancYnChk=&chrClsCd=010202&lsJoLnkSeq=1025057189), [Telecommunications Business Act Article 22-5](https://www.law.go.kr/LSW/lsLinkCommonInfo.do?chrClsCd=010202&lsJoLnkSeq=1033011555).

**Inference:** knowingly continuing a specific unlawful service, supplying tailored help for it, or distributing prohibited generated material differs from merely hosting a general model. An operator needs a workable response to actual notices and applicable orders. This need not mean collecting everyone's prompts; it does mean assigning someone responsibility and real powers to stop the covered service.

Model rights are a separate gate. For example, Llama 3.3's own license contains service/redistribution attribution and acceptable-use terms. An open-weight download or the runtime's software license is not unrestricted permission for every model/service. [Meta license, released 2024-12-06](https://huggingface.co/meta-llama/Llama-3.3-70B-Instruct/blob/main/LICENSE).

**Required legal evidence to reconsider:** a written Korean assessment of the concrete seller/contract/payment/control/data-flow map, not “is decentralized AI legal?” It must separately address business/tax, consumer duties, VASP-triggering money functions, PIPA, AI duties, model rights and abuse response. No EastSea-specific agency determination was obtained.

## 6. Product and operations: the Mac is already doing valuable work

MLX explicitly shares the CPU/GPU memory pool. EastSea already uses Metal for proving and has resource guards, so “the GPU is otherwise free” is an assumption to measure. Different processes do not isolate memory bandwidth, thermal headroom or accelerator queues. [MLX unified memory](https://ml-explore.github.io/mlx/build/html/usage/unified_memory.html), [Wallet resource controls](../../apps/wallet/Sources/ResourcesSettings.swift).

The repository records a **2026-09-29** proving incident on a 64-GB Mac: a 14-GB prover, 17.5/18.4-GB swap use and four co-located validators slowed to approximately 0.45 blocks/s. Those are historical operational observations, not an inference benchmark. Current budgets/watchdogs exist because co-tenancy was already painful. [Resource incident and guards](../ops/resource-limits.md).

Dimensional lower bounds, not measured footprints: 8B dense weights at 4 bits require **4 billion bytes (3.73 GiB)** before quantization metadata, KV cache, activations and runtime; 70B require **35 billion bytes (32.60 GiB)**. On a 16-GB Mac these compete with the node's documented 2-GiB automatic history-cache budget, 4-GiB prover allowance, the OS and the owner's apps. Budgets are allowances, not proof all that memory is allocated. Calling unused capacity “free” ignores opportunity cost and model loading.

Apple documents operating-temperature/ventilation constraints and warns that excessive heat can permanently reduce battery capacity. That does not prove inference will damage a particular Mac; sustained load introduces heat, fan noise, battery and user-experience costs that require measurement. Desktop Macs avoid battery wear, but still share power/memory/thermal resources. [Apple temperature guidance, 2026-04-02](https://support.apple.com/en-us/102336), [Apple battery guidance](https://www.apple.com/batteries/maximizing-performance/).

**Illustrative economics:** assume 40 output tokens/s, 1,000 output tokens/job, no prefill/load delay, 30 W incremental power and electricity at $0.20/kWh. That gives 25 seconds/job and **$0.0000417 electricity/job**. At the $0.000450 Groq example tariff, perfect sequential utilization yields only **$0.0648 gross/hour**, before checking, fees, wear, idle time or support. These inputs are scenarios, not measured Mac throughput/power or a Korean electricity tariff; batching can change throughput, while overheads change costs. Actual contribution margin must be measured.

A five-minute support interaction valued at an assumed $20/hour costs $1.67, equivalent to the **gross receipts of approximately 3,704 such jobs**. That is why cents saved on electricity do not settle the business case.

The support/product surface would include model/version compatibility, download integrity and disk space, sleep and power loss, residential connectivity/relays, interrupted streams, retries, cold starts, inaccurate metering, verification disagreements, refunds, privacy incidents and abuse notices. Existing P2P connectivity solves discovery/reachability; it does not guarantee inference deadlines or transfer large model artifacts cheaply.

**Inference:** a supplier scheduled only when consensus/proving/user activity permits it offers interruptible capacity. A buyer requiring reserved availability makes the supplier less idle. Provider earnings can also compete with voting/proving, concentrating reliable capacity in a few well-provisioned sellers. Both effects need measurement, rather than assuming inference strengthens the chain.

Mainnet launch and recovery checks already have documented responsibilities. A new inference service would add an independent reliability/security release matrix across chip, macOS, model and engine versions. No measured engineering estimate or support budget was established, so this review does not invent an “N weeks” schedule. Require an owner and funded operational capacity without displacing core launch/security work. [Mainnet launch requirements](../ops/mainnet-launch.md), [Roadmap](../design/13-roadmap.md).

## 7. Cheaper alternatives that retain the useful part

| Alternative | Value retained | Cost / limitation |
|---|---|---|
| **Keep the wallet model-agnostic** and use the existing owner-approved agent tools | Agents can pay and verify balances today; customers choose their own inference engine/provider. | No EastSea inference supply or earning story. Existing spending limits remain essential. |
| **Local-only model for the owner's own agent**, optional and off by default | Private document/wallet assistance; the same owner controls compute and keys. | Model quality, malicious artifacts and resource budgets still need review. Start with an already user-managed engine rather than bundling a catalog. |
| **Owner's iPhone → owner's Mac**, authenticated and explicitly chosen | Extends local model capacity while keeping a known host. | Networking/availability work; not a permissionless marketplace. Keep inference outside financial verification. |
| **Trusted private cluster / existing Exo deployment** outside the node | Large-model experimentation without strangers handling data. | Its owner manages hardware and service duties; LAN results still need relevant benchmarks. |
| **User-contracted API with their own credentials**, separate from wallet software | Low prices, stronger model options and an accountable supplier. | Supplier privacy terms and vendor dependence; no claim that EastSea verifies the answer. |
| **Deterministic rules/search for wallet questions** before an LLM | Balance/history explanations and bounded workflows without model hosting. | Less open-ended language capability; financial facts should still come from verified data. |

There is no reason to pay the chain to run the owner's own local model. Public marketplace revenue is not required to make the wallet useful to AI agents. “LATER” is not approval to bundle MLX or add an always-on background service now.

## 8. Top kill criteria and evidence that would change the verdict

These are **proposed decision gates**, not measured results, legal safe harbors, or guarantees from finite testing. A failed gate kills the corresponding marketplace scope; clearing them permits a new founder decision, not automatic adoption. Any later paid pilot requires its own legal clearance and authorization.

| Gate | Kill criterion | Evidence required to reopen |
|---|---|---|
| **K1 — actual buyer** | The only demonstrated users are project insiders, grants, free credits, emissions-funded demand or buyers already well served locally. | At least **five unrelated paying customers**, a defined lawful workload and **eight weeks of repeated unsubsidized use**; invoices/settlements, retention and reasons for preferring Mac supply. This threshold is a proposed commercial filter. Existing incumbent invoices/interviews can be gathered before any EastSea paid pilot. |
| **K2 — delivered economics** | Positive margin disappears when verification, utilization, retries, settlement, refunds and support are included; buyers do not renew at the necessary price. | Matched quality/model/latency measurements and positive supplier/platform contribution margins without token subsidy. For **commodity price substitution**, proposed hurdle: durable **≥50% all-in buyer saving** to overcome friction. For a **differentiated custom/model-access niche**, demonstrate retained willingness to pay for the specific advantage over available alternatives; no universal discount requirement. |
| **K3 — integrity contract** | Honest drift triggers penalties, the promised generation policy is not checked, or checker honesty/collusion assumptions cannot be justified. | Preregistered fleet/workload profiles; independent tests of each covered failure class and held-out prompts; separately measured false rejection/acceptance with confidence bounds; audit/incentive and dispute analysis. No automatic slashing based solely on a drifting equality/fingerprint test. |
| **K4 — privacy and wallet authority** | Prompts requiring confidentiality go to ordinary stranger-controlled hosts; responses can expand payment authority; deletion/processor claims cannot be supported. | An honest **non-personal, non-confidential data** scope or a separately validated confidential-execution design; actual processing contracts, data-flow checks, and independent wallet trust-boundary review. “Secure Enclave Mac” is insufficient. |
| **K5 — accountable lawful service** | No identified seller/refund operator; required data-processing duties or notices cannot be fulfilled; model hosting rights are absent; money functions or known abuse lack a lawful responsible operator. | Written Korean role-by-role legal assessment, model licenses, tax/billing policy, and a workable notice/incident/dispute process. Direct compute payment must be assessed separately from custody/exchange. |
| **K6 — node and owner first** | Inference causes missed consensus deadlines, disk/memory guard activation or unacceptable proving/owner latency, or safe preemption makes buyer latency untenable. | Relevant supported-Mac co-tenancy measurements, AC/battery/sleep behavior and enforceable budgets. Proposed performance hurdle: **≤5% p99 regression** in core latency plus **zero observed inference-caused stalls/guard events** in the scoped trial; report test duration and uncertainty, not a guarantee of no future stalls. |
| **K7 — opportunity cost** | Work displaces unresolved mainnet/security priorities or requires operating a business the founder still refuses to operate. | Core launch gates addressed, a funded accountable service owner, quantified maintenance burden, and an explicit new founder decision accepting the changed scope. |

K3 must specify error budgets before looking at results; this review does not invent a universal acceptable fraud rate for agents spending money. A model-identity classifier suitable for best-effort research may be unacceptable for financial penalties.

The most persuasive new evidence would be a **narrow Mac-specific niche**, independently reconciled payments, stable renewed demand, and a measured service meeting all seven gates. A successful LAN demo, a new verification paper, cheaper electricity, more node registrations or a rising token price alone would not change the verdict.

## 9. Assumptions checked, including the strongest defense

| Assumption | Review result |
|---|---|
| AI demand implies demand for EastSea home-Mac inference | **Unestablished.** Candidate buyers and relevant substitution baselines are identified in §2. |
| Decentralized inference has no paying market | **Too strong.** Chutes' offering and disclosures are credible counterevidence; independently reconciled home-Mac sales remain unknown. |
| Token emissions or chain transfers prove external sales | **False accounting inference.** §2 separates origin, subsidy and settlement. |
| Incumbents are expensive enough to leave ample margin | **Task-dependent and often false.** Current prices are fractions of a cent per small job. |
| Owned hardware / idle capacity has zero economic cost | **False.** Resource, availability, electricity and support costs remain. |
| More Macs make one inference request faster | **Not generally established.** Exo's historical single-request versus throughput distinction and LAN scope matter. |
| Seed + weight hash + greedy decode makes replay portable | **Unproven.** Kernel/software/profile and sampling contracts remain unspecified. |
| Bit-exact inference is fundamentally impossible | **Too strong.** Constrained deterministic profiles are possible research; no EastSea fleet guarantee was established. |
| Fast verification has no Mac evidence | **False.** VeriLLM includes M4s; its assumptions, denominators and environment must be retained. |
| Activation checking proves the agreed generation policy | **Not generally true.** TopLoc explicitly excludes sampling verification. |
| Checking time equals total checking cost | **False.** Include aggregate work, loading, settlement and disputes. |
| Multiple checkers mean independent honest checking | **Unestablished.** Ownership, incentives and role selection matter. |
| A valid inference proof means a correct financial answer | **False.** Faithful models can be wrong. |
| TLS / a Mac Secure Enclave makes remote prompts private | **False for ordinary host inference.** Different protections and execution domains. |
| A spending cap means prompt injection is harmless | **False.** It limits loss; it does not verify intent. |
| Safe tensor serialization solves arbitrary-model hosting | **False.** Executable code, loaders, dependencies, behavior and resource use remain. |
| Noncustodial/no-fee/open-source means no service duties | **Unsupported blanket exemption.** Legal roles have different tests. |
| Every crypto-paid compute supplier is a VASP | **Unsupported blanket classification.** Ordinary service payment differs from defined money activities. |
| Every Mac host must moderate every AI output | **No such universal duty established.** Scope, actual role, knowledge and distribution matter. |
| Existing node budgets automatically reserve capacity for inference | **Unestablished.** Co-tenancy needs explicit measurements and priority enforcement. |
| The marketplace is necessary for the agent wallet's promise | **False dependency.** Existing wallet tools and local/user-selected inference capture much of the value. |

## 10. Parallel lane and verification record

The other lane's `codex/mac-ai` branch was inspected read-only during research and final verification; it remained at `fc75af2c969af9884d997e60bcc2abf2f40e5489`, with no committed Mac-AI research or design 45. At **2026-10-09 12:12 KST**, its own worktree was also confirmed on that branch with no matching draft research or design 45. This review proceeded independently, as instructed. Any later design is a new proposal to check against these gates, not an adopted decision.

Source review used primary price pages, official Apple/MLX/Hugging Face documentation, versioned papers, current Korean law/government guidance, and transparent analytics methodology. Vendor financial/traffic claims were labeled and excluded from proven independent paid demand. Dynamic dashboards/authenticated revenue gaps were not filled with guessed numbers. Earlier AI-written repository research was context, not legal or numerical authority.

The API-job costs, paper latency percentages, memory lower bounds, power scenario and support scenario were recalculated locally. Remaining empirical gaps are fleet inference benchmarks, customer receipts, workload quality, production operating cost, privacy enforcement and an EastSea-specific legal assessment. No inference workload or attack was run. This review makes no claim that the unbuilt marketplace is secure, illegal, or already profitable.

**Final recommendation: do not adopt or implement the paid marketplace. Keep the existing agent wallet independent of inference supply. Consider owner-local inference only through a separate, later, bounded decision.**
