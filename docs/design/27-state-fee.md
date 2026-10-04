# Fixed state growth fee (A5-1, new genesis)

`node_rewards || history_v2` activates this rule from genesis. Chain 7780 has
neither flag and keeps its unlimited, unused state-gas dimension and old fee
behavior. The node substitutes a 100,000-unit block state limit for the old
`u64::MAX` sentinel only on a new genesis. Proposers, validators, and the zkVM
guest all use `aether-execution::execute_block`'s same state-diff rule.

## Price and accounting

| Committed growth | State units | Burned fee |
|---|---:|---:|
| A storage slot whose pre-transaction value is zero and post-transaction value is nonzero | 100 | 0.0001 AETH |
| A new persistent account | 100 | 0.0001 AETH |
| New code, per byte | 1 | 0.000001 AETH |

One unit costs exactly `1_000_000_000_000` wei, a fixed genesis constant.
The code counts the final revm state diff against the transaction's pre-state:
reverts, repeated writes, and set-then-clear contribute zero. A sender's
initial nonce-only account record is exempt so its first zero-value plain
transfer remains free. The fixed fee is taken from the sender's reserved
state budget and burned like the existing execution base fee. The receipt
records state units and the fee; block settlement records the total burned.
There is no new recipient. A fee-recipient account created solely by revm's
tip credit is excluded from the user's state-growth count. A transaction signs a state-unit budget in
`header.gas.state` and must cover that budget before execution; actual growth
above it makes the transaction invalid. A zero-budget contract call that only
reads or changes existing fields can therefore remain free, while a zero-
balance storage writer is invalid in both the mempool and block execution.
For a queued future nonce, admission executes against a private view with the
sender nonce advanced to the signed value; it does not admit a known writer
merely because earlier pending transactions have not finalized yet.
The app and browser wallets read the fixed state price from node status and
default to `gas_limit / 200 + 100` units for funded contract calls, 100 for a
positive-value plain transfer, and zero for a zero-value plain transfer or a
zero-balance sender. Specialized callers can sign a tighter budget.

The 40,555-gas name commitment measured in audit round 5 can fit about
`15,000,000 / 40,555 = 369` fresh slots per below-target block. That costs
`369 × 0.0001 = 0.0369 AETH` per block, about 59% of one operator's
`1 / 16 = 0.0625 AETH` maximum share of the initial 1 AETH block reward.
At the audit's 300 calls per block, the burn is 0.03 AETH per block or
2,592 AETH per 86,400-block day. A normal swap or vault operation creating
one to three slots pays 0.0001–0.0003 AETH. At an illustrative $10/AETH
(there is no on-chain dollar peg), that is $0.001–$0.003, a fraction of a cent.
A 1,000-byte deployment adds 0.001 AETH for code plus 0.0001 AETH for its
account. These prices are consensus constants; changing them requires a new
protocol rule and guest program ID.

Each transaction and each block may add at most 512 net new storage slots;
the block also allows at most 100,000 state units. The 30M execution-gas cap
and EVM code-deposit gas further bound code bytes and accounts. System writes
are outside transaction charges: the free registration lane allows at most
four registrations per block and shares the registry's 16-per-epoch cap with
contract registration. Beacon answers allow at most 1,024 per block and only
four answer slots per epoch; they update each registered candidate's bounded
record. Thus neither system path provides uncapped permanent state growth.

The name service still needs its separate commitment bond and committer
binding: the fixed state fee closes the generic free-write path but does not
prevent cheap commitment refresh or compensate the service for abandoned
commitments.

## Remaining free-account exception

A fresh zero-balance signer that sends a zero-value plain transfer still
leaves a nonce-only account record. Charging that new record would break the
explicit free first-transaction requirement. The 30M execution-gas block cap
limits its rate, but repeated fresh keys can still accumulate those records
without paying. This exception needs a separate product and consensus decision
before claiming that *all* permanent state growth is economically bounded.
