"""scripts/health-watch.py: layer-0 public chain indicators (docs/design/32-health-signal.md §4.1, O1).

    python3 -m unittest scripts/tests/test_health_watch.py
"""

import datetime
import importlib.util
import json
import pathlib
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("health_watch", ROOT / "scripts/health-watch.py")
watch = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(watch)

FIXTURE = ROOT / "scripts/tests/fixtures/health-watch-2026-10-05.json"
HOUR = 3600


def epoch(iso):
    return int(datetime.datetime.fromisoformat(iso.replace("Z", "+00:00")).timestamp())


def steady(start, hours, step=600, **fields):
    """Samples every `step` seconds for `hours`, with the given fields."""
    base = {"proofs_last_hour": 80, "interval_s": 1.0, "beacons": 12, "protocol": 3, "newest_scheduled": 3, "releases": None}
    base.update(fields)
    return [dict(base, t=start + i * step) for i in range(int(hours * HOUR / step))]


class ReplayOf20261005(unittest.TestCase):
    """The incident: proof rewards silently 0 for five days. Synthesised from
    the design's description (no recorded series exists) — see the fixture's
    "about"."""

    @classmethod
    def setUpClass(cls):
        cls.fixture = json.loads(FIXTURE.read_text())
        cls.onset = epoch(cls.fixture["onset"])
        cls.fix = epoch(cls.fixture["fix"])
        cls.log = watch.replay(cls.fixture)

    def test_fixture_says_it_is_synthesised(self):
        self.assertIn("SYNTHESISED", self.fixture["about"])

    def test_alert_within_six_hours_of_the_onset(self):
        raised = [t for t, kind, key, _ in self.log if kind == "raised" and key == "proof_rewards_zero"]
        self.assertTrue(raised, "the five silent days must raise the proof alert")
        delay = raised[0] - self.onset
        self.assertLessEqual(delay, 6 * HOUR, f"alert {delay / HOUR:.2f} h after the onset")
        self.assertGreaterEqual(delay, 5 * HOUR, "not earlier than the threshold allows")

    def test_one_alert_for_the_whole_incident_and_one_resolution(self):
        kinds = [(kind, key) for _, kind, key, _ in self.log]
        self.assertEqual(kinds, [("raised", "proof_rewards_zero"), ("resolved", "proof_rewards_zero")],
                         "five days of silence are one incident: no repeated alerts, nothing else raised")
        resolved = next(t for t, kind, _, _ in self.log if kind == "resolved")
        self.assertTrue(self.fix <= resolved <= self.fix + HOUR, "resolved within an hour of the fix")

    def test_no_alert_during_the_healthy_week_before(self):
        self.assertTrue(all(t > self.onset for t, _, _, _ in self.log))

    def test_the_command_line_replay(self):
        import contextlib
        import io
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            self.assertEqual(watch.main(["--replay", str(FIXTURE)]), 0)
        self.assertIn("2026-09-30T09:", out.getvalue())
        self.assertIn("raised   proof_rewards_zero", out.getvalue())


class Indicators(unittest.TestCase):
    T0 = epoch("2026-10-01T00:00:00Z")

    def test_quiet_chain_has_no_alerts(self):
        samples = steady(self.T0, 8 * 24)
        self.assertEqual(watch.evaluate(samples, samples[-1]["t"]), {})

    def test_zero_proofs_on_a_chain_that_never_had_any_is_not_an_alert(self):
        samples = steady(self.T0, 48, proofs_last_hour=0)
        self.assertNotIn("proof_rewards_zero", watch.evaluate(samples, samples[-1]["t"]))

    def test_unknown_proofs_never_count_as_zero(self):
        samples = steady(self.T0, 24) + steady(self.T0 + 24 * HOUR, 12, proofs_last_hour=None)
        alerts = watch.evaluate(samples, samples[-1]["t"])
        self.assertNotIn("proof_rewards_zero", alerts)
        self.assertIn("proofs_unknown", alerts)

    def test_finality_three_times_slower_for_an_hour(self):
        base = steady(self.T0, 24)
        start = base[-1]["t"] + 600
        slow = steady(start, 1.5, interval_s=3.2)
        early = [s for s in slow if s["t"] < start + 40 * 60]
        self.assertNotIn("finality_slow", watch.evaluate(base + early, early[-1]["t"]), "under an hour: wait")
        self.assertIn("finality_slow", watch.evaluate(base + slow, slow[-1]["t"]))
        twice = steady(start, 3, interval_s=2.0)
        self.assertNotIn("finality_slow", watch.evaluate(base + twice, twice[-1]["t"]), "2x is not 3x")

    def test_beacons_down_thirty_percent(self):
        base = steady(self.T0, 48, beacons=20)
        later = base[-1]["t"] + 600
        self.assertIn("beacons_drop", watch.evaluate(base + steady(later, 0.2, beacons=13), later))
        self.assertNotIn("beacons_drop", watch.evaluate(base + steady(later, 0.2, beacons=15), later))

    def test_scheduled_upgrade_and_new_release_are_notices(self):
        samples = steady(self.T0, 2, newest_scheduled=4, releases=3) + steady(self.T0 + 2 * HOUR, 0.2, newest_scheduled=4, releases=4)
        alerts = watch.evaluate(samples, samples[-1]["t"])
        self.assertEqual(alerts["upgrade_scheduled"][0], "notice")
        self.assertEqual(alerts["release_new_4"][0], "notice")

    def test_transitions_are_once_per_incident(self):
        self.assertEqual(watch.transitions({}, {"a": ("alert", "x")}), (["a"], []))
        self.assertEqual(watch.transitions({"a": "x"}, {"a": ("alert", "x")}), ([], []))
        self.assertEqual(watch.transitions({"a": "x"}, {}), ([], ["a"]))


class FakeChain:
    """An RPC endpoint in memory: `public` refuses what the read gateway refuses."""

    def __init__(self, height=100_000, proven_every=1, public=False, rewards=None):
        self.height, self.proven_every, self.public, self.rewards = height, proven_every, public, rewards or {}
        self.calls = []

    def __call__(self, method, params=None):
        self.calls.append((method, params))
        if method == "aether_status":
            return {"height": self.height, "protocol": 3, "newest_scheduled": 3}
        if method == "aether_recentBlocks":
            return [{"height": self.height - i, "timestamp_ms": (self.height - i) * 1000} for i in range(100)]
        if self.public and method in ("aether_getStorage", "aether_rewardStatus"):
            raise watch.RpcError(f"{method}: method not allowed on the public gateway")
        if method == "aether_getStorage":
            address, slot = params
            assert address == watch.PROVER_ESCROW
            value = int(slot, 16)
            assert value & 3 == watch.PROVER_FIELD, "the prover field of the block's slot"
            height = value >> 2
            return {"value": "0x1234" if self.proven_every and height % self.proven_every == 0 else "0x0"}
        if method == "aether_rewardStatus":
            return {"enabled": True, "operators_online_last_epoch": 9}
        if method == "aether_rewards":
            return self.rewards.get(params[0], [])
        raise watch.RpcError(f"{method}: not found")


class Collection(unittest.TestCase):
    NOW = epoch("2026-10-06T00:00:00Z")

    def test_full_node_counts_proven_blocks_from_the_escrow(self):
        chain = FakeChain(proven_every=1)
        sample = watch.collect("x", now=self.NOW, call=chain)
        self.assertEqual(sample["proofs_source"], "escrow")
        self.assertEqual(sample["proofs_last_hour"], 3600, "1 s blocks, every one proven")
        self.assertEqual(sample["beacons"], 9)
        self.assertAlmostEqual(sample["interval_s"], 1.0)
        heights = [int(p[1], 16) >> 2 for m, p in chain.calls if m == "aether_getStorage"]
        self.assertTrue(all(h <= chain.height - watch.SETTLE for h in heights), "only settled blocks are sampled")

    def test_nothing_proven_reads_zero(self):
        sample = watch.collect("x", now=self.NOW, call=FakeChain(proven_every=0))
        self.assertEqual(sample["proofs_last_hour"], 0)

    def test_public_gateway_falls_back_to_prover_rewards(self):
        rows = [{"kind": "proof", "timestamp_ms": (self.NOW - 600) * 1000},
                {"kind": "proof", "timestamp_ms": (self.NOW - 2 * HOUR) * 1000},
                {"kind": "node", "timestamp_ms": (self.NOW - 60) * 1000}]
        chain = FakeChain(public=True, rewards={"0xabc": rows})
        sample = watch.collect("x", now=self.NOW, provers=["0xabc"], call=chain)
        self.assertEqual((sample["proofs_source"], sample["proofs_last_hour"]), ("rewards", 1))
        self.assertIsNone(sample["beacons"], "the gateway does not answer reward status")

    def test_public_gateway_without_provers_is_unknown_not_zero(self):
        sample = watch.collect("x", now=self.NOW, call=FakeChain(public=True))
        self.assertIsNone(sample["proofs_last_hour"])

    def test_reward_timestamps_in_seconds_or_microseconds(self):
        self.assertEqual(watch.to_ms(1_790_000_000), 1_790_000_000_000)
        self.assertEqual(watch.to_ms(1_790_000_000_000), 1_790_000_000_000)
        self.assertEqual(watch.to_ms(1_790_000_000_000_000), 1_790_000_000_000)

    def test_cron_run_keeps_state_and_alerts_once(self):
        with tempfile.TemporaryDirectory() as directory:
            state = pathlib.Path(directory) / "state.json"
            before = steady(self.NOW - 8 * 24 * HOUR, 8 * 24 - 7) + steady(self.NOW - 7 * HOUR, 7, proofs_last_hour=0)
            state.write_text(json.dumps({"samples": before, "active": {}}))
            original = watch.collect
            watch.collect = lambda *a, **k: dict(before[-1], t=self.NOW)
            try:
                import contextlib
                import io
                first, second = io.StringIO(), io.StringIO()
                with contextlib.redirect_stdout(first):
                    watch.main(["--rpc", "http://node", "--state", str(state)])
                with contextlib.redirect_stdout(second):
                    watch.main(["--rpc", "http://node", "--state", str(state)])
            finally:
                watch.collect = original
            self.assertIn("No proof reward paid", first.getvalue())
            self.assertEqual(second.getvalue(), "", "the same incident is not announced twice")
            self.assertIn("proof_rewards_zero", json.loads(state.read_text())["active"])


if __name__ == "__main__":
    unittest.main()
