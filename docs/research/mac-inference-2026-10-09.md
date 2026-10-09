# AI inference on EastSea Macs: research (2026-10-09)

The founder asked on 2026-10-09: “이 블록체인 네트워크가 단순 스마트계약과 거래만 하는 네트워크면 자원낭비가 아닐까? ai model도 얹을 수 있나?” Yes: Apple Silicon can run useful inference alongside the network's existing Jolt/Metal block proving, provided inference yields to the node. The useful starting point is **one complete model on one available Mac**, with optional paid, sampled checking. It is not running an LLM in every validator's consensus execution.

This is research and design only. No code, builds, model downloads, benchmarks, payments, app launches, or remote-host operations were performed. The companion [design](../design/45-mac-inference.md) is a proposal for the **first upgrade after mainnet**, not a launch prerequisite or an implemented feature.

## Evidence and dates

Sources were accessed **2026-10-09**. Publication/revision dates below are distinct from access dates. **Documented** means a primary source describes the feature; **reported** means an author/operator supplied a measurement; **unverified** means this lane did not establish the claim independently or for EastSea. Undated live documentation can establish what its current page says, but not when a service launched. No current provider counts, market prices, uptime, or revenue are inferred from historical announcements.

## 1. Network precedents

These projects solve different problems. Training coordination, renting GPU capacity, scoring a miner's usefulness, and verifying a particular model execution are separate mechanisms.

| Project | Documented work and status | Verification and payment | Relevance and limits for EastSea |
| --- | --- | --- | --- |
| **Bittensor subnets** | Each subnet defines a task and scoring; Chutes SN64 documents an inference service. | Yuma aggregates stake-weighted validator evaluations and allocates emissions. This is utility scoring, not a proof of every customer's inference. Customer billing is separate. | Hardware/privacy vary by subnet. Chutes documents central API/validator endpoints and a TEE serving claim. Mac/MLX participation and attestation coverage are **unverified**. [B1–B4] |
| **Gensyn** | Current docs emphasize REE reproducible inference and AXL encrypted P2P. Older RL Swarm/BlockAssist/CodeAssist are marked archived; the home page says no official swarms are running. | REE receipt validation checks stored hashes; verification reruns inference. Reviewed docs establish no current paid inference escrow/bond marketplace. | Archived Apple Silicon instructions are CPU-only training, not a current Metal service. REE targets NVIDIA or CPU; MLX support **unverified**. Ordinary receipts contain plaintext prompts/output. [G1–G4] |
| **Ritual** | Current official chain docs identify a testnet and replace historical Infernet with native precompiles. | Delegated LLM/HTTP jobs run in registered TEE executors; attestations and RitualWallet prepaid fees are documented. Historical Infernet subscriptions are a different surface. | TEE-rooted execution is not zkML. Current mainnet settlement and Mac executors are **unverified**. Executor attestation/registry are trust dependencies. [R1–R3] |
| **Petals** | Distributed inference/fine-tuning by splitting layers among peers; current README includes Apple M1/M2 GPU servers. | The still-linked security wiki warns outputs are not guaranteed correct, suggests replication, and describes automated verification/reputation as future work. No native paid escrow/slashing established. | Demonstrates Apple participation. Peers can recover inputs/outputs and change results; private trusted swarms are recommended for sensitive work. Old wiki claims must retain their date. [P1–P3] |
| **Exo** | Local distributed inference, discovery, partitioning, and MLX. Current platform matrix lists Apple Silicon as maintained; Linux CUDA/CPU is planned. | Reviewed sources establish a runtime, not trustless verification/payment/slashing. | Strong Apple/MLX reference. LAN/Thunderbolt/RDMA success does not establish WAN economics or safety among untrusted owners. Owner-controlled clusters differ from public providers. [E1–E2] |
| **Akash** | Linux/Kubernetes container-capacity marketplace; tenants choose provider bids/leases. | Funded lease escrow pays providers continuously, currently documented in ACT with AKT fallback. This verifies settlement for capacity, not model outputs. | ARM64 Linux is not Apple Metal. Current provider GPU support is NVIDIA; confidential compute requires separately supported hardware. Console credit funding adds an optional centralized layer. [A1–A4] |
| **io.net** | Compute orchestration with a Mac worker guide, but that guide currently disables new-worker onboarding. Self-serve bare metal ended 2025-10-01. | Worker PoW, VRAM, stability and uptime eligibility tests are not per-inference proofs. Staking/rewards are separate from a customer's result verification. | Requires centralized device/account authorization and whole-device dedication, conflicting with validator/prover sharing. Current Mac hireability, native Metal serving and confidential prompts are **unverified**. [I1–I5] |
| **Hyperbolic** | Historical decentralized inference/marketplace; current docs describe on-demand/reserved/private GPU cloud. | Published PoSP/spML uses probabilistic checking and incentive assumptions. Current account-credit/capacity billing docs do not establish PoSP coverage of each customer request. | NVIDIA-centered current capacity. PoSP deployment coverage and a Mac provider path are **unverified**. Retention promises do not conceal prompts from an executor. [H1–H5] |
| **Prime Intellect** | Distributed training and inference/data-generation experiments; hosted Prime Inference launched **2026-10-02**. | TOPLOC was used in distributed rollout experiments with verifier servers. Current hosting offers company APIs/billing; per-customer TOPLOC proofs or on-chain job escrow are **unverified**. | Published experiments/current hosting target NVIDIA. SYNTHETIC-2 used central orchestration. Distributed machines do not by themselves imply decentralized governance or prompt secrecy. [Q1–Q4] |
| **Nous Psyche** | Cooperative distributed training with compressed updates; May 2025 architecture places broad inference later. Later releases establish training/post-training use. | Architecture discusses Solana coordination and redundant comparisons but explicitly leaves verification difficulties open. Phase-0 terms describe screened testing and valueless tokens. | Current joining guides require modern Linux/NVIDIA CUDA and administered runs. Mac serving and a paid consumer inference mainnet are **unverified**; old testnet terms do not prove no later deployment exists. [N1–N5] |

Prime's **2025-07-10** SYNTHETIC-2 report claims median verification was 25× cheaper than inference and a 0.000925% false-positive rate, including 37 slashes. These are useful operational observations for that run, not cryptographic guarantees, Mac measurements, or a justification for penalizing every numerical mismatch. [Q2]

**Design inference:** adopt peer discovery, artifact binding and escrow ideas; do not import a hosted API, CUDA-only provider stack, founder-controlled scheduler, or emissions for self-created jobs. Exo/Petals are references for distributing computation, not ready-made EastSea settlement protocols.

## 2. What verification actually buys

| Approach | Assurance | Cost and unresolved assumptions |
| --- | --- | --- |
| **zkML** | A sound proof can bind a precisely encoded computation/model to committed inputs/outputs. | Circuit semantics, quantization and model binding must match what was promised. Proving is expensive; the computing machine still sees its inputs unless a separate confidential-computation technique is used. |
| **Optimistic re-execution / fraud proofs** | A deterministic trace can be disputed and a final instruction adjudicated on chain. | Requires an honest challenger, challenge period, data availability, and a deterministic reference VM. Native MLX hash disagreement is not a fraud proof. |
| **Spot checks / PoSP** | Repeated checking can deter economically motivated cheating under stated assumptions. | Sampling misses individual bad jobs; accurate arbitration, unpredictable selection, bounded gains and sufficient penalties matter. |
| **TOPLOC** | Compact activation fingerprints can detect certain execution changes cheaply despite numerical differences. | Empirical tolerance/detection, not a SNARK or fuzzy final-text proof; decoding/sampling and some attacks remain unresolved. |
| **Redundancy + reputation** | Independent replicas and historical error rates improve confidence. | Replicas may collude or share an owner; historical reliability is not permanent honesty. Honest numerical divergence can look like fraud. |
| **TEE** | Attested execution may protect integrity/confidentiality within a supported hardware/software boundary. | Trust moves to hardware, firmware, attestation, code and side-channel assumptions. Ordinary Mac key protection supplies no general MLX enclave. |

### zkML: original costs rather than LLM extrapolations

[zkLLM, 2024-04-24, table 1/§8](https://arxiv.org/html/2404.16109v1#S8) reports the following on an **A100 SXM4 40 GB**, 12 EPYC 7413 CPU cores and 124.5 GB allocated host memory. The workload uses C4 sequences of length 2,048 and fixed-point scaling 2^16; it is not an autoregressive chat tokens/s benchmark.

| Model | Prove | Reported memory | Proof | Verify | One-time model commitment |
| --- | ---: | ---: | ---: | ---: | ---: |
| OPT-125M | 73.9 s | 1.88 GB | 141 kB | 0.342 s | 11.8 s |
| Llama-2-7B | 620 s | 15.5 GB | 183 kB | 2.36 s | 531 s |
| Llama-2-13B | 803 s | 23.1 GB | 188 kB | 3.95 s | 986 s |

The [official CUDA implementation](https://github.com/jvhs0706/zkllm-ccs2024) also distinguishes demo functionality from batching used for component measurements. **Unverified:** Metal portability, 8B/70B proof cost, conversation-level latency, and required Mac memory. A small proof does not imply a cheap prover.

[EZKL's 2024-01-28 benchmark](https://blog.ezkl.xyz/post/benchmarks/) measures non-neural workloads: linear-regression proving averages 0.118 s/19.375 MB, random-forest classification 6.161 s/382.782 MB. Successful-run hardware and proof sizes are unspecified; its 1,000 GB machine refers to a failed alternative experiment. These are not LLM cost measurements.

[EZKL's 2025-01-20 Metal report](https://blog.ezkl.xyz/post/metalbindings/) shows approximately 2× MSM acceleration near 2^20 points on Apple hardware, but only 9% overall proving improvement. It supplies no end-to-end 8B/70B cost. Existing Jolt block proofs prove chain execution; they do not prove an arbitrary off-chain MLX trace.

### Optimistic ML and PoSP

[opML, submitted 2024-01-31, revised 2024-02-05](https://arxiv.org/html/2401.17555v2), uses deterministic fixed-point/software-floating-point execution and interactive trace bisection ending in a contract-verifiable instruction. At least one honest challenger and a dispute window remain assumptions. EastSea does not currently have this arbitration machinery for Metal kernels.

[PoSP, submitted 2024-05-01, v3 revised 2025-05-31](https://arxiv.org/html/2405.00295v3), gives an incentive-equilibrium argument under deterministic computation, accurate arbitration, bounded malicious participation and suitable rewards/costs/bonds. Its spML disagreement endpoint computes the function on chain or verifies a ZKP; orchestration also has a Byzantine-fault bound. Replacing that endpoint with a Mac majority replaces an assumption and cannot inherit the theorem unchanged. Hyperbolic's [2024-12-27 implementation description](https://hyperbolic.xyz/blog/deep-dive-into-hyperbolic-proof-of-sampling) describes reputation/stake-dependent sampling, trusted validators and arbitration. It does not independently establish universal honesty or present customer coverage.

### TOPLOC, replication and commitments

[TOPLOC, submitted 2025-01-27, v2 revised 2025-05-30](https://arxiv.org/html/2501.16007v2), fingerprints top-k hidden-activation indices/values using polynomial congruences. A checker recomputes activations against the submitted token sequence and compares numerical tolerances. Reported storage is 258 bytes per 32 generated tokens; evaluated stacks include A100/RTX 4090 and different attention kernels. MLX/Metal portability is **unverified**. The paper's limitations include decoding/sampling, activation spoofing, unstable inputs, small model/input changes and precision/cache discrimination. Its related-work cost extrapolation conflicts with zkLLM's original table; the costs above use the original source.

An exact SHA-256 output commitment proves binding to bytes, not correct execution. A model content hash proves artifact identity, not that a provider used it. A fresh job nonce stops receipt replay, but cannot stop collaborators from sharing a valid result and recomputing its hash.

[BOINC's HICSS 2009 paper](https://boinc.berkeley.edu/boinc_papers/hicss_08/hicss_08.pdf) documents replication, further reruns on disagreement and adaptive checking based on error history. It explicitly treats adaptive replication as error reduction, not elimination. Application-specific homogeneous comparison is a useful precedent for execution classes; financial slashing adds a stricter false-positive requirement.

**Design inference:** first use blinded result commitments, same-profile reruns and explicitly acknowledged committee settlement. Slash objective protocol faults separately. A conflicting hash is a reason to investigate, not an independently verifiable proof of dishonesty.

## 3. MLX/Metal determinism

Official [MLX numerical precision](https://ml-explore.github.io/mlx/build/html/usage/precision.html) and [environment-variable](https://ml-explore.github.io/mlx/build/html/usage/environment_variables.html) documentation, undated/accessed 2026-10-09, says matmul precision/kernel choices depend on backend/hardware; documented `MLX_ENABLE_TF32` defaults to 1, while 0 requests full float32. Neither supplies a universal cross-generation output guarantee.

Dated upstream reports make the practical issue concrete:

- [mlx-lm #1280, 2026-05-17](https://github.com/ml-explore/mlx-lm/issues/1280): the same model/prompt, temperature 0 and seed produced different completions/token counts on M3 Ultra and M5 Max, while answer checks still passed.
- [MLX #3568, 2026-05-20](https://github.com/ml-explore/mlx/issues/3568): a float32 random-normal primitive differed on M1 Max versus M3 Ultra/M5. This is not proof about every LLM decode path.
- [MLX #3897, 2026-07-23](https://github.com/ml-explore/mlx/issues/3897): batched versus single-sequence attention differed numerically on the reported M5 setup; the displayed argmax still matched.
- [Thinking Machines, 2025-09-10](https://thinkingmachines.ai/blog/defeating-nondeterminism-in-llm-inference/): fixed-execution repeatability and invariance across batch shapes differ; its demonstrated solution targets its CUDA stack, not MLX.

These are primary author reports, **unverified on EastSea**. No complete M1/M2/M3/M4/M5 reproducibility matrix was established. Neither equal RAM nor a common chip family is sufficient. Pin exact model/tensors, tokenizer/template, quantization, OS/Metal, runtime/build, precision/kernel flags, batch and prefill shape, KV policy and decoding. Greedy decoding fixes selection policy, not floating-point reductions or near-tie logits. Certification on a corpus remains empirical, not a proof for every input.

## 4. Throughput and memory

The numbers below are **reported decode tokens/s**, not EastSea measurements or promises. RAM controls fit; chip/GPU bandwidth, kernel, quantization, context and contention control speed. Different rows are not a controlled experiment in increasing RAM. oMLX entries are operator-submitted original runs; hardware/OS fields are self-reported.

| RAM as reported | Chip / GPU | Actual model / quantization | Prompt/context; output where given | Decode tokens/s | Runtime and dated primary source |
| --- | --- | --- | --- | ---: | --- |
| 8 GB | M2 mini; GPU count unstated | Llama-3-8B-Instruct, MLX 4bit | Short story; max 256 output | 18.5 | Versions unstated; [Awni, 2024-04-19](https://github.com/ml-explore/mlx/discussions/1013) |
| 16 GB | M1 / 8 GPU | Llama-3.1-8B-Instruct, 4bit | 1,024 / 4,096 | 13.8 / 12.4 | oMLX 0.2.21, macOS 26.3.1; [2026-03-26](https://omlx.ai/benchmarks/performance/k9d0mqe5) |
| 16 GB | M4 / 10 GPU | Llama-3.1-8B-Instruct, 4bit | 1,024 / 4,096 | 21.8 / 19.6 | oMLX 0.2.13, macOS 26.3.1; [2026-03-16](https://omlx.ai/benchmarks/performance/kxx1qy0p) |
| 24 GB | M4 Pro / 16 GPU | **Hermes-3-Llama-3.1-8B**, 4bit | 1,024 | 51.9 | oMLX 0.3.6, macOS 26.5 as reported; [2026-04-17](https://omlx.ai/benchmarks/performance/wcm4t3h0) |
| 32 GB | M1 Max / 24 GPU | Llama-3.1-8B-Instruct, 4bit | 1,024 / 4,096 | 61.8 / 51.7 | oMLX 0.3.5.dev1, macOS 15.7.4; [2026-04-08](https://omlx.ai/benchmarks/performance/v2vgqvd5) |
| 48 GB | M4 Max / 40 GPU | Llama-3.1-8B-Instruct, **8bit** | 1,024 / 4,096 | 56.3 / 52.0 | oMLX 0.2.24, macOS 26.3.1; [2026-03-30](https://omlx.ai/benchmarks/performance/x9xt4415) |
| 64 GB | M3 Max; GPU count unstated | Llama-3.3-70B-Instruct, MLX 4bit | 260 / 8,015 / 32,172; output 309 / 786 / 27 | 9.351 / 8.520 / 6.482 | MLX 0.21.1, mlx-lm 0.20.4; [chigkim, 2024-12-15](https://github.com/ml-explore/mlx-examples/issues/1029#issuecomment-2543634789) |
| 96 GB | M3 Ultra / 60 GPU | **Hermes-4-70B**, 4bit | 1,024 / 4,096 | 15.9 / 14.6 | oMLX 0.2.20, macOS 26.3.2; [2026-03-25](https://omlx.ai/benchmarks/performance/yy1h465w) |
| 128 GB | M4 Max / 40 GPU | **Llama-3.3-70B-Instruct-abliterated**, 4bit | 1,024 / 4,096 | 11.1 / 9.6 | oMLX 0.2.6, macOS 26.3.1; [2026-03-10](https://omlx.ai/benchmarks/performance/83kbldmy) |

The 48 GB example is Q8, not Q4; fine-tuned/modified checkpoints remain explicitly named. There is no version/context/artifact-matched 8B/70B table covering the whole memory range. Time to first token, prompt processing, loading, queueing, thermal limits and checking add costs absent from decode-only figures. **Unverified:** same rates while proving blocks, measured energy/job, or a sustainable market price.

### Capacity is a separate calculation

1 GB = 10^9 bytes; 1 GiB = 2^30 bytes. `parameters × weight_bits / 8` gives only a lower bound: nominal 8B Q4 is 4 GB/3.73 GiB; 70B Q4 is 35 GB/32.60 GiB. Quantization metadata, higher-precision tensors and workspaces add memory.

Public converted artifact metadata, undated/accessed 2026-10-09, supplies a more concrete example; no tensor files were downloaded:

- [MLX Llama-3.1-8B Q4 tensor index](https://huggingface.co/mlx-community/Meta-Llama-3.1-8B-Instruct-4bit/blob/main/model.safetensors.index.json): 4,517,404,672 bytes = **4.207 GiB**. Its [config](https://huggingface.co/mlx-community/Meta-Llama-3.1-8B-Instruct-4bit/blob/main/config.json) has 32 layers, 8 KV heads and head dimension 128.
- [MLX Llama-3.1-70B Q4 tensor index](https://huggingface.co/mlx-community/Meta-Llama-3.1-70B-Instruct-4bit/blob/main/model.safetensors.index.json): 39,688,355,840 bytes = **36.963 GiB**. Its [config](https://huggingface.co/mlx-community/Meta-Llama-3.1-70B-Instruct-4bit/blob/main/config.json) has 80 layers, 8 KV heads and head dimension 128.

For full retained bf16/fp16 GQA cache, our calculation is `2 × layers × KV_heads × head_dimension × 2 bytes × retained_tokens × batch`. These example models need **128 KiB/token (8B)** or **320 KiB/token (70B)** per sequence: at 4K tokens, **0.5 / 1.25 GiB**; at 32K, **4 / 10 GiB**. These are architecture-based calculations, not measured process peaks. Cache quantization/rotation needs its own execution profile.

| Memory tier | Standalone fit | Conservative EastSea policy proposed |
| --- | --- | --- |
| 8 GB | Short 8B Q4 demonstrated | Smaller local models; exclude initial 8B paid-provider class with node/prover reserves |
| 16 GB | Short 8B Q4 fits | 8B only if measured loading/peak/KV fits every reserve; small context, concurrency 1 |
| 24 / 32 GB | 8B fits; 70B artifact does not | Initial 8B candidates after certification |
| 48 GB | 8B fits; 70B can be tight standalone | Initial 8B only |
| 64 GB | Standalone 70B Q4 reported | Default reject 70B coexistence with proving; standalone speed does not establish node safety |
| 96 / 128 GB | 70B Q4 candidates | Later 70B class after full memory/preemption tests |
| 192 / 256 / 512 GB+ | More capacity, not an automatic speed increase | Larger profiles remain separately budgeted; bf16 70B alone starts at 140 GB/130.39 GiB |

Repository [resource limits](../ops/resource-limits.md) reserve auto prover memory at 25% RAM (minimum 4 GiB), and bound history cache at 1–4 GiB. As an **illustrative** 64 GiB admission calculation, reserving 16 GiB for proving, 4 GiB history and 8 GiB OS/user headroom leaves 36 GiB, already below the example 70B tensors before KV/workspace. Cache budget is not the whole node footprint; admission must use measured peaks and additional margin.

## 5. Prompt privacy and Apple

Apple's [Secure Enclave key API](https://developer.apple.com/documentation/security/protecting-keys-with-the-secure-enclave) and [platform-security description](https://support.apple.com/guide/security/the-secure-enclave-sec59b0b31ff/web), undated/accessed 2026-10-09, document key protection and specialized secure processing. **Inference from the public API boundary:** no documented general third-party MLX/Metal application TEE or remote attestation of an arbitrary Mac's GPU inference was established. A Secure Enclave signature protects a key; it does not prove model execution or hide unified-memory prompts from the host owner. DeviceCheck also does not attest an MLX trace.

Apple [PCC introduction, 2024-06-10](https://security.apple.com/blog/private-cloud-compute/), combines custom server hardware, hardened restricted software, measured/attested code, transparency, ephemeral data and exclusion of privileged runtime access. The [2024-10-24 research release](https://security.apple.com/blog/pcc-security-research/) supplies research tooling; its virtual environment is not a production confidential-Mac provider entitlement.

The [2026-06-08 PCC expansion](https://security.apple.com/blog/expanding-pcc/) announces Google Cloud/NVIDIA GPUs with NVIDIA confidential computing, Intel TDX and Google's Titan while retaining the same privacy requirements. That article describes a staged preview; rollout completion is **unverified** here. PCC should not be described as exclusively Apple Silicon today. Its end-to-end trust boundary is a reference to study, not a property EastSea inherits.

Local-only jobs keep inputs/output on the requester's machine. Network reruns require provider and checker access to the prompt; encrypted transport protects the route, not against those owners. Layer activations are not a confidentiality substitute. No logging/deletion policies can stop a dishonest provider retaining plaintext.

Keep prompts, output bodies and commitment salts off chain. Unsalted low-entropy prompt hashes allow guessing; use fresh high-entropy salts. Model IDs, payments, identities, timing and audit activity still expose metadata. An on-chain public fraud trace could reveal a private prompt: the first design therefore uses off-chain evidence and explicitly accepted committee trust, not public prompt disclosure.

## 6. Consequences and remaining research

1. Opt-in local inference first; a paid network should require demonstrated demand and enough compatible independent checkers.
2. Start with bounded 8B whole-model jobs. WAN model sharding, distributed training, 70B, zkML and TOPLOC remain later experiments.
3. Freeze execution profiles and measure false mismatches before enabling correctness penalties. A checked result is statistical/committee assurance, not cryptographic proof or semantic truth.
4. Budget cold loading, KV, workspaces, allocator caches and **idle prover reserve**. CPU niceness and a two-second watchdog do not prove safe GPU preemption.
5. Establish fresh post-commit randomness and enforceable evidence availability before paid sampling. Existing future-epoch randomness can already be known.
6. Measure total job and checking cost, latency, energy, liveness and prompt-retention boundaries after mainnet. No such gates passed in this documentation lane.

## Primary source register for the network table

All **undated** entries were accessed **2026-10-09**. Dates in article titles below are publication dates unless stated otherwise.

| IDs | Primary sources and dates |
| --- | --- |
| B1–B4 | [Bittensor subnet guide](https://guides.learnbittensor.org/subnets/understanding-subnets), [Yuma consensus](https://www.bittensor.com/docs/internals/consensus), [Chutes miner overview](https://chutes.ai/docs/miner-resources/overview), [Chutes starter guide](https://chutes.ai/docs/guides/starter-guide): undated |
| G1–G4 | [Gensyn current docs](https://docs.gensyn.ai/), [archived RL Swarm](https://docs.gensyn.ai/testnet/rl-swarm/getting-started), [REE prerequisites](https://docs.gensyn.ai/tech/ree/get-started), [REE receipts](https://docs.gensyn.ai/tech/ree/receipts): undated; current archival notice takes precedence |
| R1–R3 | [Current Ritual Chain docs](https://docs.ritualfoundation.org/), [historical Infernet client](https://infernet-client.docs.ritual.net/), [Ritual website](https://www.ritual.net/): undated; mainnet/TEE coverage not independently tested |
| P1–P3 | [Petals README](https://github.com/bigscience-workshop/petals): undated current revision; [security wiki](https://github.com/bigscience-workshop/petals/wiki/Security,-privacy,-and-AI-safety): visible edit **2022-12-09**; [Petals site](https://petals.dev/): undated |
| E1–E2 | [Exo README](https://github.com/exo-explore/exo), [platform matrix](https://github.com/exo-explore/exo/blob/main/PLATFORMS.md): undated current revisions |
| A1–A4 | [Akash concepts](https://akash.network/docs/getting-started/core-concepts/), [provider overview](https://akash.network/use-cases/providers/), [hardware](https://akash.network/docs/providers/getting-started/hardware-requirements/), [confidential hardware](https://akash.network/docs/providers/operations/confidential-compute-hardware/): undated |
| I1–I5 | [io.net Mac onboarding](https://support.io.net/en/support/solutions/articles/156000014581-macos-how-to-add-a-new-worker), [cluster test](https://support.io.net/en/support/solutions/articles/156000220131-cluster-verification-test), [reward eligibility](https://support.io.net/en/support/solutions/articles/156000101702-block-rewards-nomination-eligibility-status), [device dedication](https://support.io.net/en/support/solutions/articles/156000093905-io-worker-automatically-deleting-docker-containers): undated; [bare-metal retirement](https://docs.io.net/docs/deploy-bare-metal-cluster): change effective **2025-10-01**, page undated |
| H1–H5 | [PoSP announcement](https://hyperbolic.xyz/blog/hyperbolic-introduces-novel-practical-solution-to-the-problem-of-verification-in-decentralized-ai): **2024-05-09** (indexed primary text; later direct fetch failed); [current overview](https://www.hyperbolic.ai/docs/overview/overview), [billing](https://www.hyperbolic.ai/docs/general/billing-payments), [instance tests](https://www.hyperbolic.ai/docs/on-demand/verifying-performance), [historical privacy FAQ](https://hyperbolic.xyz/privacy/faq): undated; current FAQ applicability unverified |
| Q1–Q4 | [TOPLOC announcement](https://www.primeintellect.ai/blog/toploc): **2025-01-28**; [SYNTHETIC-2](https://www.primeintellect.ai/blog/synthetic-2-release): **2025-07-10**; [INTELLECT-2](https://www.primeintellect.ai/blog/intellect-2-release): **2025-05-11**; [Prime Inference](https://www.primeintellect.ai/blog/prime-inference): **2026-10-02** |
| N1–N5 | [Psyche architecture](https://nousresearch.com/nous-psyche): **2025-05**, day unspecified; [joining](https://docs.psyche.network/enduser/join-run.html), [provider guide](https://docs.psyche.network/enduser/quickstart-compute-provider.html), [Phase-0 terms](https://psyche.network/legal): undated; [future directions](https://forum.nousresearch.com/t/psyche-future-directions/273): **2025-10-09**; [Nous release index](https://nousresearch.com/releases) lists **2025-12-03** Psyche post-training release |
