# Candidate eligibility diagnostics

`aether_candidates` (also `eastsea_candidates`) reads finalized state. Existing
fields remain available. The additional fields explain a candidate's current
qualification for the next scheduled draw; continued beaconing is required
until the pool freezes, and qualification does not guarantee a seat.

| Field | Meaning |
| --- | --- |
| `next_draw_epoch` | Next strictly future draw boundary, in registry epochs, not draw numbers. |
| `open_seats` | Maximum incoming seats allowed by that draw's budget, including replacements at the committee ceiling. Operator caps, availability and the pool can reduce the result. A finalized handoff scheduled before the draw supplies its committed roster size. Without a known roster the count is zero. |
| Candidate `missed` | The registry's existing missed-epoch counter. |
| Candidate `eligible_next_draw` | Whether the current observations meet the qualification requirements, assuming the Mac maintains liveness until the next draw. |
| Candidate `why_not` | First unmet requirement: `streak`, `uptime`, `last_epoch`, `v3_stability`, or `none` when qualified. |
| Candidate `hours_to_eligible` | Approximate hours to recover the numeric shortfall through successful beacons; zero when qualified, null when unknown. This excludes waiting for a draw or handoff. |

Legacy qualification uses the existing minimum streak and
`missed * 20 <= streak` requirement. A streak of 122 with 7 missed epochs needs
18 more successful epoch beacons to reach 140, provided no further epochs are
missed. The wall-clock estimate uses observed finalized block timestamps and
the registry's epoch length, so accelerated devnets are not labelled as
one-hour epochs.

Registry v3 uses registration age, the latest distributed stability window,
the profile at the next draw's hour, and the leaving flag. Registration age
maps to `streak`; unavailable stability, a low hour profile, departure, or an
invalid endpoint maps to `v3_stability`. No stability recovery time is guessed.

An epoch can still be in progress when the RPC is read. Recency therefore
accepts a verified beacon in the current or previous epoch, while v3 stability
uses the completed epochs distributed at the current epoch's start. An answer
during the current epoch does not make an otherwise healthy candidate fail
the view. At the actual draw, `rotation::eligible` still uses the existing exact
previous-epoch rule and determines the authoritative frozen pool.

Only finalization logs pool shortfalls at INFO: an empty candidate pool, or
fewer candidates than the available seat budget. The records carry `draw`,
`pool_size`, and `open_seats`. RPC polling and speculative block execution do
not emit these messages.

The wallet reads the verdict without duplicating the rules. Old RPC responses,
missing candidates and failed reads supply no additional line. The pure text
helper shows an approximate ETA, eligibility while waiting for a draw, or the
reported reason in English or Korean.
