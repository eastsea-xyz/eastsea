#!/usr/bin/env python3
"""Compare block hash and state root, height by height, between the dry-run
follower (new binary, clone of v3) and a live validator. Read-only RPC.

usage: compare.py <follower_port> <live_port> <from_height> [to_height] [step]
"""
import json
import sys
import urllib.request


def rpc(port, method, params):
    req = urllib.request.Request(
        f"http://127.0.0.1:{port}",
        data=json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode(),
        headers={"content-type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=5) as r:
        body = json.load(r)
    if "error" in body:
        raise RuntimeError(f"{method} on {port}: {body['error']}")
    return body["result"]


def main():
    fol, live, start = int(sys.argv[1]), int(sys.argv[2]), int(sys.argv[3])
    fol_head = rpc(fol, "aether_status", [])["height"]
    live_head = rpc(live, "aether_status", [])["height"]
    end = int(sys.argv[4]) if len(sys.argv) > 4 else min(fol_head, live_head)
    step = int(sys.argv[5]) if len(sys.argv) > 5 else 1
    checked = mismatched = 0
    for h in range(start, end + 1, step):
        a = rpc(fol, "aether_getBlock", [h])
        b = rpc(live, "aether_getBlock", [h])
        same = a.get("hash") == b.get("hash") and a.get("state_root") == b.get("state_root")
        checked += 1
        if not same:
            mismatched += 1
            print(f"MISMATCH {h}: follower {a.get('hash')} {a.get('state_root')} live {b.get('hash')} {b.get('state_root')}")
    print(
        f"follower_head={fol_head} live_head={live_head} checked={checked} "
        f"range={start}..{end} step={step} mismatched={mismatched}"
    )
    last = rpc(fol, "aether_getBlock", [end])
    print(f"last {end} hash={last.get('hash')} state_root={last.get('state_root')}")
    sys.exit(1 if mismatched else 0)


if __name__ == "__main__":
    main()
