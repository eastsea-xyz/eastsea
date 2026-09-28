# 합의 복구: 인증된 뷰에서 체인이 멈출 때

## 증상

- 높이가 멈추고, 모든 검증자 로그에 `missing parent during proposal view=V+1 missing=V`가 반복된다.
- 뷰 V는 인증(notarize)됐다. 하지만 일부 검증자는 V를 확정하자고 투표했고, 나머지는 V를 건너뛰자고(nullify) 투표했다. 어느 쪽도 정족수(4명 중 3명)를 채우지 못한다.
- 2026-09-28 테스트넷(체인 7780)이 이렇게 멈췄다. 높이 69651, 뷰 69842였다.

`missing parent`는 "블록이 없다"는 뜻이 아니다. simplex가 V+1의 부모를 고를 때 V가 인증 확인(certify)되지도, 건너뛰어지지도 않았다는 뜻이다.

## 원인 (2026-09-28 조사)

**인증 확인 결과가 검증자마다 달랐다.** 블록은 없어지지 않았다.

- 우리는 marshal의 `Deferred` 어댑터를 썼다. `Deferred`는 블록이 도착했는지만 보고 notarize에 투표한다. `Application::verify`(실행, FOCIL 포함 목록 규칙)는 그 뒤 **certify**의 판정으로 쓰인다.
- Commonware 계약: certify 판정은 모든 정직한 검증자에게 **같아야** 한다(`CertifiableAutomaton::certify`, "Determinism Requirement"). 다르면 liveness가 깨진다.
- FOCIL 규칙은 각 검증자가 **자기가 본** 포함 목록으로 판단한다(목록을 받은 시각, 750 ms 대기). 검증자마다 결과가 다를 수 있다.
- simplex에서 finalize에 투표한 검증자는 같은 뷰에 nullify할 수 없다. 그래서 2 대 2로 갈리면 영영 풀리지 않는다.

로그 근거(`~/aether-testnet/node{1..4}.log`):

| 시각 (UTC) | 검증자 | 로그 |
|---|---|---|
| 02:50:37.147 | 4 | `proposed height=69652 txs=2` (뷰 69842의 제안자) |
| 02:50:38.354 | 1 | `published inclusion list height=69651 txs=5` (새 거래가 목록에 오름) |
| 02:50:39.315 | 3 | `inclusion list violated; not voting height=69652 missing=1 first=0x1a37…` → `proposal failed certification view=69842` |
| 02:50:39.562 | 1 | 같은 거래로 같은 경고 → `proposal failed certification view=69842` |
| — | 2, 4 | 경고 없음: certify 통과, finalize 투표 |

- 1·3은 nullify, 2·4는 finalize. 정족수 3을 채울 수 없다.
- 재시작 뒤에도 검증자 1 로그에 `notarized block covered by verified write round=69842`가 찍혔다. **블록은 디스크에 있었다.** resolver의 `serve failed`는 부수 현상이다.
- 디스크가 가득 찬 것, 프로토콜 3 롤링 재시작, 버퍼 방출, marshal 캐시 정리, 파티션 이름은 원인이 아니었다. 멈춘 뒤 03:06의 재시작은 네 노드가 동시에 한 것이다.

## 수정 (로컬 투표 규칙, 합의 규칙은 그대로)

`crates/node/src/engine.rs`, `crates/node/src/voting.rs`:

1. **`Deferred` → `Inline`.** `Application::verify`(실행 + FOCIL)가 notarize 투표를 결정한다. certify는 블록이 디스크에 있는지만 기다린다. 모든 검증자가 같게 판단한다.
   - FOCIL로 거절된 블록은 notarize 정족수를 못 얻고 뷰가 넘어간다. 원래 설계(`inclusion.rs`: "refuse to notarize")대로다.
   - 3명이 notarize하면 그 3명이 모두 certify하므로 확정된다.
2. **블록이 디스크에 있어야 notarize.** `DurableVote`가 `true` 판정을 marshal이 블록을 동기화할 때까지 붙잡는다. 인증된 블록은 적어도 f+1명의 정직한 검증자 디스크에 있다. 충돌이나 재시작 뒤에도 가져올 수 있다.

블록·투표·인증서 형식은 바뀌지 않는다. 언제 투표하는지만 바뀐다. 그래서 테스트넷 7780의 기존 검증자와 섞여도 된다. 다만 옛 바이너리 검증자는 여전히 certify를 다르게 판단할 수 있다. **네 검증자를 모두 올린다.**

Commonware 버그는 아니다. `Deferred`의 계약(certify는 결정적이어야 함)을 우리가 어겼다. 업스트림에 낼 이슈는 없다.

## 시험 (`crates/node/tests/sim.rs`, 결정적 시뮬레이션)

| 시험 | 내용 | 수정 전 | 수정 후 |
|---|---|---|---|
| `split_inclusion_list_does_not_stall_the_chain` | 검증자 1·3만 아는 포함 목록 거래 (09-28 재현) | 높이 10에서 영구 정지 | 계속 확정, 90초에 높이 80 |
| `one_slow_disk_does_not_stop_the_chain` | 검증자 4의 쓰기·동기화마다 400 ms | — | 거의 제 속도, 최대 간격 3초 |
| `stalled_disks_stall_then_resume_safely` | 검증자 3·4의 쓰기·동기화마다 30초 (정족수 없음) | — | 멈췄다가 디스크가 풀리면 스스로 재개, 안전성 유지 |

디스크 지연은 `crates/node/tests/slow_disk/mod.rs`가 넣는다. 결정적 런타임을 감싸 검증자별로 blob 쓰기·크기 변경·동기화를 늦춘다.

## 이미 멈췄다면: 복구 절차

수정된 바이너리에서는 이 멈춤이 생기지 않아야 한다. 옛 바이너리에서 멈췄거나 다른 원인으로 같은 증상이 나면 이렇게 한다.

투표 기록(`aether-consensus` 파티션)만 지우고 재시작하면 안 된다. simplex가 제네시스 바닥(뷰 0)에서 다시 시작해 이미 확정된 블록과 어긋날 수 있다.

1. 모든 검증자에서 마지막으로 저장된 확정 인증서의 뷰와 높이를 확인한다. 모두 같아야 한다.
2. 모든 검증자 설정에 `AETHER_RECOVER_CONSENSUS=<뷰>@<높이>`를 넣는다. 예: `69841@69651`.
3. 모든 검증자를 **함께** 재시작한다. 첫 재시작에서 새 투표 기록(`aether-consensus-r<뷰>`)이 만들어진다.
4. 높이가 늘어나는지 확인한다.
5. 설정은 **그대로 둔다.** 이후 재시작도 같은 바닥과 같은 투표 기록을 다시 쓰므로, 한 뷰에 두 번 투표하지 않는다.
6. 위원회 에포크가 바뀌면 이 설정은 자동으로 무시된다. 그 뒤에 지워도 된다.

| | 결과 |
|---|---|
| 확정된 블록 | 그대로 |
| 인증만 되고 확정되지 않은 블록(V) | 버려지고, 같은 높이에 새 블록이 확정된다 |
| 거래 | 확정되지 않은 거래는 멤풀에서 다시 들어간다 |

- 설정한 높이의 인증서가 다른 뷰에 있으면 노드가 시작을 거부한다(panic).
- 설정한 높이에 인증서가 없어도 시작을 거부한다.

## 남은 위험

- notarize가 실행을 기다린다. 뷰 지연이 블록 실행 시간만큼 늘어난다. 증명 검증이 느리면 리더 타임아웃에 걸릴 수 있다.
- notarize가 디스크 동기화를 기다린다. 디스크가 느린 검증자는 투표가 늦다. 절반 이상이 느리면 체인도 그만큼 느려진다(시험으로 확인: 멈췄다가 재개).
- 제안자 자신의 블록은 여전히 보낸 뒤에 저장한다(Commonware 방식). 다른 투표자들이 디스크에 저장한 뒤 투표하므로, 인증된 블록은 여전히 f+1명의 정직한 검증자에게 있다.
- FOCIL 판정은 여전히 검증자마다 다를 수 있다. 이제는 그 블록이 정족수를 못 얻고 뷰가 넘어갈 뿐이다. 제안자 절반이 계속 거절당하면 블록 간격이 길어진다(시험에서 최대 17초).
