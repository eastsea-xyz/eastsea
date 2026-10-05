# 메인넷 출시 절차

한 번 만들면 되돌릴 수 없는 제네시스를 실제로 만드는 절차다. 결정 사항(사전 발행 0, faucet 없음, 창업자 몫 없음, 첫 블록부터 node rewards, history v2)은 [12-launch-plan.md](../design/12-launch-plan.md)에, 규칙은 [15-node-rewards.md](../design/15-node-rewards.md)에 있다. 이 문서는 손으로 하는 순서만 다룬다.

출시 판단 자체는 별개다: 12-launch-plan의 빠른 메인넷 조건 (1)~(8)을 모두 충족했는지가 먼저고, 이 문서는 그 다음의 실행 계획이다.

## 0. 리허설 (출시 전날과 직전, 두 번)

```bash
mkdir -p ./tmp
scripts/mainnet-rehearsal.sh "$(mktemp -d ./tmp/mainnet-rehearsal.XXXXXX)/rehearsal"
```

버리는 로컬 네트워크에 **메인넷과 같은 제네시스 플래그**(새 체인 아이디, `"protocol": 3`, `"history": 2`, node rewards, 등록기, 예비 키 3개, faucet·사전 발행 없음)를 걸고 4검증자+예비키+후보 Mac을 `aether run`만으로 띄워 확인한다(스크립트가 자체 `ceremony-check.json`을 만들어 모든 `run`에 전달하므로 실전과 같은 시작 바인딩을 그대로 예행한다 — 감사 6): 블록 확정·4검증자 일치, 높이 1의 프로토콜이 3·에포크당 등록 상한 활성, 메인넷 규칙 목록 전부(아래 2단계), 빈 블록 조용(history v2), 사전 발행·faucet 없음, 첫 에포크 분배가 `rewards::issuance`와 정확히 일치(새 Mac pool/32), 예비 키는 4석이 서 있으면 들어오지 않고 팔로워로 남음, 30일 프루닝 기본값, **현재 바이너리의 그림자 재실행 일치**. **PASS가 아니면 다음 단계로 가지지 않는다.** 출시 직전에는 출시에 쓸 바이너리로 다시 한 번.

## 업그레이드 전 필수: 메인넷 그림자 재실행

업그레이드 후보 바이너리로 메인넷의 높이 0부터 **대상 높이까지** 다시 실행한다. 원본 노드의 상태를 바꾸지 않고 `./tmp/` 아래 격리된 저장소를 사용한 뒤 지운다. 아카이브 RPC 또는 중지된 history-v2 아카이브 노드의 데이터 디렉터리와 원본 `network.json`이 필요하다. 가지치기로 블록이나 영수증이 빠진 노드로는 완전한 재실행을 할 수 없으며, 누락을 PASS로 처리하지 않는다.

```bash
aether shadow --from "$ARCHIVE_RPC" --to "$FINALIZED_HEIGHT"
# 또는: aether shadow --from "$STOPPED_ARCHIVE_DATA" --network "$GENESIS_NETWORK_JSON" --to "$FINALIZED_HEIGHT"
```

매 블록의 상태 루트를 원본의 블록 요약과 대조하고, 거래 순서대로 직렬화한 영수증의 BLAKE3 다이제스트도 대조한다. 이 **영수증 다이제스트는 재실행 검사값**이며 현재 블록 형식에 온체인 영수증 루트는 없다. 첫 불일치에서 높이·필드·양쪽 값을 출력하고 0이 아닌 코드로 끝난다. 업그레이드 전에 이 검사와 `scripts/mainnet-rehearsal.sh`가 모두 PASS여야 한다.

## 스크립트로 하는 세레머니 (Ceremony with the script)

아래 1~3단계(키 세레머니 → network.json → 제네시스 DKG)와 DKG 후 검사는 `scripts/mainnet-genesis.sh`가 그대로 안내한다. 손으로 하기 전에 `scripts/mainnet-genesis.sh --dry-run`으로 한 Mac에서 전 과정을 먼저 예행한다(일회용 키 — REHEARSAL, not a launch file; `--rehearsal`은 오직 이 모드에서만 쓴다). 수순:

1. **검증자 Mac 4대, 각각**: `scripts/mainnet-genesis.sh keys` — 이 Mac의 검증자 신원을 만들고 공개 항목만 출력한다. **`validator.pub.json` 하나만** 코디네이터(검증자 1 Mac)로 복사한다.
2. **창업자 Mac**: `scripts/reserve-keys.sh init`(아래 1단계) — 공개 항목 3개를 코디네이터로 복사한다.
3. **코디네이터**: `scripts/mainnet-genesis.sh assemble --chain-id <새 체인 아이디> --registrar <서명 Mac의 x‖y hex> --reserve-operator <창업자 주소> --reserve <예비 pub>×3 --validator <검증자 pub>×4` — 입력을 전부 검증하고(hex 형식·검증자 4개 상이·예비 키≠검증자 키·P-256 등록기 점) 공표 정책(protocol 3, history 2, node rewards, registry v3, 타이밍 3600/24/24는 기본값 그대로)으로 genesis.json을 만들어 규칙 검사까지 돌린다. 리허설 전용 값(`--epoch-blocks`·`--dev-registrar`·체인 아이디 7780/7799)은 거부한다. 옆에 `ceremony.json`(공개 기록: 이 세레머니가 조립한 체인 아이디와 검증자 목록)을 남긴다.
4. **검증자 Mac 4대, 동시에**: genesis.json을 각 Mac으로 복사해(공개 파일 — 어디로든) 아래 3단계의 `aether dkg`를 각자 자기 `--data` 디렉터리로 실행한다.
5. **코디네이터**: 검증자 1이 DKG 후 쓴 `network.json`으로 `scripts/mainnet-genesis.sh check <network.json> --chain-id <아이디> --ceremony <assemble이 만든 ceremony.json>` — strict `aether mainnet-rules`(`--rehearsal` 절대 없음)와 최종 파일의 제네시스 플래그를 PASS/FAIL 목록으로 검사한다(실패가 하나라도 있으면 0이 아닌 코드). `--chain-id`는 필수이고 ceremony.json에 기록된 아이디와 같아야 하며 7780(테스트넷)·7799(리허설)는 거부한다. 최종 network.json도 공개 파일이다(지갑·노드가 쓰는 바로 그 파일). **PASS하면** 옆에 `ceremony-check.json`(감사 6의 고정 기록: 체인 아이디, DKG 라운드, 위원회 identity, 통과한 최종 파일 **바이트의 sha256**, 불변 제네시스 전체 — 로스터·등록기·규칙 플래그·예비 키)을 남긴다. **이 기록을 최종 network.json과 함께 모든 검증자 Mac으로 복사한다**: 아래 6단계와 노드 시작 바인딩이 이 기록 없이는 새 제네시스 체인에서 투표를 거부한다.
6. **검증자 Mac 4대, 각각 투표 전**: 최종 network.json과 `ceremony-check.json`을 받은 뒤 `scripts/mainnet-genesis.sh verify-local <network.json> --data <이 Mac의 데이터 디렉터리> --ceremony <ceremony-check.json>` — 기대 체인 아이디를 **기록에서만** 읽어(검사 대상 파일에서 유도하지 않는다: 전송 중 체인 아이디 치환이 자기 승인되던 구멍, 감사 6 A6-3) 이 Mac에서 strict 규칙 검사를 다시 돌리고, 이 Mac의 `threshold.json`·`network.json`을 그 기록에 바인딩한다(`aether mainnet-bind`, **share는 절대 출력하지 않는다**). 같은 체인 아이디·위원회 identity를 가진 채 불변 제네시스가 다른 오래된 로컬 network.json은 "같은 네트워크"가 아니라 **stale로 거부**된다(감사 6 A6-4). 통과하면 기록이 `<data>/ceremony-check.json`에 저장되어, 지갑의 `aether run`(--network·--ceremony 없이 시작)도 다음 시작부터 같은 세레머니에 바인딩된다.

**절대 복사하지 않는 것**: 각 Mac의 `validator.key`·`node-account.key`·DKG 후의 `threshold.json`(모두 모드 600, 만든 Mac에만 둔다). 코디네이터 디렉터리에는 공개 파일만 있어야 한다. `scripts/test-mainnet-genesis.sh`가 이 규칙 전부 — 변조 파일 거부·중복 검증자 키 거부·리허설 값 거부·코디네이터 비밀 누출 없음·감사 6 바인딩(기록 없는 verify-local 거부, 전송 중 체인 아이디 치환 거부, 같은 로스터·라운드의 다른 output 거부, stale 로컬 제네시스 거부, 노드 시작 거부) — 를 자동 검사한다.

## 1. 키 세레머니

| 무엇 | 어디서 | 비고 |
|---|---|---|
| 검증자 키 ×4 | 검증자로 쓸 Mac 4대, 각각 `aether keygen --data <dir>` | 서로 다른 사람의 Mac(12-launch-plan). 디스크에만 있는 유일한 사본이다 |
| 예비 키 ×3 | 창업자 Mac 1대, `scripts/reserve-keys.sh init` | `~/aether-reserve/{1,2,3}`. 규칙과 운영은 [reserve-keys.md](reserve-keys.md) |
| 등록기 키 | 서명 전용 Mac, `aether-registrar-signer init` | P-256, Secure Enclave. 등록·재인증 서명에 쓴다(`aether run … --registrar-signer <socket>`). 절차는 [registrar.md](registrar.md). `aether registrar-key`(파일 키)는 개발망·예행연습용으로만 남는다 |
| DeviceCheck 키 | Apple 개발자 계정의 `.p8` | `~/.config/aether/devicecheck/`. **메인넷 등록기는 반드시 Apple 키**. 리허설의 `--dev-registrar`는 시험 전용이다 |
| faucet 키 | (없음) | 메인넷은 faucet이 없다. 만들지 않는다 |

모든 `validator.key`·`node-account.key`와 `.p8`은 오프라인(암호화된 외장 드라이브 등)에 백업한다. 잃어버린 검증자 키는 그 자리를 영원히 못 채운다. **등록기 키는 백업할 것이 없다**: 개인 키는 Secure Enclave 밖으로 나오지 않으므로(`registrar.key`는 개발망용), 서명 Mac을 잃으면 위원회 교체로 새 키를 넣는다([registrar.md](registrar.md)).

dev 계정(1–10번)은 메인넷 제네시스에서 잔액이 0이다(사전 발행 0). 후보 등록은 `aether candidate-register …`가 `aether_status.free_registration`을 확인한 뒤 무료 등록 레인으로 제출한다. 운영자가 `aether_registrationNonce`의 논스와 만료 높이를 포함한 relay 메시지에 서명하고, 확정되면 가스 0 영수증을 받는다. 레인이 없는 7780에서는 기존 계약 거래 경로를 사용한다. `--tip 0`만으로는 새 제네시스의 상태 증가 수수료가 면제되지 않는다.

## 2. network.json

검증자 1번 Mac에서(공개 항목들을 모아; `--registrar`에 넣을 등록기 공개키 hex는 서명 전용 Mac에서 `aether-registrar-signer public`이 출력한 x‖y hex를 옮겨 적는다 — 파일 키 `aether registrar-key`는 개발망·예행연습용이다):

```bash
aether network \
  --chain-id <새 체인 아이디> \
  --protocol 3 \
  --history 2 \
  --node-rewards \
  --registrar <서명 전용 Mac의 `aether-registrar-signer public`이 출력한 x‖y hex> \
  --reserve-operator <창업자 지갑 주소> \
  --reserve ~/aether-reserve/1/validator.pub.json \
  --reserve ~/aether-reserve/2/validator.pub.json \
  --reserve ~/aether-reserve/3/validator.pub.json \
  <검증자 1의 validator.pub.json> <검증자 2의> <검증자 3의> <검증자 4의> \
  > genesis.json
```

- `--faucet`을 주지 않는다: 이 네트워크에는 사전 발행이 없고, 모든 토큰이 발행(보상)으로만 나온다.
- `--protocol 3`은 제네시스부터 프로토콜 3 규칙(증명 시장, registry v3·에포크당 등록 상한, 16석 증가 추첨)을 켠다. 메인넷은 증명 보상 없이 열리지 않으므로 이 값은 생략하지 않는다(15-node-rewards "구현 순서" 2번: 메인넷은 제네시스부터 이 규칙, 업그레이드 불필요). 7780에는 이 필드가 없다(프로토콜 1 제네시스, 업그레이드로 2·3 도입 — 제네시스가 바뀌지 않는다).
- `--history 2`는 새 제네시스에서만 유효하다(7780에는 없다). 빈 블록이 조용해지고 era 파일·30일 프루닝이 기본이 된다.
- epoch_blocks/min_streak/draw_epochs는 기본값(3600/24/24)을 그대로 쓴다. 리허설에서 줄여 본 것은 시간 단축용 값이다.

### 메인넷 규칙 목록 (높이 1부터 켜져 있어야 하는 규칙)

메인넷은 어떤 규칙도 "출시 뒤 업그레이드로 켠다" 없이 제네시스부터 전부 켜져 있어야 한다. 제네시스 프로토콜 필드는 이미 있다(갭 G1 닫힘: `aether network --protocol 3`; 7780처럼 프로토콜 1로 열리면 증명 시장·등록 상한·16석 증가가 꺼진 채 시작하므로 이 값을 생략하지 않는다). 목록은 코드에 하나로 있다(`crates/node/src/mainnet.rs`, `mainnet::check`) — 항목을 추가하면 아래 세 검사가 같이 실패한다:

- `aether mainnet-rules --network genesis.json` — network.json에서 노드와 똑같이 제네시스를 만들어 항목마다 `ok`/`FAIL`을 출력하고, 꺼진 것이 하나라도 있으면 실패한다(DKG 뒤 최종 network.json으로 다시 한 번). 규칙은 20개 제네시스 규칙 + 최종 파일 게이트 4개이며, **실제 출시 검사는 에포크(3600블록)·후보 워밍업(24)·추첨(24에포크)의 공표된 정책 값을 정확히 요구**하고 레지스트라 키가 0이거나 곡선 밖이면 실패한다. 리허설만 `--rehearsal`로 단축 값을 허용하며, 허용했다는 사실이 출력에 `REHEARSAL VALUES`로 남는다. 최종 파일 게이트는 `output` 문자열을 노드 자신의 디코더로 디코딩해 위원회를 복원하고 로스터 일치·identity=그룹 공개키·노출된 share 거부를 검사한다(감사 5 A5-4: 구조만 그럴듯한 `identity: "aa"`·`output: "bb"` 파일은 여기서 실패한다). `--bundle`을 붙이면 25번째 규칙 "bundled ceremony record"가 추가된다 — network.json **옆의** `ceremony-check.json`이 그 파일의 정확한 바이트를 pin하는지 검사하는 릴리스 게이트(6단계). 세레머니 자신의 check는 기본 24규칙으로 돈다: 기록은 PASS **뒤에** 쓰여지므로.
- `scripts/mainnet-rehearsal.sh`(0단계) — 같은 검사를 PASS 항목으로 돌리고, 살아 있는 네트워크에서 높이 1의 프로토콜과 등록 상한도 확인한다.
- 단위 테스트(`crates/node/tests/mainnet_rules.rs`) — 메인넷 플래그 제네시스로 모든 항목이 켜져 있는지, 플래그를 하나 빼면 정확히 그 항목이 꺼지는지 확인한다.

| 규칙 | 켜져 있다는 것 |
|---|---|
| chain id | 테스트넷(7780)·리허설(7799)가 아닌 새 체인 아이디이다 (리허설에서만 예약 아이디 허용) |
| protocol from genesis | 높이 1의 프로토콜이 이 바이너리의 최신(지금 3)이다 |
| proof market | 첫 블록부터 statement 기록·증명 지급이 살아 있다 (프로토콜 2) |
| registry v3 | 등록기 컨트랙트가 v3 코드로 시작한다 |
| registration cap | 에포크당 신규 등록 상한(16)이 온체인에 있다 |
| 16-seat growth | 검증자 증가 추첨이 16석까지 자란다 (프로토콜 3) |
| node rewards | 노드 보상이 첫 블록부터 분배된다 |
| beacons | 한 에포크(3600블록)에 비콘 슬롯 4개가 들어간다 |
| re-attestation | 하루 한 번 재확인(등록기 P-256 서명)이 살아 있다 |
| reserve rules | 창업자 예비 키 3개가 온체인에 있고 독립 운영자 4명 미만에서만 앉는다 |
| smooth issuance | 발행이 매끄러운 감쇠다: 1 AETH/블록에서 연 15% 감쇠, 0.1 AETH 바닥 |
| history v2 | 빈 블록이 조용하고 era 파일이 쌓인다 |
| receipt commitments | 높이 1부터 각 블록이 자신의 실행 영수증 루트를 커밋하고 검증자가 재실행해 검사한다 |
| paid state growth | 새 슬롯·계정당 0.0001 AETH, 코드 바이트당 0.000001 AETH를 소각하고 블록당 새 슬롯을 512개로 제한한다 |
| pruning default | 프루닝이 기본(30일 보존)이다 |
| no premine, no faucet | 제네시스 잔액이 전부 0이다 |
| zero-tip acceptance | 첫 블록 실행·증명 base fee가 0이어서 잔액 0 계정의 단순 송금은 팁 0으로 거래한다 |

최종 파일 게이트(감사 5 A5-4; DKG 뒤 network.json의 4개 규칙 — pre-DKG 파일은 "아직 게이트 대상 아님"으로 통과):

| 규칙 | 켜져 있다는 것 |
|---|---|
| committee output decodes | `output`이 노드 시작 때 쓰는 같은 디코더로 디코딩된다 |
| output seats the genesis roster | 복원된 플레이어 집합이 검증자 로스터(같은 키·같은 수)와 정확히 일치한다 |
| identity is the group public key | `identity`가 이 output의 그룹 공개키이다 (지갑이 고정하는 바로 그 키) |
| no revealed seated share | output에 앉은 검증자의 share가 노출돼 있지 않다 (A4-1의 같은 검사) |

눈으로 확인한다:

```bash
python3 - <<'PY'
import json
n = json.load(open("genesis.json"))
assert n["chain_id"] == <새 체인 아이디>
assert n.get("protocol") == 3, "메인넷은 제네시스부터 프로토콜 3"
assert n.get("history") == 2 and n.get("node_rewards") is True
assert n.get("faucet") is None, "faucet이 있으면 안 된다"
assert len(n.get("reserve", {}).get("validators", [])) == 3
assert len(n["validators"]) == 4
PY
aether mainnet-rules --network genesis.json
```

## 3. 제네시스 DKG

각 검증자 Mac에서(서로를 `index@ip:port`로):

```bash
aether dkg --network genesis.json --port <p2p 포트> --data <자기 데이터 디렉터리> \
  --peers 1@<ip1>:<port>,2@<ip2>:<port>,… 
```

4개가 모두 끝나면 검증자 1의 `<data>/network.json`이 최종본이다(identity, output이 들어 있다). 예비 키 폴더의 network.json은 아직 없어도 된다(제네시스 4석이 서 있으면 예비 키는 팔로워로 남고, 위원회가 4석에 못 미칠 때 모자란 자리를 채우면서 자기 몫을 받는다).

DKG가 쓴 network.json이 제네시스 플래그를 그대로 가져갔는지 확인한다(세레머니가 플래그를 흘려버리면 검증자가 전부 아카이브 모드로 도는 식의 사고가 된다):

```bash
python3 - <<'PY'
import json
n = json.load(open("<검증자 1 데이터 디렉터리>/network.json"))
assert n.get("protocol") == 3 and n.get("history") == 2 and n.get("node_rewards") is True
assert n.get("faucet") is None and n.get("epoch_blocks") is None
assert len(n.get("reserve", {}).get("validators", [])) == 3
PY
aether mainnet-rules --network <검증자 1 데이터 디렉터리>/network.json
```

## 4. 사용자 확인 직전 — 되돌릴 수 없는 마지막 단계

**제네시스 블록이 확정되는 순간 체인 아이디·제네시스 해시·발행 규칙이 영원히 굳는다.** 잘못된 network.json으로 시작하면 수정은 새 체인 아이디로 처음부터 다시하는 것뿐이다(토큰이 이미 분배된 뒤에는 사실상 불가능). 그러니 DKG 후, 첫 블록 전에:

화면에 다음을 크게 띄우고 **창업자 본인이 소리 내어 확인하고 진행을 말로 확인한다**:

- 체인 아이디가 의도한 값이다.
- 검증자 4개 키 지문과 예비 키 3개 지문이 키 세레머니 결과와 일치한다.
- 등록기 키 지문이 일치한다(`aether-registrar-signer public`의 x‖y가 `aether network --registrar`에 넣은 값과 같다). DeviceCheck `.p8`이 레지스트라 노드에 있고, 서명 전용 Mac의 도우미가 `serve` 중이다([registrar.md](registrar.md)).
- `"protocol": 3`(제네시스부터 증명 시장·등록 상한·16석 증가), `"history": 2`, node rewards 켜짐, faucet 없음, 사전 발행 없음(2단계 검증 출력과 `aether mainnet-rules` 전 항목 ok).
- 리허설(0단계)이 이 바이너리로 PASS했다.
- 소스 공개 준비가 됐다(12-launch-plan: 메인넷과 동시 공개).
- 앱 번들(6단계)이 심사 중이다.

하나라도 아니오면 여기서 멈춘다. 키는 폐기하고 1단계부터 다시한다. 아직 잃은 것은 없다.

## 5. 검증자 가동

- 각 검증자 Mac: `aether run --network <최종 network.json> --ceremony <ceremony-check.json> --data <dir>`을 launchd + `caffeinate -s`로(`scripts/testnet-launchagent.sh` 패턴, 라벨은 메인넷용으로 따로). 데이터 디렉터리는 메인넷 전용으로 새로 만든다(테스트넷 것을 재사용하지 않는다). 시작할 때 노드는 위 6단계와 같은 바인딩을 스스로 다시 한다(record 대 파일 바이트·이 Mac의 network.json/threshold.json). 기록은 `--ceremony` → `<data>/ceremony-check.json` → `--network` 파일 **옆의** `ceremony-check.json` 순서로 해상된다(fail-closed: 명시적으로 전달된 경로가 없으면 그 사실을 말하고 거부한다) — 그래서 소비자 앱의 시작 경로(`--network`만 주고 시작)는 6단계의 번들 쌍에서, 첫 시작이 저장해 둔 `<data>` 사본으로 재시작된다. **share가 있는 Mac**(제네시스 검증자, reshare로 seated된 Mac)은 기록이 없거나 불일치면 **시작을 거부**하고 verify-local 안내를 출력한다 — 이 Mac들은 투표할 수 있으므로. reshare로 seated된 Mac은 로컬 network.json의 라운드가 기록의 라운드보다 크면 기록의 불변 제네시스 반쪽에 묶여 번들 기록만으로 재시작된다(운영자 조치 없음). **share 없는 Mac**(팔로워·후보)은 도달 가능한 기록이 없으면 경고 로그를 남기고 **계속 따라간다** — 이 Mac은 투표할 수 없고(증명서로 블록을 검증한다) 잘못된 제네시스는 동기화 실패로만 끝난다. 기록이 있는데 불일치면 shareless든 signer든 거부한다.
- 프로토콜 업그레이드 단계는 없다: 제네시스가 `"protocol": 3`으로 시작해 첫 블록부터 전부 마지막 규칙이다. 출시 뒤의 규칙 변경만 위원회 서명 업그레이드로 한다.
- 등록기 노드: `--devicecheck-key <.p8 경로> --devicecheck-key-id <KID> --devicecheck-team <팀 아이디> --registrar-signer <서명 Mac의 signer.sock 경로>`를 함께. 서명 키는 노드가 아니라 서명 전용 Mac의 Secure Enclave에 있다([registrar.md](registrar.md)).
- 창업자 Mac: `scripts/reserve-keys.sh install <최종 network.json>`. 예비 키 3개는 위원회가 4석에 못 미칠 때만 모자란 자리를 채운다 — 제네시스 4석이 서 있으면 하나도 들어오지 않고 팔로워로 남는다([15-node-rewards.md](../design/15-node-rewards.md) "창업자 예비 키"). 사람이 할 일은 없다.
- 확인: 각 노드 `aether status`로 높이가 오르고 `schedule`이 `[3, 0]`이다(제네시스부터 프로토콜 3). `aether_handoff`는 위원회가 **바뀔 때만** 값을 준다 — 제네시스 4석이 그대로면 `null`이다. 예비 키 쪽에서 볼 것은 `threshold.json`이 없다는 점이고, 위원회가 4석 아래로 짧아져 모자란 자리를 채울 때만 생긴다.

## 6. 앱 번들 업데이트

- `apps/wallet/Resources/network.json`을 최종 network.json으로 바꾸고, **코디네이터의 check가 그 옆에 남긴 `ceremony-check.json`을 같이 번들한다** — 이 쌍이 소비자 Mac의 시작 경로 전부다: 지갑 앱은 `--network`만 넘기고 `aether run`이 옆의 기록을 찾아 바인딩·저장한다(5단계). 최종 파일의 바이트가 조금이라도 다시 쓰여지면 기록의 digest가 어긋나므로, check를 통과한 그 파일을 그대로 복사한다.
- 게이트: `aether mainnet-rules --bundle --network apps/wallet/Resources/network.json` — 25번째 규칙 "bundled ceremony record"가 번들 쌍을 검사한다(기록 누락·다른 세레머니의 기록·digest 불일치 FAIL). `scripts/build-wallet.sh`가 **모든 앱 빌드에 이 게이트를 자동으로** 돌리므로, 이 게이트를 통과하지 못한 새 제네시스 빌드는 앱이 만들어지지 않는다. 기록 없는 7780 예외는 코드에 고정된 배포 파일의 **전체 SHA256과 바이트 단위로 일치할 때만** 통과한다. 체인 아이디나 제네시스 플래그만 7780처럼 바꾼 파일은 거부한다.
- 체인 아이디가 바뀌므로 앱의 표시명·설명에서 "테스트넷" 문구를 뺀다.
- 앱은 faucet이 없는 네트워크임을 사용자에게 그대로 보여 준다(에어드랍 안내 문구 없음).

## 7. 공표 체크리스트

- [ ] 소스 코드 공개(메인넷 시작과 동시 — 12-launch-plan).
- [ ] 발행 규칙 공개: 사전 발행 0, 연 감쇠 발행, 절반은 노드 보상·절반은 증명 보상, 1/16 상한, 미분배 몫은 영구 미발행.
- [ ] 검증자 4대의 운영자 공개(각각 다른 사람/조직), 예비 키 정책 공개(독립 운영자 4명 미만 동안 창업자 Mac의 안전망, 보상 없음).
- [ ] 네트워크 파라미터 공개(체인 아이디, 프로토콜 3 제네시스, epoch, history v2, 프루닝 기본값).
- [ ] 출시 공지(앱 릴리스 노트, 웹사이트, SNS) — 하루 전에 초안 확정.

## 8. 중단·복구 기준

제네시스 **전**에는 언제든 중단할 수 있다(4단계의 답이 아니오). 제네시스 **후**에는 롤백이 없다. 상황별:

- **합의가 멈춤(파티션, 검증자 다수 다운)**: [consensus-recovery.md](consensus-recovery.md)의 복구 절차(함께 재시작; 저널을 잃은 검증자는 다음 위원회 라운드까지 팔로우). 검증자 4대 중 3대만 살아 있으면 체인은 계속 간다(3f+1).
- **예비 키 Mac이 죽음**: 독립 운영자가 4명 이상이면 영향 없다(그 자리는 어차피 비어 있다). 미만이면 창업자 Mac을 최우선으로 복구한다(launchd KeepAlive가 재시작한다).
- **등록기 죽음**: 새 등록·재인증만 멈춘다(기존 Mac의 재인증 유예가 지나면 그 Mac의 보상이 줄어든다). 서명 전용 Mac의 등록기를 최우선으로 복구한다.
- **보상·발행 규칙 불일치 발견**: 즉시 공표하고 합의된 프로토콜 업그레이드로만 고친다. 잘못 발행된 물량을 되돌리는 상태 롤백은 없다.
- **아예 처음부터(새 제네시스)**: 마지막 수단. 체인 아이디를 바꾸고 1단계부터. 기존 체인은 그대로 두고 "이주"로 안내한다. 이미 분배가 시작된 뒤에는 커뮤니티 합의 없이는 불가능하다.

리스크를 줄이는 규칙: 출시 후 첫 24시간은 창업자 전원이 온라인(`scripts/soak/monitor.sh` 알림 켜기), 하루가 지나고 안정이 확인되면 공표를 확대한다.
