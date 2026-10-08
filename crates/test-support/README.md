# Aether integration-test ports

TCP tests share `Port::reserve()` across crates. Keep the returned guard alive
until the server and its descendants have stopped; cloning a guard keeps the
same reservation alive. Use `port.port()` for CLI arguments, `port.addr()` for
an address, and `port.bind_tcp()` for a mock listener.

Each reservation holds an exclusive loopback **UDP** socket with the same
number as the TCP port. The kernel coordinates reservations between processes
and worktrees even when every lane has a different `TMPDIR`. TCP remains free
for the node to bind. Allocation uses 40000–49151 and skips ports already used
by TCP or UDP. UDP/iroh servers should keep their own bound sockets instead of
using this TCP-only guard.

The legacy security harness reserves a contiguous ten-port block and binds
the nine fallback TCP listeners itself. The binary can use only the requested
port, so its automatic fallback cannot occupy another test's reservation.

`TestChild::spawn(command, log_path)` captures both output streams in an
append-only log. Poll `try_wait()` while waiting for readiness, then call
`mark_started()` when the service responds. A recognized startup bind failure
is logged loudly and retried once at the same reserved addresses. A second
bind failure, another startup error, or an error after readiness is surfaced.
On Unix, retries and cleanup stop the whole private process group, including
supervisor descendants, before the ports can be reused. `wait_with_output()`
returns only the final attempt's combined log in both output fields; earlier
attempts remain in the diagnostic file.

`cargo test -p aether-test-support` checks two concurrently allocating harness
processes with separate temporary directories, retained reservations and clones,
transient and persistent bind errors, listener actors that fail while their
process stays alive, genuine/late failures, and descendant listener cleanup.
The allocation stress holds 4096 ports and performs 32768 allocations; it
reproduces overlapping live test ports with the former bind-zero/drop approach.
