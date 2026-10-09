# Critique drafts

[English paper](eastsea.md) is the primary text; [Korean](eastsea.ko.md) is its
faithful translation. Both are dated drafts, not evidence of a mainnet launch.
The [dated critique](critique-2026-10-09.md) ranks the hardest questions in five
reader voices and records the number ledger, text corrections and missing evidence.
[Source notes](source-notes.md) pin the reviewed checkout and other lanes,
and distinguish the founder's reported CPU/GPU observation from a reproducible
benchmark. The paper's 128-word abstract (including citation labels) and numbered sections use plain
technical prose. Markdown has no intrinsic page count. For the initial
2026-10-09 draft, a temporary local pagination proof fitted the English text,
tables, and references into **11 A4 pages**, using 15 mm margins,
approximately 10.5-point Times body text, and
8.6-point table text. That checked the original requested 10–14-page length;
the critique revisions have not been repaginated. Another renderer or
typography can paginate differently. The original proof is not committed.

Use [announce-plain.txt](announce-plain.txt), also copied as the requested
[announce-draft.txt](announce-draft.txt), for the Cryptography Mailing List
and similar moderated technical lists. It opens personally, identifies the
technical question, qualifies the measurement, and asks for specific critique.
The list describes itself as low-noise and moderated, with cryptographic
technology on topic. A calmer subject and opening fit that stated policy;
that is an editorial judgment, not a claim that its page explicitly bans
all hooks. [List guidance](https://www.metzdowd.com/mailman/listinfo/cryptography).
The craft reference is the short personal opening and direct paper link in
[Satoshi's 2008 message](https://www.metzdowd.com/pipermail/cryptography/2008-October/014810.html),
not a claim that EastSea has the same properties.

Use [announce-hook.txt](announce-hook.txt) for the founder's own email and
broader channels such as Hacker News, Nostr, X/Bluesky, or Korean developer
communities, adapting length and language to the channel. It gives five
subjects and three openings. Its opening uses a specific recorded timing,
then leaves the reader with the unresolved finality/proof trade. Present voting
and founder-reserve powers appear beside the ownership question. Simulation and
workload qualifications stay next to the facts. The core sendable drafts are
under 250 words; the hook alternatives are outside that body. Every version
invites node runs, assumption challenges, code/paper review, and hardware
measurements, and says not to join expecting money.

The founder sends or posts. These files authorize no agent publication.
The founder's gate is **after the toolbox contracts pass on mainnet**. This
lane has not performed or certified that gate. Before sending, refresh the
network/release status and paper against the verified artifacts; keep the
dated measurement conditions and remaining limitations. Existing executor
fixture tests in the [contract report](../research/contracts-onchain-2026-10-06.md)
are not a mainnet pass or an independent audit.

To join locally, follow the [README build prerequisites](../../README.md#try-it),
and give the demo and stop commands the **same disposable data directory**:

```bash
export AETHER_DEVNET_DIR="$(git rev-parse --show-toplevel)/tmp/eastsea-demo"
scripts/demo.sh
scripts/devnet.sh stop
```

Run these commands from the repository root. The demo starts four local
validators and resets the selected directory; use it only for disposable
demo data. The export matters: [demo.sh](../../scripts/demo.sh) and
[devnet.sh](../../scripts/devnet.sh) otherwise default to different directories,
so the unqualified stop command in the baseline README does not stop the
demo nodes. The README's “Run a real network” section describes creating an
independent network and key ceremony, not joining an existing public network.
Running the demo alone does not register a Mac or grant a public voting seat.
Report non-sensitive experimental
results to the [repository issue tracker](https://github.com/eastsea-xyz/eastsea/issues),
including revision, Mac/RAM, OS, workload, network conditions, timing method,
raw samples, and failures. No private security-report address or SECURITY.md
was present in the reviewed checkout, so the drafts do not invent one.
