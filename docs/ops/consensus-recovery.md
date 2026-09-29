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
- **첫 복구는 마지막 확정 높이만:** 그 뷰의 투표 기록(`aether-consensus-r<뷰>`)이 아직 없는 시작, 즉 복구 자체라면 `<높이>`가 이 노드가 저장한 마지막 확정 높이와 같아야 시작한다. 다르면 거부한다: 더 낮은 높이에서 시작하면 다른 검증자가 보관한 확정을 스스로 버리는 포크가 된다(1단계에서 모두 같은 값을 확인하는 이유). 복구 뒤의 재시작은 그 기록이 이미 있으므로 예전대로 허용된다. 뷰만 쓰면(`69841`처럼 `@` 없이) 마지막 확정 인증서를 뜻하므로 항상 통과한다.

## 남은 위험

- notarize가 실행을 기다린다. 뷰 지연이 블록 실행 시간만큼 늘어난다. 증명 검증이 느리면 리더 타임아웃에 걸릴 수 있다.
- notarize가 디스크 동기화를 기다린다. 디스크가 느린 검증자는 투표가 늦다. 절반 이상이 느리면 체인도 그만큼 느려진다(시험으로 확인: 멈췄다가 재개).
- 제안자 자신의 블록은 여전히 보낸 뒤에 저장한다(Commonware 방식). 다른 투표자들이 디스크에 저장한 뒤 투표하므로, 인증된 블록은 여전히 f+1명의 정직한 검증자에게 있다.
- FOCIL 판정은 여전히 검증자마다 다를 수 있다. 이제는 그 블록이 정족수를 못 얻고 뷰가 넘어갈 뿐이다. 제안자 절반이 계속 거절당하면 블록 간격이 길어진다(시험에서 최대 17초).

---

# 2026-09-29 스톨: 포함 목록 정렬 버그와 열린 파일 한도

2026-09-28 사고 다음날, 높이 104408에서 다시 멈췄다. 이번엔 원인이 둘이었다.

## 증상

- 모든 제안이 포함 목록(FOCIL) 검사에 걸렸다: `inclusion list violated; not voting … missing=1`이 모든 뷰에 반복된다(09-28과 같은 로그, 다른 원인).
- 검증자 재시작이 `Too many open files`로 반복됐다(crash loop). 투표 저널 파티션 `aether-consensus-r69841`의 섹션 파일들을 여는 순간 EMFILE.

## 원인

**1. 제안 정렬 버그(체인을 멈춘 쪽).** 제안자는 포함 목록에 오른 거래를 블록 앞쪽에 먼저 놓고 나머지 멤풀 거래를 뒤에 이었다. 어떤 상장 거래는 송신자의 논스 n+32였는데, 같은 송신자의 이전 논스 n..n+31은 아직 멤풀 뒤쪽에 있었다. `build_block`은 후보를 주어진 순서대로 한 번만 실행하고 실패한 거래를 다시 시도하지 않는다. 논스 n+32는 상태 논스 n 앞에서 스킵되고, `rest`에도 없으니 영영 들어가지 못했다. 검증자들의 append 검사는 "논스 n+32를 넣을 수 있는데 빠졌다"로 나왔고, 모든 제안이 같은 거래에 걸려 정족수를 잃었다.

**2. 열린 파일 한도(재시작을 막은 쪽).** simplex 투표 저널은 뷰마다 섹션 파일 하나를 만들고, 마지막 확정 − view_retention(20) 아래로만 지운다. 체인이 멈춘 동안 탄 뷰마다 섹션이 쌓여 190개가 됐다(확정 위의 뷰들은 안전성 증거라 지우면 안 된다 — 쌓인 것 자체는 정상 동작). 저널은 시작할 때 모든 섹션을 열어 파일 디스크립터를 하나씩 잡고, 검증자는 평소에도 ~70개를 쓴다. launchd가 넘겨주는 소프트 한도는 256: 70 + 190 > 256이라 EMFILE.

## 수정

| 무엇 | 어디 | 내용 |
|---|---|---|
| 제안 정렬 | `chain.rs` `mempool_candidates` (482400e) | 상장 거래와 나머지를 (논스, 상장 우선, 송신자) 하나의 순서로 정렬. 상장 거래가 같은 송신자의 이전 논스 뒤에 온다 |
| 저널 쓰기 버퍼 | `engine.rs` `VOTE_WRITE_BUFFER` | 1 MiB → 64 KiB. 섹션마다 버퍼 하나라, 스톨 중 190 섹션 × 1 MiB의 RAM을 들고 있었다 |
| 시작 로그 | `engine.rs` | 시작할 때 저널 섹션 수를 기록, 128개 넘으면 경고(`vote journal holds many section files`) |
| 소프트 한도 | `main.rs` `raise_nofile_limit` | 노드가 시작할 때 자기 RLIMIT_NOFILE 소프트 한도를 하드 한도(또는 65 536, macOS `kern.maxfilesperproc`)까지 올리고 값을 기록(`open-file limit`). GUI 앱의 자식 노드가 256을 물려받아도 된다 |
| launchd | LaunchAgents/LaunchDaemons | `NumberOfFiles` 상향(오너 조치, 2026-09-29 적용) |

회귀 시험: `chain.rs`의 `listed_txs_at_a_later_nonce_land_after_their_senders_earlier_ones`(한 송신자 64거래, 목록은 논스 32..47 — 수정 전 순서를 그대로 둔 `candidates_2026_09_28`로 위반 재현), `listed_txs_of_several_senders_wait_for_their_own_earlier_nonces`(3송신자). 저널 상한: `devnet.rs`의 `the_vote_journal_keeps_a_bounded_number_of_section_files`(높이 80에서 각 노드 섹션 ≤ 40; 정상 21~23).

## 탐지 (모니터링 경보 텍스트)

- `Too many open files` — 노드 로그. 즉시 페이지.
- `vote journal holds many section files` — 시작 로그. 섹션 128개 이상: 스톨이 길었다는 뜻.
- `inclusion list violated; not voting` — 여러 뷰에 반복되면 제안 정렬/포함 목록 문제.
- 높이가 수 분간 멈춤(기존 liveness 경보).

## 복구 (당시 한 일)

1. launchd `NumberOfFiles` 상향.
2. 정렬 수정 바이너리(482400e) 배포.
3. **네 검증자를 함께 재시작했다.** `AETHER_RECOVER_CONSENSUS`는 09-28 복구 값(뷰 69841 → 저널 `aether-consensus-r69841`)을 그대로 뒀다(위 절차 5: 두 번 투표 방지).
4. 높이 상승 확인. 저널은 확정이 이어지면서 다시 21~23 섹션으로 줄었다(확인: `ls ~/aether-testnet/<n>/aether-consensus-r69841 | wc -l`).

## 남은 위험

- 스톨이 길어지면 저널 섹션은 계속 쌓인다(뷰당 하나, 확정 전엔 못 지운다). fd 예산은 이제 65 536(또는 하드 한도)까지고 시작 로그로 보이지만, 아주 긴 스톨 뒤 재시작은 여전히 파일 수만큼 fd를 쓴다.
- 소프트 한도는 `kern.maxfilesperproc`보다 크게 못 올린다.
