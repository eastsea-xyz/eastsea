#!/usr/bin/env python3
"""Refresh the contract inventory/measurements without inventing unrun results."""
import argparse
from collections import defaultdict
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
REPORT = ROOT / "docs/research/contracts-onchain-2026-10-06.md"
FIXTURES = ROOT / "crates/contracts-onchain/fixtures/artifacts.json"
BEGIN = "<!-- CONTRACTS-ONCHAIN-TABLE:BEGIN -->"
END = "<!-- CONTRACTS-ONCHAIN-TABLE:END -->"
WORKFLOWS_BEGIN = "<!-- CONTRACTS-ONCHAIN-WORKFLOWS:BEGIN -->"
WORKFLOWS_END = "<!-- CONTRACTS-ONCHAIN-WORKFLOWS:END -->"
SCOPES = {
    "AtomicSwap": "lock/claim/refund; native/ERC20, SHA-256, timeout, fees, failed payouts, callbacks",
    "AtomicSwapEVM": "same complete swap lifecycle and errors as AtomicSwap",
    "CommitteeRegistry": "signed attestation, registrar/caller/duplicate/cap/epoch/beacon; predeploy v2 compatibility",
    "CommitteeRegistryV3": "registration, beacon, leaving/back, epochs, reserve sentinel, registrar rotation/revocation",
    "EastSeaAccount": "7702, owner/session/recovery mutations, P-256 signatures, limits, expiry, replay, revoke, F-05, ERC-1271, NFT receive",
    "EastSeaNames": "commit/reveal min/max age, register/clear/renew, transfer, records/reverse, grace, refund callbacks",
    "EastSeaVault": "signed spend/proposal/approve/cancel/execute/settings, quorum/delay/nonce, token/native callbacks",
    "EastSeaVaultFactory": "predict/create, deterministic address, duplicate salt, invalid owner configuration",
    "MerkleDistributor": "sorted proofs, sponsored claim, duplicate/expired/bad proof, sweep, callbacks",
    "MerkleDistributorFactory": "create/atomic funding, bad amount/token/root/expiry, taxed-token refusal",
    "Randomness": "no public mutation; paid deployment, unpublished/published epoch reads and explicit system seed writes",
    "ReleaseLog": "7705 pin, permissionless publish, bounds/empty inputs, duplicate metadata, receipt commitment",
    "TokenBatch": "send; shape/zero/allowance/failure/taxed/self-recipient/delta checks, atomicity",
    "TokenLocker": "lock/extend/withdraw, caller/amount/token/time, taxed deposits, callbacks, retry",
    "TokenVesting": "create/claim/cancel, cliff/end/permissions, taxed deposit, callbacks, uint256-max arithmetic",
    "AgentVending": "order/deliver/refund, role/amount/hash/deadline/double-pay, brake, callbacks",
    "AllOrNothingCrowdfund": "contribute/refund/withdraw, failed/successful rounds, deadlines, brake, callbacks",
    "AmmFactory": "createPair, ordering/zero/same/duplicates/brake, empty-pair reuse",
    "AmmPair": "mint/burn/swap/skim/sync, LP ERC20, invariant/reserve/liquidity, TWAP, taxed tokens, callbacks",
    "AmmRouter": "liquidity add/remove, exact/taxed swaps, multihop, missing/empty pools, slippage/deadline",
    "BondingLaunchpad": "buy/sell/graduate, fee/tax/cap/decay, poisoned/empty pairs, braked exit, callbacks",
    "CommitRevealRaffle": "enter/reveal/drawWithoutSeed, block seed, early/bad/withheld seed, payout callbacks",
    "Editions1155": "create/mint/withdraw, limits, ERC1155 approvals/transfers/batches, royalties, narrowing, callbacks",
    "FixedPriceMarket": "list/buy/cancel/withdraw/brake, ERC721 escrow, price/caller, royalties/credits, callbacks; ERC1155 refused",
    "FixedSupplyToken": "ERC20 transfer/approve/transferFrom, genuine EIP-2612 permit, expiry/nonce/signature/zero/max",
    "InvoiceBook": "issue/settle/void/purge, roles/amount/memo/deadline/status, brake, payouts/callbacks",
    "LinearVesting": "preapproved constructor deposit, cliff/partial/full/public claim, donation conservation",
    "MerkleAirdrop": "native claim/sweep, sorted proof/deadline/double/max, callbacks, 128-user bucket capacity",
    "MilestoneEscrow": "createDeal/approveMilestone/sellerWithdraw/buyerRefund, roles/status/amount, brake/callbacks",
    "NameGatedDrop": "name eligibility/expiry/duplicate/amount/pool, claim/sweep and callbacks",
    "OnchainNFT": "mint/burn, ERC721 approvals/transfers/safe callbacks, traits/cap/brake, fresh-user capacity",
    "RewardDistributor": "fundRewards/stake/unstake/claim, time accrual, debt/conservation, taxed tokens/brake/callbacks",
    "SimpleDAO": "propose/execute, secp signatures/quorum/replay, vote/timelock/expiry, mutable voting balance/callback",
    "SimpleMultisig": "native receive/execute, secp sorted quorum/nonce/domain/replay, failed execution/callback",
    "SubscriptionManager": "subscribe/cancel/settleExpired/claimRevenue, dust/reserves/time/brake/callbacks, uint64/uint88 bounds",
    "TokenTimeLock": "lockFor/release, cliff/linear/end, recipient/amount/token/brake, completed-grant reuse",
    "NativeCallback": "test instrument: native/NFT receive failure and callback forwarding/reentry observation",
    "TestToken": "test instrument: ERC20 fee/false-return/transfer callback observation",
    "MarketNFT": "test instrument: ERC721 royalties/ERC165 failures and transfer failures",
    "SignatureCheckerProbe": "test instrument: OZ SignatureChecker (Permit2-style ERC-1271) against a delegated P-256 account",
    "SignedIntentBook": "test instrument: state-changing ERC-1271 consumer (OZ SignatureChecker), relayed owner-key approvals, replay record",
    "BrakeReferenceVault": "reference local deterministic entry brake: deficit/code/unreadable predicate, permissionless latch, entry halted, exit open",
    "SeizableToken": "test instrument: ERC20 with an open issuer seizure to reproduce a vault backing deficit",
}


# Block limits of the new-genesis context (aether_execution::fees and the
# node's block context); a burst block may spend the whole state budget once.
EXEC_LIMIT = 30_000_000
STATE_BURST = 100_000
SUSTAINED_DAILY_STATE = 32 * 86_400
NEW_SLOT_LIMIT = 512
RECEIPT_BYTE_LIMIT = 2_097_152
COMMON_ACTIONS = [
    ("Native transfer, existing recipient", "system/account", "FirstSender/subsequent-nonce-receipt"),
    ("Native transfer, new recipient account", "system/account", "FirstSender/funded-account-growth"),
    ("ERC20 transfer to a new holder", "toolbox/FixedSupplyToken", "transfer funded recipient"),
    ("ERC20 approve", "system/account", "token approval"),
    ("NFT mint (OnchainNFT)", "toolbox/OnchainNFT", "mint second"),
    ("Edition mint (ERC1155)", "toolbox/Editions1155", "mint exact price"),
    ("Airdrop claim (native)", "toolbox/MerkleAirdrop", "airdrop claim inclusive deadline"),
    ("Merkle distributor claim (ERC20)", "core/MerkleDistributor", "distributor/sponsored-claim"),
    ("AMM router exact-input swap", "toolbox/AmmRouter", "router exact input swap"),
    ("Market list (ERC721 escrow)", "toolbox/FixedPriceMarket", "list escrows approved token"),
    ("Market buy with royalty", "toolbox/FixedPriceMarket", "buy five percent royalty"),
    ("Name commit", "core/EastSeaNames", "commit funded bond"),
    ("Name register", "core/EastSeaNames", "register relayer full fee"),
    ("Subscription subscribe", "toolbox/SubscriptionManager", "subscription/expiry flow subscribe"),
    ("Invoice issue", "toolbox/InvoiceBook", "invoice/issue"),
    ("Invoice settle", "toolbox/InvoiceBook", "invoice/settle exact deadline"),
    ("Atomic swap native lock", "core/AtomicSwap", "swap/native-lock"),
    ("Atomic swap token claim", "core/AtomicSwap", "swap/token-claim"),
]


def block_fits(row):
    """Per-block ceiling and the dimension that binds it."""
    caps = {
        "exec": EXEC_LIMIT // max(1, row["exec_gas"]),
        "state": STATE_BURST // max(1, row["state_units"]),
        "bytes": RECEIPT_BYTE_LIMIT // max(1, row["persisted_bytes"]),
    }
    if row["new_slots"]:
        caps["slots"] = NEW_SLOT_LIMIT // row["new_slots"]
    bound = min(caps, key=caps.get)
    return caps[bound], bound


def common_actions(rows):
    lines = ["\nCommon actions (worst measured record of each case). Burst fits/block applies to a block with the full 100,000-unit state budget; sustained/day is the 32-unit-per-height refill (2,764,800 units/day) divided by state units, an upper bound only.\n",
             "| Action | Exec gas | State units | New slots | Persisted bytes | Burst fits/block (binding limit) | Sustained/day ceiling |",
             "|---|---:|---:|---:|---:|---:|---:|"]
    for title, contract, case in COMMON_ACTIONS:
        found = [r for r in rows if r.get("contract") == contract and r.get("case") == case and r.get("success")]
        if not found:
            lines.append(f"| {title} | unmeasured | | | | | |")
            continue
        r = max(found, key=lambda x: (x["state_units"], x["exec_gas"]))
        fits, bound = block_fits(r)
        daily = SUSTAINED_DAILY_STATE // max(1, r["state_units"])
        lines.append(f"| {title} | {r['exec_gas']} | {r['state_units']} | {r['new_slots']} | {r['persisted_bytes']} | {fits} ({bound}) | {daily:,} |")
    return lines


def dbln(wei):
    return f"{int(wei) / 10**18:.6f}"


def workflow_lines(path, verified):
    """Summary table of A0 workflow records (schema eastsea.workflow-record/v1)."""
    records = [json.loads(line) for line in path.read_text().splitlines() if line.strip()] if path and path.exists() else []
    if not records:
        return ["NOT RUN: no workflow records yet."]
    status = "verified complete Rust run" if verified else "NOT VERIFIED (no complete passing Rust transcript)"
    lines = [f"{len(records)} workflow records; status: {status}.\n",
             "| Workflow | Txs (failed) | User tx sigs | Typed sigs | Relayer txs | Cold units | Warm units | Cold floor fee (DBLN) | Failure fees (DBLN) | Warm/day at 10% / 50% / 100% refill (binding) |",
             "|---|---:|---:|---:|---:|---:|---:|---:|---:|---|"]
    for r in records:
        cold, warm, sig = r["totals"]["cold"], r["totals"]["warm"], r["signatures"]
        failure_fee = sum(int(f["fee_paid_wei"]) for f in r["failures"])
        per_day = " / ".join(f"{s['workflows_per_day']:,} ({s['binding_limit']})" for s in r["b5"]["warm"]["sustained_per_day"])
        lines.append(f"| `{r['workflow']}` | {cold['transactions']} ({cold['failed_transactions']}) | {sig['user_transaction']} | {sig['user_typed_message']} | {sig['relayer_transaction']} | {cold['state_units']} | {warm['state_units']} | {dbln(cold['fee_floor_wei'])} | {dbln(failure_fee)} | {per_day} |")
    return lines


def display(values):
    values = sorted(set(int(v) for v in values))
    return str(values[0]) if len(values) == 1 else f"{values[0]}–{values[-1]}"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--metrics", type=Path)
    parser.add_argument("--workflows", type=Path)
    args = parser.parse_args()
    manifest = json.loads(FIXTURES.read_text())
    grouped = defaultdict(list)
    transcript = ""
    verified = False
    if args.metrics:
        for line in args.metrics.read_text().splitlines():
            row = json.loads(line)
            grouped[row["contract"]].append(row)
        log = args.metrics.parent / "contracts-onchain-test.txt"
        transcript = log.read_text() if log.exists() else ""
        verified = bool(re.search(r"test result: ok\. \d+ passed; 0 failed;", transcript))
    tests = sum(len(re.findall(r"#\[test\]", f.read_text())) for f in (ROOT / "crates/contracts-onchain/tests/contracts_onchain").glob("*.rs"))
    lines = [BEGIN, f"Inventory: **{len(manifest)} compiled fixtures; {tests} Rust test functions**. Status is {'verified complete Rust run' if verified else 'NOT RUN / no complete passing Rust transcript'}.\n",
             "Gas, state units, bytes and floor fee below describe successful wallet-budget deployments. Multiple constructor configurations are shown as ranges. Fits/block is a **ceiling** from execution, state, new-slot and logical receipt limits; the independent encoded-payload limit can lower it.\n",
             "| Contract | Implemented cases | Chain result | Deploy exec gas | State units | Persisted bytes | Floor fee (wei) | Fits/block ceiling |",
             "|---|---|---|---:|---:|---:|---:|---:|"]
    for name, artifact in sorted(manifest.items()):
        rows = grouped[name]
        deploys = [r for r in rows if r.get("case") == "deploy/wallet-recommended-budget" and r.get("success")]
        if deploys:
            columns = [display(r[k] for r in deploys) for k in ["exec_gas", "state_units", "persisted_bytes", "floor_fee_wei"]]
            columns.append(display(block_fits(r)[0] for r in deploys))
        else:
            columns = ["unmeasured"] * 5
        cases = SCOPES[name.split("/")[-1]]
        status = f"PASS ({len(rows)} records)" if rows and verified else "NOT RUN" if not rows else "PARTIAL / unverified"
        lines.append(f"| `{name}` | {cases} | {status} | " + " | ".join(columns) + " |")
    all_rows = [r for rs in grouped.values() for r in rs]
    deploys = [r for r in all_rows if r.get("case") == "deploy/wallet-recommended-budget" and r.get("success")]
    if deploys:
        top = max(deploys, key=lambda r: r["state_units"])
        lines.append(f"\nLargest measured deployment: `{top['contract']}` at {top['state_units']:,} state units, {top['state_units'] * 100 / STATE_BURST:.1f}% of the {STATE_BURST:,}-unit burst ({top['exec_gas']:,} exec gas, {top['persisted_bytes']:,} persisted bytes).")
    if verified:
        lines += common_actions(all_rows)
    for capacity in re.findall(r"(?:NFT|AIRDROP)_CAPACITY[^\n]+", transcript):
        lines.append(f"\nMeasured capacity: `{capacity}`.")
    lines += [END]
    report = REPORT.read_text()
    start, stop = report.index(BEGIN), report.index(END) + len(END)
    report = report[:start] + "\n".join(lines) + report[stop:]
    start, stop = report.index(WORKFLOWS_BEGIN), report.index(WORKFLOWS_END) + len(WORKFLOWS_END)
    report = report[:start] + "\n".join([WORKFLOWS_BEGIN, *workflow_lines(args.workflows, verified), WORKFLOWS_END]) + report[stop:]
    REPORT.write_text(report)
    print(f"Updated {REPORT.relative_to(ROOT)}; Rust results verified={verified}")


if __name__ == "__main__":
    main()
