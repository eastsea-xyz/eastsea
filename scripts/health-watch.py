#!/usr/bin/env python3
"""Layer 0 of the health signal: public chain indicators, read from any RPC.

docs/design/32-health-signal.md §4.1 (O1). Everything here is already public
on the chain — no report, no personal data — so anyone can run it, and Pipln
running a copy gives Pipln no control. Point it at any node's JSON-RPC or at
the public read gateway; run it from cron every 10 minutes and it alerts
once per incident and once when the incident ends.

Indicators and thresholds (design §4.1, drafts):
  proof_rewards_zero  proofs paid in the last hour stayed 0 for 6 h, while the
                      7-day baseline before it was above 0 (the 10-05 incident:
                      five silent days; this alone would have caught it in 6 h)
  finality_slow       the block interval stayed >= 3x its 7-day median for 1 h
  beacons_drop        live registered Macs (beacon answers in the last epoch)
                      fell 30 % below their 7-day median
  upgrade_scheduled   a protocol upgrade is scheduled and not active (notice)
  release_new         a new ReleaseLog entry appeared (notice)

How each is read (whichever the endpoint allows):
  proofs   aether_getStorage on the prover escrow's per-block prover slot,
           sampled over the last settled hour (a full node's RPC); else
           aether_rewards of the --prover addresses given (works through the
           public gateway); else "unknown" — never a false zero.
  interval median timestamp gap of aether_recentBlocks
  beacons  aether_rewardStatus operators_online_last_epoch (full nodes only)
  upgrade  aether_status protocol / newest_scheduled
  releases aether_releaseEntries of --release-log (optional)

Usage:
  scripts/health-watch.py --rpc https://rpc.eastsea.xyz [--prover 0x...]
  scripts/health-watch.py --rpc http://127.0.0.1:18545 --mac        # macOS notification
  scripts/health-watch.py --rpc URL --notify-cmd 'mail -s eastsea me@example.com'
  scripts/health-watch.py --replay scripts/tests/fixtures/health-watch-2026-10-05.json
"""

import argparse
import datetime
import json
import pathlib
import random
import statistics
import subprocess
import sys
import time
import urllib.request

HOUR = 3600
DAY = 24 * HOUR

# Thresholds (design §4.1).
PROOF_SILENCE = 6 * HOUR
FINALITY_FACTOR = 3.0
FINALITY_SUSTAIN = 1 * HOUR
BEACON_DROP = 0.30
BASELINE = 7 * DAY
KEEP = 8 * DAY
# Samples see the chain at their own times, not on the second: a span counts
# as covered when the samples bound it to within this. With cron every 10
# minutes (two sampling steps of uncertainty at most), a proof silence is then
# reported no later than 6 h after the last paid proof — never later, possibly
# up to this much earlier.
SLACK = 20 * 60

# The prover escrow keeps, per block, who proved it (crates/execution/src/proofs.rs:
# slot = height << 2 | field, field 2 = prover; zero = nobody yet).
PROVER_ESCROW = "0x00000000000000000000000000000000000e5c00"
PROVER_FIELD = 2
# Proofs land after their block: only look at blocks this many seconds old.
SETTLE = 15 * 60
PROOF_SAMPLES = 24

DEFAULT_STATE = pathlib.Path.home() / ".local/state/eastsea-health-watch.json"


class RpcError(Exception):
    pass


def rpc(url, method, params=None, timeout=10):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params or []}).encode()
    request = urllib.request.Request(url, data=body, headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            answer = json.loads(response.read())
    except Exception as error:  # network, HTTP, JSON: all "this endpoint cannot say"
        raise RpcError(f"{method}: {error}") from error
    if answer.get("error"):
        raise RpcError(f"{method}: {answer['error']}")
    return answer.get("result")


# ---------------------------------------------------------------- collection

def to_ms(value):
    """Reward rows carry milliseconds; tolerate seconds and microseconds too."""
    value = int(value or 0)
    while value > 10**14:
        value //= 1000
    if 0 < value < 10**11:
        value *= 1000
    return value


def block_interval(call):
    """Median seconds between the newest blocks, or None."""
    try:
        blocks = call("aether_recentBlocks", [100]) or []
    except RpcError:
        return None
    stamps = sorted((b["height"], b["timestamp_ms"]) for b in blocks if "timestamp_ms" in b)
    gaps = [(t2 - t1) / 1000 / (h2 - h1) for (h1, t1), (h2, t2) in zip(stamps, stamps[1:]) if h2 > h1]
    return statistics.median(gaps) if gaps else None


def prover_slot(height):
    return hex((height << 2) + PROVER_FIELD)


def proofs_from_escrow(call, height, interval):
    """Estimated blocks proven in the last settled hour, from sampled prover slots."""
    if not interval or interval <= 0:
        return None
    per_hour = max(1, int(HOUR / interval))
    end = height - int(SETTLE / interval)
    if end - per_hour < 1:
        return None
    heights = sorted({end - per_hour + 1 + i * per_hour // PROOF_SAMPLES for i in range(PROOF_SAMPLES)})
    proven = 0
    for h in heights:
        answer = call("aether_getStorage", [PROVER_ESCROW, prover_slot(h)])
        value = answer.get("value") if isinstance(answer, dict) else answer
        if int(str(value or "0"), 0) != 0:
            proven += 1
    return round(proven / len(heights) * per_hour)


def proofs_from_rewards(call, provers, now):
    """Proof rewards paid to the given addresses in the last hour."""
    since = (now - HOUR) * 1000
    count = 0
    for address in provers:
        rows = call("aether_rewards", [address, 1000]) or []
        count += sum(1 for r in rows if r.get("kind") == "proof" and to_ms(r.get("timestamp_ms")) >= since)
    return count


def collect(url, now=None, provers=(), release_log=None, call=None):
    """One sample of every indicator this endpoint can answer (None = cannot)."""
    now = int(now if now is not None else time.time())
    call = call or (lambda method, params=None: rpc(url, method, params))
    status = call("aether_status", [])
    sample = {
        "t": now,
        "height": status.get("height"),
        "protocol": status.get("protocol"),
        "newest_scheduled": status.get("newest_scheduled"),
        "interval_s": block_interval(call),
        "proofs_last_hour": None,
        "proofs_source": None,
        "beacons": None,
        "releases": None,
    }
    try:
        sample["proofs_last_hour"] = proofs_from_escrow(call, sample["height"] or 0, sample["interval_s"])
        sample["proofs_source"] = "escrow" if sample["proofs_last_hour"] is not None else None
    except RpcError:
        pass  # the public gateway refuses aether_getStorage: try the rewards route
    if sample["proofs_last_hour"] is None and provers:
        try:
            sample["proofs_last_hour"] = proofs_from_rewards(call, provers, now)
            sample["proofs_source"] = "rewards"
        except RpcError:
            pass
    try:
        reward_status = call("aether_rewardStatus", []) or {}
        if reward_status.get("enabled"):
            sample["beacons"] = reward_status.get("operators_online_last_epoch")
    except RpcError:
        pass
    if release_log:
        try:
            sample["releases"] = (call("aether_releaseEntries", [release_log, 0, 64]) or {}).get("count")
        except RpcError:
            pass
    return sample


# ---------------------------------------------------------------- evaluation

def _known(samples, key):
    return [s for s in samples if s.get(key) is not None]


def _run_back(samples, predicate):
    """The newest unbroken run of samples meeting `predicate`, oldest first."""
    run = []
    for s in reversed(samples):
        if not predicate(s):
            break
        run.append(s)
    return list(reversed(run))


def evaluate(samples, now):
    """Active alerts {key: (level, sentence)} from the samples up to `now`. Pure."""
    alerts = {}
    window = [s for s in samples if now - BASELINE - PROOF_SILENCE <= s["t"] <= now]

    proofs = _known(window, "proofs_last_hour")
    zero_run = _run_back(proofs, lambda s: s["proofs_last_hour"] == 0)
    if zero_run:
        # Each sample covers the hour before it: the run's silence starts an
        # hour before its first sample.
        silent_since = zero_run[0]["t"] - HOUR
        before = [s["proofs_last_hour"] for s in proofs if silent_since - BASELINE <= s["t"] < zero_run[0]["t"]]
        usual = statistics.median(before) if before else 0
        if now - silent_since >= PROOF_SILENCE - SLACK and usual > 0:
            hours = round((now - silent_since) / HOUR)
            alerts["proof_rewards_zero"] = (
                "alert",
                f"No proof reward paid for about {hours} h (usually about {usual:.0f} an hour). "
                "Provers may be running a program the validators reject: check aether_proverStatus "
                "program_mismatch on a prover Mac.",
            )

    intervals = _known(window, "interval_s")
    baseline = [s["interval_s"] for s in intervals if s["t"] < now - FINALITY_SUSTAIN]
    if len(baseline) >= 3:
        usual = statistics.median(baseline)
        slow = _run_back(intervals, lambda s: s["interval_s"] >= FINALITY_FACTOR * usual)
        if slow and now - slow[0]["t"] >= FINALITY_SUSTAIN - SLACK:
            alerts["finality_slow"] = (
                "alert",
                f"Blocks have come every {slow[-1]['interval_s']:.1f} s for over an hour "
                f"(usually {usual:.1f} s): finality is slow.",
            )

    beacons = _known(window, "beacons")
    if beacons:
        base = [s["beacons"] for s in beacons if now - BASELINE <= s["t"] < now - HOUR]
        if len(base) >= 3:
            usual = statistics.median(base)
            latest = beacons[-1]["beacons"]
            if usual > 0 and latest < (1 - BEACON_DROP) * usual:
                alerts["beacons_drop"] = (
                    "alert",
                    f"Only {latest} registered Macs answered the last epoch (usually {usual:.0f}): "
                    "Macs are dropping off.",
                )

    latest = samples[-1] if samples else {}
    if (latest.get("newest_scheduled") or 0) > (latest.get("protocol") or 0):
        alerts["upgrade_scheduled"] = (
            "notice",
            f"Protocol {latest['newest_scheduled']} is scheduled (running {latest['protocol']}): "
            "apps that cannot run it must update before it activates.",
        )
    releases = _known(samples, "releases")
    if len(releases) >= 2 and releases[-1]["releases"] > releases[-2]["releases"]:
        alerts[f"release_new_{releases[-1]['releases']}"] = (
            "notice", f"A new ReleaseLog entry was published (now {releases[-1]['releases']}).")
    if latest and latest.get("proofs_last_hour") is None and "t" in latest:
        alerts["proofs_unknown"] = (
            "notice",
            "This endpoint cannot show proof rewards: pass --prover addresses for the public "
            "gateway, or point --rpc at a full node.",
        )
    return alerts


# What each incident's one "resolved" notification says.
RESOLVED = {
    "proof_rewards_zero": "proof rewards are being paid again",
    "finality_slow": "blocks are coming at their usual pace again",
    "beacons_drop": "registered Macs are back to their usual count",
    "upgrade_scheduled": "the scheduled protocol is active",
    "proofs_unknown": "proof rewards are readable again",
}


def transitions(active_before, alerts):
    """(new, resolved) alert keys: one notification per incident, one per end."""
    new = [k for k in alerts if k not in active_before]
    resolved = [k for k in active_before if k not in alerts]
    return new, resolved


# ---------------------------------------------------------------- replay

def _epoch(value):
    """Seconds since 1970 from a number or an ISO-8601 UTC string."""
    if isinstance(value, (int, float)):
        return int(value)
    return int(datetime.datetime.fromisoformat(value.replace("Z", "+00:00")).timestamp())


def expand(fixture):
    """A fixture's samples: given outright, or synthesised from segments.

    A segment holds rates over [from, to). Each synthesised sample reports what
    a live run would see at its time: proofs paid over the hour before it (the
    overlap of that hour with each segment's rate — an onset between samples
    fades in exactly as it would live), the block interval and the beacon
    count of the segment it falls in, all with seeded noise."""
    if "samples" in fixture:
        return fixture["samples"]
    rng = random.Random(fixture.get("seed", 0))
    segments = [dict(seg, start=_epoch(seg["from"]), end=_epoch(seg["to"])) for seg in fixture["segments"]]
    step = fixture["step_s"]

    def jitter(value, spread):
        return None if value is None else max(0.0, value * (1 + rng.uniform(-spread, spread)))

    samples = []
    t = segments[0]["start"] + HOUR  # the first full hour of data
    while t < segments[-1]["end"]:
        seg = next(g for g in segments if g["start"] <= t < g["end"])
        paid = sum(g["proofs_per_hour"] * max(0, min(t, g["end"]) - max(t - HOUR, g["start"])) / HOUR
                   for g in segments)
        samples.append({
            "t": t,
            "protocol": seg.get("protocol", 3),
            "newest_scheduled": seg.get("newest_scheduled", seg.get("protocol", 3)),
            "proofs_last_hour": round(jitter(paid, 0.25)),
            "interval_s": jitter(seg.get("interval_s"), 0.1),
            "beacons": None if seg.get("beacons") is None else round(jitter(seg["beacons"], 0.05)),
            "releases": seg.get("releases"),
        })
        t += step
    return samples


def replay(fixture):
    """Feed a recorded or synthesised window sample by sample, as cron would.
    Returns the list of (t, "raised"|"resolved", key, sentence)."""
    seen, active, log = [], {}, []
    for sample in expand(fixture):
        seen.append(sample)
        seen = [s for s in seen if s["t"] >= sample["t"] - KEEP]
        alerts = evaluate(seen, sample["t"])
        new, resolved = transitions(active, alerts)
        log += [(sample["t"], "raised", k, alerts[k][1]) for k in new]
        log += [(sample["t"], "resolved", k, RESOLVED.get(k, "over")) for k in resolved]
        active = {k: v[1] for k, v in alerts.items()}
    return log


# ---------------------------------------------------------------- main

def notify(text, args):
    print(text, flush=True)
    if args.mac:
        script = f"display notification {json.dumps(text)} with title \"EastSea health\""
        subprocess.run(["osascript", "-e", script], check=False)
    if args.notify_cmd:
        subprocess.run(args.notify_cmd, shell=True, input=text.encode(), check=False)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--rpc", help="any node's JSON-RPC URL, or the public read gateway")
    parser.add_argument("--prover", action="append", default=[], help="prover address to count proof rewards of (repeatable)")
    parser.add_argument("--release-log", help="ReleaseLog contract address (optional)")
    parser.add_argument("--state", type=pathlib.Path, default=DEFAULT_STATE, help="where samples and active alerts are kept")
    parser.add_argument("--mac", action="store_true", help="also post a macOS notification")
    parser.add_argument("--notify-cmd", help="also pipe each alert's text into this shell command")
    parser.add_argument("--replay", type=pathlib.Path, help="replay a fixture instead of reading an RPC")
    args = parser.parse_args(argv)

    if args.replay:
        for t, kind, key, text in replay(json.loads(args.replay.read_text())):
            stamp = time.strftime("%Y-%m-%dT%H:%MZ", time.gmtime(t))
            print(f"{stamp} {kind:8} {key}: {text}")
        return 0
    if not args.rpc:
        parser.error("--rpc is required (or --replay)")

    try:
        state = json.loads(args.state.read_text())
    except (OSError, ValueError):
        state = {"samples": [], "active": {}}
    try:
        sample = collect(args.rpc, provers=args.prover, release_log=args.release_log)
    except RpcError as error:
        print(f"health-watch: {args.rpc} did not answer: {error}", file=sys.stderr)
        return 2
    samples = [s for s in state.get("samples", []) + [sample] if s["t"] >= sample["t"] - KEEP]
    alerts = evaluate(samples, sample["t"])
    active = state.get("active", {})
    new, resolved = transitions(active, alerts)
    for key in new:
        level, text = alerts[key]
        notify(f"[{level}] {text}", args)
    for key in resolved:
        notify(f"[resolved] {key}: {RESOLVED.get(key, 'over')}", args)
    args.state.parent.mkdir(parents=True, exist_ok=True)
    args.state.write_text(json.dumps({"samples": samples, "active": {k: v[1] for k, v in alerts.items()}}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
