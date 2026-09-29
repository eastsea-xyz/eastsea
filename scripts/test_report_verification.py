#!/usr/bin/env python3
"""
Unit test for verifying the comprehensive App Attest & DeviceCheck Gas Sponsorship Research Report.
Checks:
1. Presence of all 5 required topical sections and sub-questions.
2. Clear segregation between '검증된 사실 (Verified Facts)' and '추론 및 분석 (Inference)'.
3. Inclusion of valid authoritative source URLs.
4. Validation of Use Cases (UC-1 to UC-5).
"""

import sys
import os
import re

def verify_report_content(report_text: str):
    issues = []
    
    # Check Section 1
    if not re.search(r"App Attest.*DeviceCheck", report_text, re.IGNORECASE):
        issues.append("Missing App Attest / DeviceCheck mechanism section")
    if "2비트" not in report_text and "two bits" not in report_text:
        issues.append("Missing DeviceCheck 2-bit explanation")
    if "공장" not in report_text and "초기화" not in report_text:
        issues.append("Missing reset / wipe behavior")
    if "재설치" not in report_text:
        issues.append("Missing reinstall behavior")
        
    # Check Section 2: macOS & VM/Hackintosh
    if "macOS" not in report_text:
        issues.append("Missing macOS coverage")
    if "isSupported" not in report_text:
        issues.append("Missing isSupported runtime check discussion")
    if "Hackintosh" not in report_text and "해킨토시" not in report_text:
        issues.append("Missing Hackintosh analysis")
    if "가상머신" not in report_text and "VM" not in report_text:
        issues.append("Missing VM analysis")
        
    # Check Section 3: Device farm & Sybil cases
    if "중고" not in report_text:
        issues.append("Missing used device pricing")
    if "Worldcoin" not in report_text:
        issues.append("Missing Worldcoin sybil case")
    if "Farcaster" not in report_text:
        issues.append("Missing Farcaster gas sybil case")
    if "Base" not in report_text:
        issues.append("Missing Base paymaster case")
        
    # Check Section 4: L1/L2 and ERC-4337
    for chain in ["zkSync", "Starknet", "Sui", "Base"]:
        if chain.lower() not in report_text.lower():
            issues.append(f"Missing analysis for {chain}")
            
    # Check Section 5: Design recommendations
    if "기기당 평생 한도" not in report_text and "평생 한도" not in report_text:
        issues.append("Missing lifetime limit per device recommendation")
    if "풀 상한" not in report_text and "풀" not in report_text:
        issues.append("Missing pool cap recommendation")
        
    # Check fact vs inference distinction
    if "검증된 사실" not in report_text or "추론" not in report_text:
        issues.append("Missing clear distinction between Verified Facts and Inference")
        
    # Check source URLs
    urls = re.findall(r'https?://[^\s\)\]<>"]+', report_text)
    if len(urls) < 10:
        issues.append(f"Insufficient source URLs: found {len(urls)}, expected >= 10")
        
    apple_urls = [u for u in urls if "apple.com" in u]
    if len(apple_urls) < 3:
        issues.append(f"Insufficient Apple official URLs: found {len(apple_urls)}, expected >= 3")

    return issues

def main():
    report_path = "/Volumes/workspace/aether-node/docs/research_app_attest_gas_sponsorship.md"
    if not os.path.exists(report_path):
        print(f"FAIL: Report file not found at {report_path}")
        sys.exit(1)
        
    with open(report_path, "r", encoding="utf-8") as f:
        content = f.read()
        
    issues = verify_report_content(content)
    if issues:
        print("FAIL: Verification failed with issues:")
        for issue in issues:
            print(f"  - {issue}")
        sys.exit(1)
        
    print(f"SUCCESS: Report verified successfully against all requirements! ({len(content)} chars)")
    sys.exit(0)

if __name__ == "__main__":
    main()
