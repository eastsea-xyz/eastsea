# 공개 읽기 전용 게이트웨이 (rpc.eastsea.xyz)

앱 없이 익스플로러를 여는 방문자를 위한 얇은 게이트웨이. 출처: `docs/research/public-read-access-2026-10-05.md` §8(a).
**이것은 "신뢰하는 공용 RPC"가 아니다.** 검증자가 아닌 팔로워 1대가 익스플로러가 쓰는 읽기 메서드만,
상한을 붙여 대답한다. 화면의 표기는 "Public gateway · not verified"이고, 확장(`DEFAULT_RPCS`)에는
들어가지 않는다(2026-09-29 결정). 쓰기는 존재하지 않는다: 어떤 메서드도 중계하지 않고 서명도 없다.

## 무엇을 허용하고 무엇을 거부하나

`aether follow --public-read-only`를 켜면 `rpc::handle_value` 앞의 게이트가 동작한다(`crates/node/src/rpc.rs`,
`PUBLIC_READ_METHODS`). `eastsea_*` 별칭은 정규화 뒤 게이트를 지나므로 같은 규칙을 받는다.

**허용(익스플로러 `apps/explorer/js`가 실제로 부르는 읽기 + 계정 검증이 필요한 것):**

`aether_status`, `aether_recentBlocks`, `aether_candidates`, `aether_proverStatus`, `aether_getBlock`,
`aether_getReceipt`, `aether_getAccount`, `aether_getFinalized`, `aether_history`, `aether_historyProof`,
`aether_eraInfo`, `aether_eraProof`, `aether_rewards`, `aether_accountHistory`,
`eth_blockNumber`, `eth_call`, `eth_getLogs`.

**거부(그 외 전부):** `aether_sendTransaction`, `aether_faucet`, `aether_registerDevice`, `aether_sendBeacon`,
`aether_sendRegistration`, `aether_signHandoff`, `aether_submitProof`, `aether_handoff`, `aether_reattest`,
`aether_snapshot*`, `aether_eraChunk`, `aether_shard*`, `aether_rotation`, `aether_network`, … 와
허용 목록에 없는 모든 메서드. 노드 로컬 작업(snapshot·shard·era 전송)도 게이트가 막는다. 에러:

```json
{ "code": -32601, "message": "public read-only gateway: aether_sendTransaction is not a public read method (docs/ops/read-gateway.md); writes and node-local methods are refused" }
```

## 상한

낯선 사람의 요청 한 건이 노드에게 강요할 수 있는 양의 한계(`rpc.rs`의 `pub const`, 테스트가 지킨다):

| 항목 | 상한 | 에러 |
|---|---|---|
| 요청 본문(HTTP) | 1 MiB (`PUBLIC_MAX_BODY`, axum body limit) | 413 |
| JSON-RPC 배치 | 8 calls (`PUBLIC_MAX_BATCH`) | -32002 |
| `eth_getLogs` 범위 | 2,000 블록 (`PUBLIC_GETLOGS_WINDOW`; 익스플로러 실사용 1,999) | -32002, 조용한 클램프 아님 — 거부하고 알린다 |
| `eth_call` 가스 | 1,000,000 (`PUBLIC_CALL_GAS`; 명시 gas가 이보다 크면 -32002, 실행은 이 상한으로) | -32002 |
| `aether_rewards` rows | 1,000 (`PUBLIC_REWARDS_LIMIT`) | -32002 |
| `aether_accountHistory` page | 100 (`PUBLIC_HISTORY_LIMIT`) | -32002 |

바인딩은 언제나 loopback이다: `--public-read-only` 상태에서 비-loopback 주소를 요구하면 `serve()`가
`PermissionDenied`로 거부한다. 외부 노출은 cloudflared 터널이 유일한 길이다.
같은 `RpcState`가 iroh QUIC(`aether_net::serve_rpc`) 경로에도 쓰이므로 게이트는 그쪽에도 붙는다 —
게이트웨이 전용 팔로워라 접수 가능하며, 팔로워의 노드 키를 공개 role로 쓰고 싶지 않다면 별도 데이터
디렉터리(아래)로 분리한다.

## 실행

`scripts/run-read-gateway.sh` — **기본은 dry-run**이다. 아무것도 시작하지 않고 계획(명령, `config.yml`,
DNS 레코드)만 출력한다. 실제 실행은 `--apply`일 때뿐:

```bash
scripts/run-read-gateway.sh                                    # dry-run: 계획만
scripts/run-read-gateway.sh --apply \
  --data /Volumes/workspace/eastsea-read-gateway \
  --port 18550 --hostname rpc.eastsea.xyz --tunnel eastsea-read \
  --network <네트워크의 network.json> --from-rpc <검증자 RPC>
```

`--apply`는 뒤로 물러나지 않고 포그라운드의 감독 루프로 실행된다(아래 "감독과 재시작").

- `--data`는 **게이트웨이 전용 디렉터리**여야 한다(기본 `/Volumes/workspace/eastsea-read-gateway`).
  검증자의 데이터 디렉터리를 공유하지 않는다.
- 터널: named tunnel `eastsea-read`을 찾고 없으면 만든다. `cloudflared tunnel route dns`에 실패하면(존 DNS
  권한 없음) 운영자가 추가할 레코드를 출력한다:

  ```
  rpc.eastsea.xyz  CNAME  <tunnel-id>.cfargotunnel.com  (proxied)
  ```

- Cloudflare에 레이트 리밋 규칙 1개를 둔다: 같은 IP 10초 창(연구 §8(a)-3, [CF5]). 이 규칙은 대시보드에서
  수동으로 만든다(스크립트가 건드리지 않는다).
- 로그는 `<data>/logs/`(`follower.log`, `cloudflared.log`, `cloudflared.pid`). follower는 러너의
  직속 자식으로 감독된다(아래).

## 감독과 재시작 (A7-2)

`--apply`는 백그라운드로 떠나보내는 스크립트가 아니라 **감독 루프 그 자체**다. `aether follow`는 스스로
종료할 때가 있다 — 스톨 와치독(10분간 검증된 진행 없음)이 전송 계층을 재시작하려 exit 11로 나가고,
디스크 여유가 바닥나면 exit 12로 기다린다. `aether run`에는 in-process 슈퍼바이저
(`crates/node/src/supervisor.rs`)가 있지만 게이트웨이는 `follow`를 직접 돌리므로 **재시작 소유자는 이
러너**다. 예전의 nohup 방식은 아무도 재시작하지 않았다: 첫 스톨 종료 뒤 터널만 살아 있는 죽은 오리진이
남았고, 업스트림이 회복돼도 게이트웨이는 돌아오지 않았다 (audit 7 A7-2).

정책은 노드 슈퍼바이저와 같다:

| follower 종료 코드 | 러너의 행동 |
|---|---|
| 0 | 의도적 정지로 보고 종료 (cloudflared도 함께 정리) |
| 3 업그레이드 필요 · 4 저장소 · 5 검증자 없음 · 6 신원 · 7 잠김 | 재시작으로 고칠 수 없는 코드: 그 코드로 종료 — 운영자가 봐야 한다 |
| 12 디스크 바닥 | 30초 기다렸다 재시작 |
| 11 스톨 | 백오프(1s → 2배, 최대 60s) 후 재시작; **1시간 창에 4번이면 정지** |
| 9 치명적 태스크 · 10 재생성 가능한 캐시 · 기타 | 백오프 후 재시작 |

- 살아 있던 시간이 5분 이상이면 진행 중이었던 것으로 보고 백오프는 다시 1초부터(슈퍼바이저 `PROGRESS_MS`).
- cloudflared가 죽었으면 follower 재시작 전에 터널을 다시 띄운다.
- **매 재시작 동일한 인자**(`--public-read-only`, loopback bind)를 그대로 쓴다 — 게이트웨이가 꺼졌다가
  더 넓게 열려 돌아오는 일은 없다.
- 러너가 종료하면(정지 코드든 Ctrl-C든) cloudflared도 함께 정리된다.

러너는 포그라운드에서 돌며 재시작·정지를 콘솔에 출력한다(`scripts/tests/test_read_gateway.py`가 이
정책을 지킨다). 로그아웃 후에도 유지하려면:

```bash
nohup scripts/run-read-gateway.sh --apply ... >> /Volumes/workspace/eastsea-read-gateway/logs/runner.log 2>&1 &
```

## 킨 뒤 확인 (1분 체크리스트)

```bash
# 1. 허용된 읽기가 대답한다
curl -s https://rpc.eastsea.xyz -X POST -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"aether_status","params":[]}'
# 2. 쓰기는 -32601 로 거부된다
curl -s https://rpc.eastsea.xyz -X POST -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"aether_faucet","params":[]}'
# 3. getLogs 창 상한은 -32002 로 거부된다
curl -s https://rpc.eastsea.xyz -X POST -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"eth_getLogs","params":[{"fromBlock":"0x0","toBlock":"0x100000"}]}'
```

익스플로러에서: 앱 없는 브라우저에서 열면 헤더 배지가 "Public gateway · not verified"로 바뀌어야 하고,
계정 페이지의 잔액은 위원회 인증서 검증을 통과할 때만 "verified by committee certificate"라고 쓴다.

## 익스플로러 쪽 (함께 바뀐 것)

- `apps/explorer/js/rpc.js`: 읽기는 순서 있는 소스로 간다 — 방문자 자신의 노드 `127.0.0.1:18545` 먼저,
  그다음 Settings에서 바꿀 수 있는 게이트웨이(기본 `https://rpc.eastsea.xyz`). **전송 실패일 때만** 다음
  소스로 넘어가고, 소스가 응답한 JSON-RPC 에러(게이트웨이의 -32601 같은)는 그대로 화면에 나온다.
  실패한 소스는 60초 동안 건너뛴다(차단된 loopback이 매 호출마다 타임아웃을 만들지 않게).
- `apps/explorer/js/app.js`: 헤더 배지가 현재 소스를 말한다("Your Mac's node" / "Public gateway · not
  verified"). loopback을 브라우저가 막았을 때(Chrome Local Network Access 거부, Safari 혼합 콘텐츠 — §3.4)는
  평이한 안내를 보여준다: 앱 설치 / 로컬 네트워크 허용 / 게이트웨이 계속 사용.
- `apps/explorer/js/verify.js`: 계정 페이지는 지갑 확장과 같은 wasm(`scripts/build-extension.sh`가
  `apps/explorer/wasm`에 복사)으로 `verifyAccount`를 돌린다. 통과할 때만 verified 표시.

## 경계 (하지 않는 것)

- 게이트웨이를 확장 `DEFAULT_RPCS`에 넣지 않는다. 확장은 로컬 노드 또는 사용자 지정 HTTPS RPC.
- 쓰기 메서드 중계를 만들지 않는다.
- 검증되지 않은 데이터를 "확인됨"으로 표기하지 않는다.
- 연구 §8(b)의 정적 이력 피드와 상태 증명 전송이 자리 잡으면 이 게이트웨이는 대체 가능하다("이력은 피드,
  상태는 팔로워"). 남겨 두어도 신뢰 모델은 그대로다.
