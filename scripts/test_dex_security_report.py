#!/usr/bin/env python3
"""
Unit test for verifying the comprehensive Immutable DEX Security Research Report (2023-2026).
Verifies:
1. Section 1: Known Vulnerabilities & Incidents (Uniswap v2/v3/v4 hooks, Curve Vyper, Balancer 2025 rounding,
   Velodrome/Aerodrome Slipstream, Trader Joe LB, FOT tokens, Rebase tokens, ERC-777, First LP inflation, TWAP manipulation).
2. Section 2: Sandwich & MEV in Proposer-Enforced Ordering Chains (FOCIL, residual MEV, slippage defaults, deadline anti-patterns).
3. Section 3: Lessons from Operating Immutable AMMs without Admin Key / Pause / Fee Switch.
4. Section 4: Actionable Recommendation Checklist (Invariant fuzzing, Differential testing, Audit priority, Wallet UI safeguards).
5. Explicit demarcation between '검증된 사실' and '추론 및 분석'.
6. Authoritative Source URLs (>= 15 URLs).
"""

import sys
import os
import re

def verify_dex_report(report_text: str):
    issues = []
    
    # 1. Check Section 1 Incidents & Protocols
    required_keywords_sec1 = [
        ("Uniswap", "Missing Uniswap coverage"),
        ("v4", "Missing Uniswap v4 coverage"),
        ("Hook", "Missing Uniswap v4 Hook coverage"),
        ("Curve", "Missing Curve coverage"),
        ("Vyper", "Missing Vyper compiler bug coverage"),
        ("Balancer", "Missing Balancer coverage"),
        ("2025", "Missing Balancer 2025 rounding incident coverage"),
        ("ComposableStablePool", "Missing Balancer ComposableStablePool coverage"),
        ("KyberSwap", "Missing KyberSwap coverage"),
        ("Velodrome", "Missing Velodrome / Aerodrome coverage"),
        ("Slipstream", "Missing Slipstream coverage"),
        ("Trader Joe", "Missing Trader Joe coverage"),
        ("Liquidity Book", "Missing Liquidity Book coverage"),
        ("전송 수수료", "Missing Fee-on-transfer (FOT) token coverage"),
        ("리베이스", "Missing Rebase token coverage"),
        ("ERC-777", "Missing ERC-777 / callback hook coverage"),
        ("인플레이션", "Missing First LP inflation attack coverage"),
        ("TWAP", "Missing TWAP oracle manipulation coverage"),
    ]
    for kw, err in required_keywords_sec1:
        if kw.lower() not in report_text.lower():
            issues.append(f"Section 1: {err} (keyword: '{kw}')")

    # 2. Check Section 2 MEV & Ordering
    required_keywords_sec2 = [
        ("FOCIL", "Missing FOCIL coverage"),
        ("샌드위치", "Missing Sandwich attack coverage"),
        ("슬리피지", "Missing Slippage default analysis"),
        ("마감 시간", "Missing Deadline parameter analysis"),
        ("block.timestamp", "Missing block.timestamp deadline vulnerability analysis"),
    ]
    for kw, err in required_keywords_sec2:
        if kw.lower() not in report_text.lower():
            issues.append(f"Section 2: {err} (keyword: '{kw}')")

    # 3. Check Section 3 Immutable AMM Operations
    required_keywords_sec3 = [
        ("수수료 스위치", "Missing Fee switch analysis"),
        ("긴급 정지", "Missing Emergency pause (Pause) analysis"),
        ("불변", "Missing Immutability analysis"),
        ("거버넌스", "Missing Governance risk analysis"),
    ]
    for kw, err in required_keywords_sec3:
        if kw.lower() not in report_text.lower():
            issues.append(f"Section 3: {err} (keyword: '{kw}')")

    # 4. Check Section 4 Recommendation Checklist
    required_keywords_sec4 = [
        ("불변식", "Missing Invariant fuzzing coverage"),
        ("차등 테스트", "Missing Differential testing coverage"),
        ("감사 우선순위", "Missing Audit priority matrix"),
        ("지갑", "Missing Wallet UI protections"),
        ("시뮬레이션", "Missing Simulation safeguard recommendation"),
    ]
    for kw, err in required_keywords_sec4:
        if kw.lower() not in report_text.lower():
            issues.append(f"Section 4: {err} (keyword: '{kw}')")

    # 5. Check Fact vs Inference demarcation
    facts_count = len(re.findall(r"\[검증된 사실\]|### 검증된 사실", report_text))
    inferences_count = len(re.findall(r"\[추론 및 분석\]|### 추론 및 분석|\[설계적 추론\]", report_text))
    if facts_count < 4:
        issues.append(f"Insufficient '검증된 사실' markers: found {facts_count}, expected >= 4")
    if inferences_count < 4:
        issues.append(f"Insufficient '추론 및 분석' markers: found {inferences_count}, expected >= 4")

    # 6. Check URLs
    urls = re.findall(r'https?://[^\s\)\]<>"]+', report_text)
    if len(urls) < 15:
        issues.append(f"Insufficient source URLs: found {len(urls)}, expected >= 15")

    return issues

def main():
    report_path = "/Volumes/workspace/aether-node/docs/research_immutable_dex_security.md"
    if not os.path.exists(report_path):
        print(f"FAIL: Report file not found at {report_path}")
        sys.exit(1)
        
    with open(report_path, "r", encoding="utf-8") as f:
        content = f.read()
        
    issues = verify_dex_report(content)
    if issues:
        print("FAIL: Verification failed with issues:")
        for issue in issues:
            print(f"  - {issue}")
        sys.exit(1)
        
    url_count = len(re.findall(r'https?://[^\s\)\]<>"]+', content))
    print(f"SUCCESS: Report verified successfully against all requirements! ({len(content)} chars, {url_count} source URLs)")
    sys.exit(0)

if __name__ == "__main__":
    main()
