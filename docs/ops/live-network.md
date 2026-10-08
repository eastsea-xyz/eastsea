# Live network observations (privacy schema 2)

`aether_presence` returns a privacy-filtered **unverified cohort observation**,
independent of consensus, validator votes and candidate beacons. It makes no
chain writes and uses no central counting service. The explorer checks it every
10 seconds; the producer releases counts once per fixed 10-minute window.

This is not a count of distinct people, physical Macs or every node in the
network. Locally it counts this endpoint and recently observed authenticated
transport peers. A forwarded aggregate may describe an overlapping or invented
cohort. The node forwards one complete largest safe observation and never adds
anonymous observations together. Partitions, old peers and the sampling window
can leave counts withheld or incomplete. Presence is unsuitable for consensus,
rewards or eligibility.

## Public RPC

`aether_presence` takes an empty parameter array and supports the `eastsea_`
alias. The public read gateway permits this read. Schema 2 has exactly these
fields:

```json
{"schema":2,"available":true,"scope":"unverified cohort observation",
 "observed_at":1791440400,"ttl_seconds":600,"minimum_bucket_size":3,
 "total":6,"by_role":{"other":6},"by_region":{"unknown":6},
 "by_version":{"unknown":6}}
```

`observed_at` is Unix seconds rounded down to a 600-second boundary. The full
snapshot remains fixed within that window, including when peers connect,
disconnect or the local country setting changes. The next window can contain a
new observation. Rounding a timestamp without freezing the counts would still
expose the exact moment of a change.

There are no individual `nodes`, observer identities, country codes, signatures,
exact activity times, IP addresses, relay URLs, cities or coordinates. Per-peer
version/quality claims were removed; the compatibility `by_version` field is
one coarse `unknown` cohort, rather than a rare build histogram.

Every published count is at least **k=3**, including `total`. Role and region
maps are disjoint partitions whose sums equal the published total. Small role
buckets fold into `other`; small continent buckets fold into `world`. If their
combined remainder is only one or two, a whole larger sibling is also folded.
For example, `validator:3,follower:1,total:4` becomes `other:4,total:4` instead of
publishing `validator:3` with a total that reveals the hidden singleton. Zero
buckets are omitted; an omitted label must not be shown as a zero.

A cohort smaller than three has `total:null` and empty breakdown maps. Clients
show **Count withheld**, not zero. An endpoint without presence support uses
`available:false`, `total:null` and empty maps; clients show unavailable.
Schema 1 is rejected because its identifying raw records do not satisfy this
boundary. Thresholding is a minimization measure, not an anonymity or legal
compliance guarantee: auxiliary knowledge and changes across release windows
can still reveal information.

## Aggregate gossip and limits

The wire protocol uses `aether/presence/2`. Schema 1 is no longer served, so old
raw presence records cannot enter or leave the aggregate path. Nodes exchange
this envelope on startup and approximately every 60 seconds:

```json
{"schema":2,"network":"7777:0","aggregate":{
 "schema":2,"available":true,"scope":"unverified cohort observation",
 "observed_at":1791440400,"ttl_seconds":600,"minimum_bucket_size":3,
 "total":6,"by_role":{"other":6},"by_region":{"unknown":6},
 "by_version":{"unknown":6}}}
```

The aggregate has the same producer filtering and frozen release as public
RPC. It carries no sender field, identity binding, individual role, country,
build version or signed per-node heartbeat. QUIC necessarily exposes its
authenticated immediate transport peer to the other endpoint; that identity
stays in the receiver's bounded local table and is not forwarded in a payload.
Authentication does not verify the remote aggregate's counts or uniqueness.

Received messages must match the network and current 10-minute bucket. Unknown
fields, identifying schema-1 records, sub-k buckets, incomplete partitions,
individual versions and counts over 4,096 are rejected. Local received-peer
observations expire after 180 seconds using wall and monotonic clocks, while
the already released public snapshot remains fixed until its window ends.
Forwarded cohorts are not added or expanded into invented individual records.

Each round exchanges one bounded aggregate with at most eight rotating
bootstrap/current neighbors. Two bounded startup retries handle peers still
starting. The envelope is limited to 32 KiB, with 64 global and two per-peer
concurrent streams, a per-peer burst of four then one request per second, and a
five-second IO deadline. No persistent presence history is kept by this code.
A recipient can independently record or republish an aggregate; the local TTL
does not promise deletion of copies held by other people.

## Local diagnostics and country choice

`aether_peers` is an identifiable diagnostic for native loopback HTTP only;
browser-origin, iroh and public gateway requests cannot use it. A non-loopback
HTTP bind disables these diagnostics too. It reports current authenticated
connections, deduplicated by transport node id:

```json
[{"node_id":"<64 hex characters>","role":"validator","version":null,
  "connected_since":1791440017,"last_seen":1791440031,"path":"direct"}]
```

Pinned validator ids can establish a peer's role. Other peer roles and versions
stay unknown because identifying presence metadata is no longer transmitted.
`path` is `direct`, `relay` or `unknown`; exact times remain local diagnostics.
No peer addresses or relay URLs enter these rows.

An optional ISO country choice stays in local process memory. It supplies only
this endpoint's broad continent input; ambiguous or unlisted mappings use
`unknown`. Without a country choice, the endpoint's home relay provides that
input using known iroh relay domains and region prefixes. A custom, absent or
unrecognized relay stays `unknown`. This does not locate the Mac.

Raw country codes never enter RPC snapshots or gossip. Other direct peers'
regions are unknown because they no longer advertise individual locations.
Consequently this endpoint's single country-derived continent contribution
normally folds into a broader safe group. A country choice does not guarantee
that a named continent will appear on the public globe.

The app's first-launch country screen must be answered before its country value
is passed to the local node. The configured policy can be default-on with that
notice or ask-before-sending. Refusal does not block wallet/node use.
Command-line operators may set `--presence-country=KR`; native loopback HTTP
can update the local input with `aether_setPresenceCountry` and `["KR"]` or
`[null]`. Browser-origin, iroh, public gateway and non-loopback HTTP requests
cannot change it. A setting change leaves the current public release frozen.

## Verification

Node unit tests inspect the actual QUIC request/reply bytes and the actual
public RPC handler's serialized JSON for a populated four-peer cohort with a
singleton residual. They assert identity, signature, country, exact-time and
raw-version removal; complementary folding; under-k totals; schema/network
rejection; non-additive forwarding; and a fixed release within each window.

The integration test starts four offline validators and two HTTP followers,
each with a local-only presence endpoint. It checks the actual RPC JSON from
all six processes, complete thresholded partitions, omitted identifiers and
local peer diagnostics. It stops and reaps every process even if an assertion
fails. Its data and logs stay under this worktree's `tmp/`; it never uses the
real node directory, DHT or relays.

Run builds only on this Mac, with the counting semaphore in the same shell
command. No fresh Jolt guest build is needed for these tests. Run the devnet
scenario separately so only one devnet runs in this lane:

```sh
task_root="$(git -C . rev-parse --show-toplevel)"
mkdir -p "$task_root/tmp"
export TMPDIR="$task_root/tmp"
export CARGO_BUILD_JOBS=4
cd "$task_root"
~/.claude/playbooks/aether-team/wait-compile.sh && cargo test -j4 -p aether-node --lib presence::tests -- --test-threads=1
~/.claude/playbooks/aether-team/wait-compile.sh && cargo test -j4 -p aether-node --test presence_devnet -- --test-threads=1
node --test apps/explorer/test/presence.test.mjs
```

The wallet pure-Swift suite covers schema-2 parsing, withheld states, country
first-launch gates and the five-language catalog. The lane lead owns gated
compiles and sequential devnet execution. A compile-slot wait over 20 minutes
requires committing the finished work and recording the remaining gates.
