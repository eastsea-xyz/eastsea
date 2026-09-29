#!/usr/bin/env python3
"""
Unit test for verifying the Bonding Curve Launchpad Security & Architecture Research Report (2024-2026).
Verifies:
1. Target platforms: pump.fun, four.meme, Clanker, Moonshot, Believe, letsbonk.
2. 5 Major Attacks & Scale: Sniping, Bundling, Rug pulls, Post-graduation dumping, Sandwich attacks.
3. 5 Platform Defenses & Real Efficacy: Initial N-block limit, Creator buy cap & lock, Fee-based snipe tax, Fair launch (Batch Auction), Bundle detection.
4. Target System Constraints: Admin-less immutable contract, Zero fees, No trending/ranking, FOCIL-enforced chain.
5. Actionable Recommendations: Sybil-resistant on-chain guards, Sybil-vulnerable guards, Concrete parameters (Creator cap %, Initial blocks, Limits).
6. Explicit demarcation: [검증된 사실] and [추론 및 기술적 제언] (or [추론 및 분석]).
7. Authoritative source URLs (>= 12 URLs).
"""

import sys
import os
import re

def verify_launchpad_report(report_path: str):
    if not os.path.exists(report_path):
        print(f"FAIL: Report file does not exist at {report_path}")
        sys.exit(1)
        
    with open(report_path, "r", encoding="utf-8") as f:
        report_text = f.read()

    issues = []

    # 1. Target Platforms
    platforms = [
        ("pump.fun", "pump.fun coverage missing"),
        ("four.meme", "four.meme coverage missing"),
        ("Clanker", "Clanker coverage missing"),
        ("Moonshot", "Moonshot coverage missing"),
        ("Believe", "Believe coverage missing"),
        ("letsbonk", "letsbonk coverage missing"),
    ]
    for name, err in platforms:
        if name.lower() not in report_text.lower():
            issues.append(f"Platform: {err} (name: '{name}')")

    # 2. 5 Major Attacks & Scale
    attacks = [
        ("스나이핑", "Sniping attack coverage missing"),
        ("번들", "Bundling coverage missing"),
        ("러그풀", "Rug pull coverage missing"),
        ("졸업", "Post-graduation dumping coverage missing"),
        ("샌드위치", "Sandwich attack coverage missing"),
    ]
    for kw, err in attacks:
        if kw not in report_text:
            issues.append(f"Attack: {err} (keyword: '{kw}')")

    # 3. 5 Platform Defenses & Efficacy
    defenses = [
        ("지갑당", "Wallet limit defense analysis missing"),
        ("창업자", "Creator buy cap & lock analysis missing"),
        ("세금", "Fee-based snipe tax analysis missing"),
        ("공정 출시", "Fair launch / Batch Auction analysis missing"),
        ("번들 탐지", "Bundle detection analysis missing"),
    ]
    for kw, err in defenses:
        if kw not in report_text:
            issues.append(f"Defense: {err} (keyword: '{kw}')")

    # 4. Target System Constraints
    constraints = [
        ("불변", "Immutable contract analysis missing"),
        ("수수료 0", "Zero fee constraint analysis missing"),
        ("FOCIL", "FOCIL block ordering analysis missing"),
        ("추천", "No recommendation/trending constraint analysis missing"),
    ]
    for kw, err in constraints:
        if kw not in report_text:
            issues.append(f"Constraint: {err} (keyword: '{kw}')")

    # 5. Sybil bypass & Parameter Recommendations
    recommendations = [
        ("시빌", "Sybil bypass analysis missing"),
        ("창업자 상한", "Creator cap recommendation missing"),
        ("초기 블록", "Initial block count recommendation missing"),
        ("한도", "Inflow limit recommendation missing"),
    ]
    for kw, err in recommendations:
        if kw not in report_text:
            issues.append(f"Recommendation: {err} (keyword: '{kw}')")

    # 6. Demarcation between Facts and Inferences
    fact_tags = re.findall(r"\[검증된 사실\]", report_text)
    infer_tags = re.findall(r"\[추론 및 (?:분석|기술적 제언)\]", report_text)
    
    if len(fact_tags) < 5:
        issues.append(f"Fact demarcation: Expected >= 5 '[검증된 사실]' tags, found {len(fact_tags)}")
    if len(infer_tags) < 5:
        issues.append(f"Inference demarcation: Expected >= 5 '[추론 및 기술적 제언/분석]' tags, found {len(infer_tags)}")

    # 7. URLs Check
    urls = re.findall(r"https?://[^\s\)\>\]]+", report_text)
    if len(urls) < 12:
        issues.append(f"Source URLs: Expected >= 12 URLs, found {len(urls)}")

    # Summary
    if issues:
        print("FAIL: Verification failed with the following issues:")
        for idx, issue in enumerate(issues, 1):
            print(f"  {idx}. {issue}")
        sys.exit(1)
    else:
        print(f"SUCCESS: Report verified successfully!")
        print(f"  - Length: {len(report_text)} characters")
        print(f"  - Verified Facts Tags: {len(fact_tags)}")
        print(f"  - Inferences Tags: {len(infer_tags)}")
        print(f"  - Source URLs: {len(urls)}")
        sys.exit(0)

if __name__ == "__main__":
    target = sys.argv[1] if len(sys.argv) > 1 else "docs/research_bonding_curve_launchpad_attacks_and_defenses.md"
    verify_launchpad_report(target)
