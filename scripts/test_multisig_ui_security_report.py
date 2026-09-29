#!/usr/bin/env python3
"""
Unit test for Multisig & Token Lock/Claim UI Security Research Report.
Validates report structure, historical incidents, technical countermeasures,
fact/inference separation, and citation URLs.
"""

import os
import re
import sys

REPORT_PATH = os.path.join(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
    "docs",
    "research_multisig_ui_security_and_vault_guidelines.md"
)

def test_report_exists_and_length():
    assert os.path.exists(REPORT_PATH), f"Report not found at {REPORT_PATH}"
    with open(REPORT_PATH, "r", encoding="utf-8") as f:
        content = f.read()
    assert len(content) > 7000, f"Report too short: {len(content)} chars (expected > 7000)"
    return content

def test_required_incidents(content: str):
    incidents = [
        ("Bybit-Safe 2025 Incident", [r"Bybit", r"Safe", r"delegatecall"]),
        ("Radiant Capital 2024 Incident", [r"Radiant", r"2024"]),
        ("WazirX 2024 Incident", [r"WazirX", r"Liminal"]),
        ("Squads Solana Security", [r"Squads", r"Solana"]),
        ("Sui / Aptos Move Multisig", [r"(Sui|Aptos)", r"Move|MSafe"]),
        ("Hardware Wallet Blind Signing", [r"(블라인드\s*서명|Blind\s*Signing)", r"(하드웨어|Ledger|Trezor)"])
    ]
    for name, patterns in incidents:
        for p in patterns:
            assert re.search(p, content, re.IGNORECASE), f"Missing pattern '{p}' for {name}"
    print("✅ All required security incidents verified.")

def test_technical_solutions(content: str):
    solutions = [
        ("Clear Signing / EIP-712 / ERC-7730", [r"EIP-712", r"(ERC-7730|Clear\s*Signing|명확\s*서명)"]),
        ("Transaction Simulation", [r"(트랜잭션|거래)\s*시뮬레이션|Transaction\s*Simulation"]),
        ("delegatecall Mitigation", [r"delegatecall"]),
        ("Independent Verification Path", [r"(독립.*검증|Out-of-band|대역\s*외)"])
    ]
    for name, patterns in solutions:
        for p in patterns:
            assert re.search(p, content, re.IGNORECASE), f"Missing pattern '{p}' for {name}"
    print("✅ All technical solutions verified.")

def test_aether_recommendations(content: str):
    components = [
        ("AetherVault", [r"AetherVault", r"(지연|delay|한도|limit|M-of-N|소유자)"]),
        ("TokenLocker", [r"TokenLocker", r"(unlockAt|락업|단축\s*불가|beneficiary)"]),
        ("MerkleDistributor", [r"MerkleDistributor", r"(머클|merkleRoot|claim|증명)"]),
        ("macOS / iOS UI", [r"(macOS|iOS|SwiftUI|Secure\s*Enclave|P-256)"])
    ]
    for name, patterns in components:
        for p in patterns:
            assert re.search(p, content, re.IGNORECASE), f"Missing pattern '{p}' for {name}"
    print("✅ All Aether ecosystem recommendations verified.")

def test_fact_inference_separation(content: str):
    facts = re.findall(r"\[검증된\s*사실\]", content)
    inferences = re.findall(r"\[추론.*\]", content)
    assert len(facts) >= 5, f"Expected >= 5 [검증된 사실], found {len(facts)}"
    assert len(inferences) >= 5, f"Expected >= 5 [추론...], found {len(inferences)}"
    print(f"✅ Fact/Inference separation verified: {len(facts)} facts, {len(inferences)} inferences.")

def test_citation_urls(content: str):
    urls = re.findall(r"https?://[^\s\)\>\]]+", content)
    unique_urls = set(urls)
    assert len(unique_urls) >= 12, f"Expected >= 12 unique URLs, found {len(unique_urls)}: {unique_urls}"
    print(f"✅ Citation URLs verified: {len(unique_urls)} unique URLs found.")

def main():
    print("Starting verification of Multisig UI Security Report...")
    content = test_report_exists_and_length()
    test_required_incidents(content)
    test_technical_solutions(content)
    test_aether_recommendations(content)
    test_fact_inference_separation(content)
    test_citation_urls(content)
    print("🎉 All tests passed successfully!")

if __name__ == "__main__":
    main()
