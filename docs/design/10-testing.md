# 10. 검증 체계

원칙 3. 기능보다 먼저 만든다.

## 차등 테스트 (`tests/differential/`)

- 생성기: proptest로 계정 N, tx M, 워크로드 혼합(송금/swap/배포/호출/7702 위임) 무작위.
- 오라클: (a) 순차 revm 실행, (b) grevm 2.1 (dev-dep, revm 40 → 결과 root만 비교), (c) 이더리움 메인넷 블록(grevm 테스트 데이터).
- 불변식:
  - `root(seq) == root(blockstm_dynamic) == root(blockstm_static_dag)`
  - 총 잔액 보존 (수수료 소각 포함 회계)
  - 같은 입력 1,000회 → root 동일 (결정성)
  - 생산자 BAL == 검증자 재수집 BAL
  - 게스트 실행 root == 호스트 실행 root
- 케이스 수: CI 1만, 야간 10만.

## 상태 트리 테스트

- 참조 구현(단순 재귀 SMT) vs NOMT root 일치.
- multiproof 검증: 무작위 키 집합, 위조 형제 삽입 시 실패.
- 해시 백엔드 두 개 모두.

## 증명 테스트

- 게스트 단위: 고정 블록 → ChunkProof → verify == true; io 변조 시 false.
- 집계: 청크 2/4/8 → BlockProof; 청크 연결 위조 시 실패.
- 크기·시간 게이트: 증명 ≤300KiB, 검증 ≤100ms(M1) — CodSpeed 회귀.

## 합의 시뮬레이션 (`crates/node/tests/sim.rs`)

- Commonware deterministic runtime: 시드 고정, 네트워크 지연·분할·드롭 주입.
- 시나리오: 위원회 1/3 오프라인, 생산자 침묵, 이중 서명 투표, 시계 오차, 분할 후 병합, 인계 대기 중 크래시 재시작, 포함 목록 누락 블록. 인증서-증명 불일치 주입은 예정.
- Quint 트레이스 재생: `specs/consensus.qnt` → 트레이스 → Rust 상태 기계 일치.

## 네트워크 테스트

- turmoil로 iroh 연결·재연결, 미러 다운로더 fallback, Pkarr 갱신.

## 보안 테스트 (0단계부터)

- 외부 IP에서 상태 변경 API 호출 → 전부 거부.
- gossip으로 임의 tx/블록 주입 → 서명·BAL 검증 실패로 거부.
- 대시보드 XSS 페이로드(계약 이름) → 렌더링 텍스트로만.
- 퍼징: 봉투·블록·매니페스트 파서 (cargo-fuzz).

## 벤치 회귀 (CodSpeed + Bencher)

- 실행: TPS by (계정, 스레드, 워크로드).
- 증명: cycles/s, 청크 시간, 집계 시간.
- 상태: NOMT 커밋/s, multiproof 크기.
- 메모리: 역할별 RSS (2주 가동 곡선은 야간 잡).
- 회귀 10% 초과 시 PR 실패. CPU 명령어 수 기준(CodSpeed)이라 공유 러너 노이즈 무관.

## 커버리지

- 라인 80% 이상(cargo-llvm-cov). `guest/`는 호스트 실행으로 측정.

## 정직성 규칙

- README·문서의 모든 수치는 `benches/` 재현 명령과 CodSpeed 링크를 가진다.
- 정확성 미통과 구현의 성능 수치는 어디에도 쓰지 않는다.
