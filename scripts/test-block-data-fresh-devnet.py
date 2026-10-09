#!/usr/bin/env python3
"""Restart a devnet follower on empty block data and measure certified sync.

macOS only. Uses an existing node binary; never builds or launches EastSea.
All fixture files, mountpoints and logs live under this checkout's tmp/.

    AETHER_CATCHUP_TEST_BIN=/path/to/aether python3 scripts/test-block-data-fresh-devnet.py

The RAM volume needs 6 GiB capacity because checkpoint downloads retain the
shipped 5 GiB recovery reserve. Only a small devnet's actual files are written.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.request import Request, urlopen


ROOT = Path(__file__).resolve().parents[1]
ACCOUNT = "0x00000000000000000000000000000000000b0b00"


def rpc(url, method, params=()):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": list(params)}).encode()
    with urlopen(Request(url, data=body, headers={"Content-Type": "application/json"}), timeout=3) as response:
        answer = json.load(response)
    if "error" in answer:
        raise RuntimeError(f"{method}: {answer['error']}")
    return answer.get("result")


class ReadProxy:
    """Pin a manifest and temporarily cap replay at its first child block."""

    def __init__(self, upstream, manifest, directory):
        self.upstream = upstream
        self.manifest = manifest
        self.cap = manifest["height"] + 1
        self.proof_account = None
        self.proof_source = None
        self.records = []
        self.lock = threading.Lock()
        self.log = (directory / "requests.jsonl").open("w")
        proxy = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                method, params = body["method"], body.get("params", [])
                started = time.monotonic()
                record = {"method": method, "params": params}
                try:
                    if method == "aether_snapshot":
                        value = proxy.manifest
                    elif method == "aether_getAccount" and proxy.proof_account is not None:
                        assert params == [ACCOUNT], "only the fixture account is exposed"
                        value = proxy.proof_account
                    elif method == "aether_getFinalized" and proxy.proof_source is not None:
                        value = rpc(proxy.proof_source, method, params)
                    elif method == "aether_getFinalizedRange" and proxy.cap is not None:
                        start, count = params
                        value = [] if start > proxy.cap else rpc(proxy.upstream, method, [start, min(count, proxy.cap - start + 1)])
                    elif method in {"aether_status", "aether_getFinalized", "aether_getFinalizedRange", "aether_snapshotChunk", "aether_proverProgram"}:
                        value = rpc(proxy.upstream, method, params)
                        if method == "aether_status" and proxy.cap is not None:
                            value["height"] = min(value["height"], proxy.cap)
                    else:
                        raise RuntimeError(f"unapproved fixture read: {method}")
                    record["height"] = value.get("height") if isinstance(value, dict) else None
                    record["ok"] = True
                    answer = {"jsonrpc": "2.0", "id": body["id"], "result": value}
                except Exception as error:
                    record["ok"] = False
                    record["error"] = str(error)
                    answer = {"jsonrpc": "2.0", "id": body["id"], "error": {"code": -32000, "message": str(error)}}
                record["seconds"] = time.monotonic() - started
                with proxy.lock:
                    proxy.records.append(record)
                    proxy.log.write(json.dumps(record) + "\n")
                    proxy.log.flush()
                data = json.dumps(answer).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.daemon_threads = True
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.url = f"http://127.0.0.1:{self.server.server_port}"

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=3)
        self.log.close()


class Scenario:
    def __init__(self, binary):
        self.binary = binary
        temp = ROOT / "tmp"
        temp.mkdir(exist_ok=True)
        self.directory = Path(tempfile.mkdtemp(prefix="block-data-fresh-", dir=temp))
        self.mount = self.directory / "ram"
        self.mount.mkdir()
        self.environment = os.environ.copy()
        self.environment.update(TMPDIR=str(self.directory), RUST_LOG="info", NO_COLOR="1")
        self.device = None
        self.children = []
        self.reservations = []
        self.proxy = None
        self.summary = {"binary": str(binary), "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "artifacts": str(self.directory)}

    def command(self, arguments, timeout=30):
        return subprocess.run(arguments, cwd=ROOT, env=self.environment, capture_output=True, text=True, timeout=timeout, check=True)

    def reserve(self):
        listener = socket.socket()
        listener.bind(("127.0.0.1", 0))
        self.reservations.append(listener)
        return listener.getsockname()[1]

    def spawn(self, name, arguments, ports=()):
        for listener in list(self.reservations):
            if listener.getsockname()[1] in ports:
                listener.close()
                self.reservations.remove(listener)
        log = (self.directory / f"{name}.log").open("w")
        try:
            child = subprocess.Popen(["/usr/bin/nice", "-n", "15", str(self.binary), *arguments], cwd=ROOT, env=self.environment, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        finally:
            log.close()
        self.children.append(child)
        print(f"started {name}: pid={child.pid}", flush=True)
        return child

    def stop(self, child):
        if child.poll() is None:
            os.killpg(child.pid, signal.SIGCONT)
            for sig, bound in ((signal.SIGINT, 5), (signal.SIGTERM, 3), (signal.SIGKILL, 3)):
                if child.poll() is not None:
                    break
                os.killpg(child.pid, sig)
                try:
                    child.wait(timeout=bound)
                except subprocess.TimeoutExpired:
                    continue
        child.wait(timeout=3)

    def wait(self, label, operation, bound=120):
        deadline = time.monotonic() + bound
        last = None
        while time.monotonic() < deadline:
            for child in self.children:
                if child.poll() is not None and child not in getattr(self, "stopped", set()):
                    raise AssertionError(f"owned node {child.pid} exited with {child.returncode}; logs: {self.directory}")
            try:
                last = operation()
                if last:
                    return last
            except (OSError, RuntimeError) as error:
                last = str(error)
            time.sleep(0.02)
        raise AssertionError(f"{label} did not complete in {bound}s: {last}; logs: {self.directory}")

    def height(self, url, minimum):
        def ready():
            status = rpc(url, "aether_status")
            return status if status["height"] >= minimum else None
        return self.wait(f"height {minimum} at {url}", ready)

    def pause(self, validators, value):
        for child in validators[2:]:
            os.kill(child.pid, signal.SIGSTOP if value else signal.SIGCONT)
        if value:
            time.sleep(1)

    def proof(self, name, account, follower_url):
        self.proxy.proof_account = account
        self.proxy.proof_source = follower_url
        result = self.command([str(self.binary), "balance", ACCOUNT, "--rpc", self.proxy.url, "--validators", "4"], timeout=30)
        (self.directory / f"{name}.txt").write_text(result.stdout)
        child = account["height"] + 1
        assert f"finality certificate of block {child}:" in result.stdout, result.stdout
        assert "EIP-7864 proof for this address verifies" in result.stdout, result.stdout
        self.proxy.proof_account = None
        self.proxy.proof_source = None
        return child

    def run(self):
        # The capacity satisfies the shipped download guard; no test seam or
        # production-volume exception changes the node's certificate checks.
        attached = self.command(["/usr/bin/hdiutil", "attach", "-nomount", "ram://12582912"])
        devices = re.findall(r"^(/dev/disk[0-9]+)\s", attached.stdout, re.M)
        assert len(devices) == 1, attached.stdout
        self.device = devices[0]
        self.summary["ram_device"] = self.device
        self.command(["/sbin/newfs_hfs", "-v", f"AetherFresh-{os.getpid()}", self.device.replace("/dev/disk", "/dev/rdisk")])
        self.command(["/sbin/mount_hfs", "-o", "nobrowse,nodev,nosuid", self.device, str(self.mount)])
        assert os.path.ismount(self.mount), "RAM volume must be mounted before any node starts"
        p2p = [self.reserve() for _ in range(4)]
        ports = [self.reserve() for _ in range(5)]
        urls = [f"http://127.0.0.1:{port}" for port in ports]
        resources = ["--prover-max-memory", "0", "--max-memory", "64M", "--min-free-disk", "0"]
        validators = []
        for i in range(4):
            peers = ",".join(f"{j+1}@127.0.0.1:{p2p[j]}" for j in range(4) if i != j)
            validators.append(self.spawn(f"validator-{i+1}", ["node", "--index", str(i+1), "--validators", "4", "--port", str(p2p[i]), "--rpc-port", str(ports[i]), "--data", str(self.mount / f"validator-{i+1}"), "--peers", peers, "--offline", "--block-time-ms", "100", *resources], [p2p[i], ports[i]]))
        self.height(urls[0], 5)
        old = self.mount / "old-follow"
        follow = ["follow", "--validators", "4", "--rpc-port", str(ports[4]), *resources]
        child = self.spawn("follower-old", [*follow, "--from-rpc", urls[0], "--data", str(old)], [ports[4]])
        old_status = self.height(urls[4], 8)
        self.stopped = {child}
        self.stop(child)
        assert (old / "state.redb").is_file(), "old follower must have persisted its own history"
        self.summary["old_followed_height"] = old_status["height"]
        print(f"old follower stopped at {old_status['height']}", flush=True)
        sent = self.command([str(self.binary), "send", "--rpc", urls[0], "--from-dev", "1", "--to", ACCOUNT, "--value", "777", "--wait"], timeout=60)
        assert "success=true" in sent.stdout, sent.stdout
        (self.directory / "transfer.txt").write_text(sent.stdout)
        self.height(urls[0], max(30, old_status["height"] + 10))
        self.pause(validators, True)
        manifest = self.wait("checkpoint manifest", lambda: rpc(urls[0], "aether_snapshot"))
        h = manifest["height"]
        checkpoint_account = rpc(urls[0], "aether_getAccount", [ACCOUNT])
        assert checkpoint_account["height"] == h, "snapshot and frozen account proof must describe the same finalized state"
        assert checkpoint_account["balance"] in (777, "777", "0x309"), checkpoint_account
        self.pause(validators, False)
        self.height(urls[0], h + 3)
        self.pause(validators, True)
        self.wait("snapshot child certificate", lambda: rpc(urls[0], "aether_getFinalized", [h+1]))
        self.proxy = ReadProxy(urls[0], manifest, self.directory)
        fresh = self.mount / "fresh-follow"
        fresh.mkdir()
        assert not list(fresh.iterdir()), "new block-data location must be empty, with no database copied"
        self.summary["fresh_directory_was_empty"] = True
        started = time.monotonic()
        self.spawn("follower-fresh", [*follow, "--from-rpc", self.proxy.url, "--data", str(fresh), "--checkpoint"])
        first = self.height(urls[4], h+1)
        elapsed = time.monotonic() - started
        assert first["height"] == h+1, "the fixture caps replay at exactly the first followed block"
        self.summary.update(checkpoint_height=h, first_followed_height=h+1, restart_to_first_followed_seconds=round(elapsed, 6), first_answer_height=first["height"])
        followed_account = rpc(urls[4], "aether_getAccount", [ACCOUNT])
        assert followed_account["height"] == h+1
        log = (self.directory / "follower-fresh.log").read_text()
        assert "checkpoint: started from a certified snapshot (history not replayed)" in log, log
        assert "checkpoint sync failed" not in log, log
        with self.proxy.lock:
            records = list(self.proxy.records)
        chunk = next(i for i, row in enumerate(records) if row["method"] == "aether_snapshotChunk" and row["ok"])
        anchor = next(i for i, row in enumerate(records) if row["method"] == "aether_getFinalized" and row["params"] == [h+1] and row["ok"])
        replay = next(i for i, row in enumerate(records) if i > anchor and row["method"] in {"aether_getFinalized", "aether_getFinalizedRange"} and row["params"][0] == h+1 and row["ok"])
        assert chunk < anchor < replay, "snapshot is downloaded and authenticated by H+1 before following"
        assert all(row["params"][0] >= h+1 for row in records if row["method"] in {"aether_getFinalized", "aether_getFinalizedRange"}), "fresh start must not replay old blocks before its checkpoint"
        self.summary["checkpoint_certificate_height"] = self.proof("checkpoint-proof", checkpoint_account, urls[4])
        print(f"empty-dir restart: checkpoint={h}, first followed={h+1}, elapsed={elapsed:.3f}s", flush=True)
        self.proxy.cap = None
        self.pause(validators, False)
        later = self.height(urls[4], h+4)
        self.summary.update(later_followed_height=later["height"], first_followed_state_certificate_height=self.proof("first-followed-proof", followed_account, urls[4]), first_followed_state_root=first["state_root"])
        assert (fresh / "state.redb").is_file()
        self.summary["passed"] = True

    def close(self):
        errors = []
        for child in reversed(self.children):
            try:
                self.stop(child)
            except Exception as error:
                errors.append(f"pid {child.pid}: {error}")
        self.summary["owned_pids"] = [child.pid for child in self.children]
        self.summary["all_owned_processes_reaped"] = all(child.returncode is not None for child in self.children)
        if self.proxy:
            self.proxy.close()
        for listener in self.reservations:
            listener.close()
        if self.device:
            try:
                if os.path.ismount(self.mount):
                    self.command(["/sbin/umount", str(self.mount)])
                self.command(["/usr/bin/hdiutil", "detach", self.device])
                self.summary["ram_disk_detached"] = True
            except Exception as error:
                errors.append(f"detach {self.device}: {error}")
                self.summary["ram_disk_detached"] = False
        self.summary["cleanup_errors"] = errors
        (self.directory / "summary.json").write_text(json.dumps(self.summary, indent=2) + "\n")
        print(json.dumps(self.summary, indent=2), flush=True)
        if errors:
            raise RuntimeError("fixture cleanup failed: " + "; ".join(errors))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=os.environ.get("AETHER_CATCHUP_TEST_BIN", str(ROOT / "target/release/aether")))
    args = parser.parse_args()
    binary = args.binary.expanduser().resolve()
    if sys.platform != "darwin":
        parser.error("this isolated RAM-disk scenario requires macOS")
    if not binary.is_file() or not os.access(binary, os.X_OK):
        parser.error("set AETHER_CATCHUP_TEST_BIN to an existing executable node binary; this script never builds")
    scenario = Scenario(binary)

    def interrupted(signum, _):
        raise KeyboardInterrupt(f"received signal {signum}")

    signal.signal(signal.SIGTERM, interrupted)
    try:
        scenario.run()
    except BaseException as error:
        scenario.summary["passed"] = False
        scenario.summary["error"] = str(error)
        raise
    finally:
        scenario.close()


if __name__ == "__main__":
    main()
