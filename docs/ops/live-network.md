# Live network observations (0.7.4)

`aether_presence` answers how many distinct node keys this node has heard from
recently. The explorer polls it every 10 seconds; a wallet with a local node
uses the same read for its Network globe and menu-panel count. The UI says
**what this node can see**.
This is an observation, independent of consensus, validator votes and hourly
candidate beacons. It makes no chain writes and uses no central counting service.

## RPC

Both reads take an empty parameter array, and support `eastsea_` aliases.
All times below are Unix seconds.

`aether_peers` returns an array of this endpoint's current authenticated iroh
connections, deduplicated by transport node id:

```json
[{"node_id":"<64 hex characters>","role":"validator","version":"0.7.4",
  "connected_since":1791440000,"last_seen":1791440010,"path":"direct"}]
```

Role and version are null until known. A pinned validator id can establish its
role before a presence exchange. Path is `direct`, `relay`, or `unknown` during
path negotiation. Last seen is the time this tracker observed received packets;
connection and path changes are read again on each query. Neither RPC returns
addresses, relay URLs, cities or coordinates.

`aether_presence` returns schema 1:

```json
{"schema":1,"available":true,"observer":"<node id>",
 "scope":"what this node can see","observed_at":1791440010,"ttl_seconds":180,
 "total":6,"by_role":{"validator":4,"candidate":0,"follower":2},
 "by_version":{"0.7.4":6},
 "by_region":{"asia":4,"europe":2,"north_america":0,"south_america":0,
              "africa":0,"oceania":0,"unknown":0},
 "by_country":{},
 "nodes":[{"node_id":"<node id>","role":"follower","version":"0.7.4",
           "timestamp":1791440000,"last_seen":1791440010,"region":"asia"}]}
```

The abbreviated `nodes` example illustrates one row; a real response lists all
`total` node ids. Roles and regions always include their zero buckets. A country
field appears on a row only when its operator opted in. An endpoint without
presence support reports `available:false`, a null observer and empty counts;
clients show unavailable or hide the line, rather than implying no Macs exist.
The public read gateway permits `aether_presence`, while peers and settings stay
outside its read allowlist.

## Gossip, identity and limits

Nodes emit signed pings on startup and approximately every 60 seconds over
`aether/presence/1`, a versioned ALPN older nodes can reject independently of
consensus and RPC. Two bounded startup retries handle peers that are still
starting; subsequent rounds retain the normal minute cadence. Each ping binds
schema, node id, role, build version, timestamp,
continent, optional country and network (`chain id:group`) to the node key.
An additional signed transport binding lets a follower keep its existing wallet
RPC endpoint separate from its stable node identity. Candidate resharing retains
its established endpoint ownership; rotations and paused followers keep the
original signed presence identity when that key remains readable.

Records expire after 180 seconds, using monotonic deadlines shortened by the
signed timestamp's age. Duplicate or older pings never renew expiry. Signatures,
canonical ids, network binding, field sizes, ISO country codes and clock freshness
are checked before acceptance. A node id gets at most one update per 10 seconds.
Timestamps more than 10 seconds ahead are refused.

The table holds at most 4,096 identities. A packet carries at most 64 records,
each at most 512 bytes, within a 32 KiB transport envelope. A round exchanges
packets with at most eight rotating bootstrap/current neighbors. Transport work
has 64 global and two per-peer concurrent streams, a per-peer burst of four then
one request per second, and a five-second IO deadline. Large tables rotate their
packet sample. Expired entries are removed during reads and exchanges.

Signatures prove possession of a key. Role, version and region remain signed
claims, and presence does not prove one physical Mac per key. Partitions, old
nodes, sampling, invalid clocks and the table bound can reduce what a node sees.
It is unsuitable for consensus, rewards, eligibility or authoritative network
totals. Validators still use their finalized roster and candidate beacons.

## Regions and country sharing

The continent comes from the endpoint's actual home relay, using known iroh
relay domains and region prefixes (for example `aps1` means Asia). A custom,
unrecognized or absent relay reports `unknown`. The continent describes the
relay; it does not locate the Mac.

The wallet shares an ISO 3166-1 alpha-2 country from `Locale.current.region`
by default, with the first-launch notice specified in docs/design/38-live-globe.md.
The owner can turn it off in Settings. It makes no location or IP lookup.
The globe folds country groups below three into their relay continent. Command-line
operators can supply a country with `--presence-country=KR` on `node`, `follow` or `run`.
Settings updates use `aether_setPresenceCountry` with `["KR"]` or `[null]` on
native loopback HTTP. Browser-origin requests, public iroh requests and the
public read gateway cannot change it. Settings refreshes unattended arguments
and restarts the supervisor so a later role change cannot restore an old opt-in.
Already propagated country records can remain visible until replaced or their
180-second TTL expires.

The wallet's globe receives only the native aggregate projection: schema-1
individual fields stay outside WebKit, and countries are disclosed only at k≥3
within a relay continent. Schema-1 quality and reserve seating are unavailable;
the wallet displays their absence instead of estimating them from signed pings.
Validated schema-3 quality summaries enable the shared gradient and region strips.
The globe page itself cannot connect to any RPC or external asset. See
`docs/design/38-live-globe.md` for the bundled origin, country notice and fixture
renderer contract.

The binary advertises marketing version `0.7.4`; release builders can override
it with compile-time `AETHER_VERSION`. Keep that value aligned with the release.

## Verification

The node integration test creates four TCP validators and two HTTP followers,
each with a local-only iroh presence endpoint. Followers connect through
different validators; all six observers must report six unique identities within
two minutes. It never publishes devnet keys to the DHT or attaches to relays.
The hidden dev presence bind/peer options accept only loopback endpoints and
require offline validators or explicit loopback HTTP follower sources.

Set `TMPDIR` to this worktree's `tmp` before tests so node data stays isolated:

```sh
task_root="$(git -C . rev-parse --show-toplevel)"
mkdir -p "$task_root/tmp"
export TMPDIR="$task_root/tmp"
cd "$task_root"
~/.claude/playbooks/aether-team/wait-compile.sh
cargo test -j4 -p aether-node --tests
```

The explorer suite and `scripts/test-swift-pure.sh` also cover count parsing,
unavailable states, polling, country flags and the five-language wallet catalog.
