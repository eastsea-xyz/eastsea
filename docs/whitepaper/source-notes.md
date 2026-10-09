# Source notes for the critique draft

Recorded 2026-10-09. This file preserves the founder's instructions where no
independent repository measurement or protocol artifact establishes them. It
is provenance, not additional verification.

The paper describes checkout `345c36a4999cb3cb9a28e5f23947ef4c2a040cf8` on
`codex/whitepaper`, derived from `lead-merge`. Cross-branch evidence is pinned
to these commits, read without checking out another lane:

| Source | Revision |
|---|---|
| `codex/committee-scale`, requested measurement revision | `9e033c8263f43d5cec01bbfad5ba6815974bd620` |
| `codex/storage-defaults` | `c5a0d018b0d120dd74ca9cb285a28f1b8405bf61` |
| `codex/storage-reward-v2` | `2c519e6bdd1aee09a919a1baf59f65a291c982c4` |

## Founder-supplied observations and decisions

Source: the founder's whitepaper task brief, received 2026-10-09.

- The experiment is: “Can ordinary Macs run and prove a public network with
  no owner?” The paper should invite participation and critique, disclose
  compromises, and say that participants should not expect money.
- The requested launch committee ceiling is 16, following the committee-scale
  recommendation. This documentation task does not change a genesis, node
  rule, or launch checklist.
- Registrar and update keys are designs of necessity, with **no forced
  sunset**. Do not promise their removal. Any discussion of alternatives in
  the paper is a research condition, not a removal commitment.
- The brief reports a **0.7.1 observation of approximately 400% CPU and at most
  5% GPU utilisation**. It does not supply the Mac model, workload, sampling
  tool or interval, raw samples, or observation date. The reporting date is
  2026-10-09, not an established measurement date. No matching utilisation
  record was located in the reviewed repository sources. The paper records
  this as an attributed operational observation, separately from reproducible
  timings. It does not infer the fraction of proving work executed on the GPU.
- First community contact is a request for critique after the toolbox
  contracts pass on mainnet. Only the founder sends or posts it. This lane
  writes drafts; it does not certify that gate or publish anything.

The checkout has no `docs/legal/` directory. Legal wording was checked against
`DISCLAIMER.md`, `docs/research/legal-review-2026.md`, its verification memo,
and `docs/ops/legal-open-questions.md`. Their legal conclusions are not adopted
as an external legal opinion or regulatory approval.
