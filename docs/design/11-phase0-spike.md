# 11. 0단계 보안 정리 + 0.5단계 스파이크 작업 목록

## 0단계 (1주) — 기존 코드, 설계와 무관하게 필요

| # | 작업 | 파일 | 완료 기준 |
|---|---|---|---|
| 0.1 | 바인딩 127.0.0.1 기본, `--public` 플래그 | src/bin/node.rs | 외부 IP 접속 불가 |
| 0.2 | `/api/p2p/gossip`, `/api/p2p/sync`의 상태 적용 제거 (조회 전용) | node.rs | 원격 상태 변경 0건 |
| 0.3 | 로컬 API 토큰 + Origin/Host 검사, CORS `*` 제거 | node.rs, dashboard.html | 토큰 없는 POST 401 |
| 0.4 | 대시보드 `innerHTML` → `textContent` (사용자 데이터 경로 전부) | dashboard.html | XSS 페이로드 무해 |
| 0.5 | 하드코딩 TPS·가짜 `/api/mev_attack`·개인 IP·미사용 revm 제거 | node.rs, p2p.rs, peers.json, Cargo.toml | grep 0건 |
| 0.6 | README 수치 삭제·재분류("시뮬레이션"/"로드맵"), "배당" 표현 삭제 | README.md | 근거 없는 수치 0개 |
| 0.7 | `install.sh`의 `xattr -cr` 제거, 노터라이즈 확인으로 대체 | install.sh | |
| 0.8 | dist/ 바이너리 git 제거, .gitignore | | |
| 0.9 | 보안 테스트 5종 추가 (10장) | tests/ | 통과 |
| 0.10 | 커밋: `fix(security): …`, `docs: …` | | |

## 0.5단계 스파이크 (2주) — 실측

환경: M1 Max 64GB(로컬), poc-m3 24GB, poc-cuda RTX 5080(비교).

| # | 실측 | 산출물 | 판정 |
|---|---|---|---|
| S1a | Lattice Jolt `feat/akita-metal` 빌드, 고정 RISC-V 프로그램(예: 1M cycle 루프) cycles/s | 표 | >1,000만 재현? |
| S1b | Stwo M31 동일 프로그램 (Cairo 또는 RISC-V 게스트 가능 여부 확인) | 표 | |
| S2 | RISC Zero 3.x macOS 빌드, Metal 실제 사용 여부(GPU 사용률) | 결론 | deprecated 의미 |
| S3 | 각 백엔드로 revm 게스트(송금 100건 블록) 증명: 시간, 피크 메모리, 증명 크기 | 표 | 15분 이내? |
| S4 | 블록 크기 100/500/1,000 tx 스케일링 | 곡선 | 맥 1대 TPS 상한 |
| S5 | 프로파일: NTT/해시/sumcheck/witness 비율 | 표 | 커널 우선순위 |
| S6 | 5080 CUDA 동일 측정 | 표 | 격차 |
| S7 | 검증 시간 (M1, wasm) | 표 | ≤100ms? |
| S8 | reth SDK 최소 노드 vs Commonware+grevm 스켈레톤: 바이너리 크기, RSS | 표 | S3 결정 |
| S9 | NOMT macOS 커밋/s, multiproof 크기 (Poseidon2 vs BLAKE3) | 표 | |
| S10 | iroh-blobs vs librqbit 2GiB 청크 전송 (LAN·Tailscale) | 표 | S6 결정 |

산출물: `docs/research/spike-2026-10.md` + `benches/spike/` 재현 스크립트 + CodSpeed 기준선.

## 판정 규칙

- S3 ≤ 15분 & S7 ≤ 100ms → 1단계 진행, S1 결과로 백엔드 확정.
- S3 > 15분 → 블록 축소(S4)로 재측정. 그래도 > 15분이면 Metal 커널 개발을 1단계 첫 항목으로.
- S1a 미재현 & S1b 양호 → Stwo 1순위.
- 모두 > 60분 → "맥용" 정체성 재검토 회의.

## 1단계 착수 조건

- 0단계 커밋 완료
- 스파이크 리포트 공개
- 03의 trait 시그니처를 실제 코드로 확정(빈 구현 + 테스트 스텁)
