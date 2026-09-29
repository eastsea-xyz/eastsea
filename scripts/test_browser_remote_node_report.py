#!/usr/bin/env python3
"""
scripts/test_browser_remote_node_report.py

검증 스크립트:
브라우저 확장의 로컬 노드 탈피 및 원격 블록체인 노드 통신(2025-2026) 조사 보고서 검증 단위 테스트.
"""

import os
import re
import sys

REPORT_PATH = "docs/research_browser_extension_remote_node_communication_2025_2026.md"

def test_report_completeness():
    print("=== [TC-1] 보고서 구성 및 핵심 주제 완전성 검증 ===")
    assert os.path.exists(REPORT_PATH), f"보고서 파일이 존재하지 않습니다: {REPORT_PATH}"
    
    with open(REPORT_PATH, "r", encoding="utf-8") as f:
        content = f.read()

    assert len(content) > 8000, f"보고서 내용이 너무 짧습니다. (현재 {len(content)}자, 최소 8000자 이상 요구)"

    required_keywords = [
        # 1. WebTransport & SW
        ["WebTransport", "HTTP/3", "Chrome", "Firefox", "Safari", "서비스 워커"],
        # 2. serverCertificateHashes & 14일
        ["serverCertificateHashes", "14일", "ECDSA", "SHA-256", "인증서 롤링"],
        # 3. iroh & libp2p
        ["iroh", "libp2p", "WebRTC", "WebSocket", "릴레이"],
        # 4. MV3 수명 제한
        ["MV3", "서비스 워커", "30초", "Keep-Alive", "Offscreen"],
        # 5. 라이트 클라이언트 사례
        ["Helios", "smoldot", "Lodestar", "라이트 클라이언트", "머클"],
        # 6. Aether 권고안
        ["Aether", "로컬 노드", "권고", "단계적"]
    ]

    for group in required_keywords:
        for kw in group:
            assert kw.lower() in content.lower(), f"필수 키워드 누락: '{kw}'"

    print(" -> TC-1 PASS: 모든 핵심 주제 및 키워드 포함 확인 완료.")
    return content

def test_fact_inference_and_urls(content):
    print("=== [TC-2] 사실/추론 구분 및 URL 출처 검증 ===")
    
    fact_tags = re.findall(r"\[검증된 사실\]", content)
    inference_tags = re.findall(r"\[추론 및 기술적 제언\]", content)
    
    print(f" -> 검출된 [검증된 사실] 태그 수: {len(fact_tags)}개")
    print(f" -> 검출된 [추론 및 기술적 제언] 태그 수: {len(inference_tags)}개")
    
    assert len(fact_tags) >= 5, f"[검증된 사실] 태그 수가 부족합니다: {len(fact_tags)}"
    assert len(inference_tags) >= 5, f"[추론 및 기술적 제언] 태그 수가 부족합니다: {len(inference_tags)}"

    urls = re.findall(r"https?://[^\s\)\>\]]+", content)
    unique_urls = set(urls)
    print(f" -> 검출된 고유 URL 출처 수: {len(unique_urls)}개")
    assert len(unique_urls) >= 20, f"신뢰할 수 있는 출처 URL 수가 부족합니다 (현재 {len(unique_urls)}개, 최소 20개 필요)"

    print(" -> TC-2 PASS: 사실/추론 태깅 및 충분한 고유 공식 출처 URL 확인 완료.")

def test_five_stage_framework(content):
    print("=== [TC-3] 시스템 프롬프트 5대 필수 분석 프레임워크 검증 ===")
    
    # 1. 목표 한 문장 요약 -> 계획, 추론, 검증
    assert "목표 한 문장 요약" in content or "목표 요약" in content, "1단계 '목표 한 문장 요약' 누락"
    assert "계획·추론·검증" in content or ("계획" in content and "추론" in content and "검증" in content), "1단계 3단계 프로세스 누락"
    
    # 2. 브레인스토밍 >=3안, 장단점 표, 내부 투표
    assert "브레인스토밍" in content, "2단계 브레인스토밍 누락"
    assert "| 장점 | 단점 |" in content or "| 장·단점 |" in content or ("| 장점 |" in content and "| 단점 |" in content), "2단계 장단점 표 누락"
    assert "내부 투표" in content or "선택 및 근거" in content, "2단계 내부 투표 누락"

    # 3. TAO 루프
    assert "TAO" in content or "Thought-Action-Observation" in content, "3단계 TAO 루프 누락"

    # 4. 그래프 분해
    assert "그래프 분해" in content, "4단계 그래프 분해 누락"

    # 5. 다섯 가지 이상 풀이 -> 자기-일관성 투표
    assert "다섯 가지" in content or "풀이 1" in content or "풀이 5" in content or "안 5" in content, "5단계 다각도 풀이 누락"
    assert "자기-일관성" in content or "Self-Consistency" in content, "5단계 자기-일관성 투표 누락"

    print(" -> TC-3 PASS: 시스템 프롬프트 5대 분석 프레임워크 완비 확인 완료.")

def main():
    try:
        content = test_report_completeness()
        test_fact_inference_and_urls(content)
        test_five_stage_framework(content)
        print("\n==========================================")
        print("  ALL UNIT TESTS PASSED SUCCESSFULLY! ")
        print("==========================================")
    except AssertionError as e:
        print(f"\n[FAIL] 검증 실패: {e}", file=sys.stderr)
        sys.exit(1)

if __name__ == "__main__":
    main()
