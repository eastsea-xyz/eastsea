# 메인넷 출시 절차

한 번 만들면 되돌릴 수 없는 제네시스를 실제로 만드는 절차다. 결정 사항(사전 발행 0, faucet 없음, 창업자 몫 없음, 첫 블록부터 node rewards, history v2)은 [12-launch-plan.md](../design/12-launch-plan.md)에, 규칙은 [15-node-rewards.md](../design/15-node-rewards.md)에 있다. 이 문서는 손으로 하는 순서만 다룬다.

출시 판단 자체는 별개다: 12-launch-plan의 빠른 메인넷 조건(E1–E7)을 모두 충족했는지가 먼저고, 이 문서는 그 다음의 실행 계획이다.

## 0. 리허설 (출시 전날과 직전, 두 번)

```bash
scripts/mainnet-rehearsal.sh $(mktemp -d)/rehearsal
```

버리는 로컬 네트워트에 **메인넷과 같은 제네시스 플래그**(새 체인 아이디, `"history": 2`, node rewards, 등록기, 예비 키 3개, faucet·사전 발행 없음)를 걸고 4검증자+예비키+후보 Mac을 `aether run`만으로 띄워 확인한다: 블록 확정·4검증자 일치, 빈 블록 조용(history v2), 사전 발행·faucet 없음, 첫 에포크 분배가 `rewards::issuance`와 정확히 일치(새 Mac pool/32), 예비 키 자동 착석, 30일 프루닝 기본값. **PASS가 아니면 다음 단계로 가지 않는다.** 출시 직전에는 출시에 쓸 바이너리로 다시 한 번.

## 1. 키 세레머니

| 무엇 | 어디서 | 비고 |
|---|---|---|
| 검증자 키 ×4 | 검증자로 쓸 Mac 4대, 각각 `aether keygen --data <dir>` | 서로 다른 사람의 Mac(12-launch-plan). 디스크에만 있는 유일한 사본이다 |
| 예비 키 ×3 | 창업자 Mac 1대, `scripts/reserve-keys.sh init` | `~/aether-reserve/{1,2,3}`. 규칙과 운영은 [reserve-keys.md](reserve-keys.md) |
| 등록기 키 | 검증자 1번 Mac, `aether registrar-key --data <dir>` | P-256. 등록·재인증 서명에 쓴다 |
| DeviceCheck 키 | Apple 개발자 계정의 `.p8` | `~/.config/aether/devicecheck/`. **메인넷 등록기는 반드시 Apple 키**. 리허설의 `--dev-registrar`는 시험 전용이다 |
| faucet 키 | (없음) | 메인넷은 faucet이 없다. 만들지 않는다 |

모든 `validator.key`, `registrar.key`, `.p8`는 오프라인(암호화된 외장 드라이브 등)에 백업한다. 잃어버린 검증자 키는 그 자리를 영원히 못 채운다.

dev 계정(1–10번)은 메인넷 제네시스에서 잔액이 0이다(사전 발행 0). 후보 등록은 수수료 팁 없이 한다: `aether candidate-register … --tip 0`(기본 base fee가 0인 동안 잔액 0으로 등록된다).

## 2. network.json

검증자 1번 Mac에서(공개 항목들을 모아):

```bash
aether network \
  --chain-id <새 체인 아이디> \
  --history 2 \
  --node-rewards \
  --registrar <aether registrar-key가 출력한 x‖y hex> \
  --reserve-operator <창업자 지갑 주소> \
  --reserve ~/aether-reserve/1/validator.pub.json \
  --reserve ~/aether-reserve/2/validator.pub.json \
  --reserve ~/aether-reserve/3/validator.pub.json \
  <검증자 1의 validator.pub.json> <검증자 2의> <검증자 3의> <검증자 4의> \
  > genesis.json
```

- `--faucet`을 주지 않는다: 이 네트워크에는 사전 발행이 없고, 모든 토큰이 발행(보상)으로만 나온다.
- `--history 2`는 새 제네시스에서만 유효하다(7780에는 없다). 빈 블록이 조용해지고 era 파일·30일 프루닝이 기본이 된다.
- epoch_blocks/min_streak/draw_epochs는 기본값(3600/24/24)을 그대로 쓴다. 리허설에서 줄여 본 것은 시간 단축용 값이다.

눈으로 확인한다:

```bash
python3 - <<'PY'
import json
n = json.load(open("genesis.json"))
assert n["chain_id"] == <새 체인 아이디>
assert n.get("history") == 2 and n.get("node_rewards") is True
assert n.get("faucet") is None, "faucet이 있으면 안 된다"
assert len(n.get("reserve", {}).get("validators", [])) == 3
assert len(n["validators"]) == 4
PY
```

## 3. 제네시스 DKG

각 검증자 Mac에서(서로를 `index@ip:port`로):

```bash
aether dkg --network genesis.json --port <p2p 포트> --data <자기 데이터 디렉터리> \
  --peers 1@<ip1>:<port>,2@<ip2>:<port>,… 
```

4개가 모두 끝나면 검증자 1의 `<data>/network.json`이 최종본이다(identity, output이 들어 있다). 예비 키 폴더의 network.json은 아직 없어도 된다(착석 때 자기 몫을 받는다).

DKG가 쓴 network.json이 제네시스 플래그를 그대로 가져갔는지 확인한다(세레머니가 플래그를 흘려버리면 검증자가 전부 아카이브 모드로 도는 식의 사고가 된다):

```bash
python3 - <<'PY'
import json
n = json.load(open("<검증자 1 데이터 디렉터리>/network.json"))
assert n.get("history") == 2 and n.get("node_rewards") is True
assert n.get("faucet") is None and n.get("epoch_blocks") is None
assert len(n.get("reserve", {}).get("validators", [])) == 3
PY
```

## 4. 사용자 확인 직전 — 되돌릴 수 없는 마지막 단계

**제네시스 블록이 확정되는 순간 체인 아이디·제네시스 해시·발행 규칙이 영원히 굳는다.** 잘못된 network.json으로 시작하면 수정은 새 체인 아이디로 처음부터 다시하는 것뿐이다(토큰이 이미 분배된 뒤에는 사실상 불가능). 그러니 DKG 후, 첫 블록 전에:

화면에 다음을 크게 띄우고 **창업자 본인이 소리 내어 확인하고 진행을 말로 확인한다**:

- 체인 아이디가 의도한 값이다.
- 검증자 4개 키 지문과 예비 키 3개 지문이 키 세레머니 결과와 일치한다.
- 등록기 키 지문이 일치한다. DeviceCheck `.p8`이 검증자 1에 있다.
- `"history": 2`, node rewards 켜짐, faucet 없음, 사전 발행 없음(2단계 검증 출력).
- 리허설(0단계)이 이 바이너리로 PASS했다.
- 소스 공개 준비가 됐다(12-launch-plan: 메인넷과 동시 공개).
- 앱 번들(6단계)이 심사 중이다.

하나라도 아니오면 여기서 멈춘다. 키는 폐기하고 1단계부터 다시한다. 아직 잃은 것은 없다.

## 5. 검증자 가동

- 각 검증자 Mac: `aether run --network <최종 network.json> --data <dir>`을 launchd + `caffeinate -s`로(`scripts/testnet-launchagent.sh` 패턴, 라벨은 메인넷용으로 따로). 데이터 디렉터리는 메인넷 전용으로 새로 만든다(테스트넷 것을 재사용하지 않는다).
- 검증자 1(등록기): `--devicecheck-key <.p8 경로> --devicecheck-key-id <KID> --devicecheck-team <팀 아이디>`를 함께.
- 창업자 Mac: `scripts/reserve-keys.sh install <최종 network.json>`. 독립 운영자가 4명이 되기 전까지 예비 키 3개가 자리를 지킨다. 사람이 할 일은 없다.
- 확인: 각 노드 `aether status`로 높이가 오르고, `aether_handoff`가 7멤버(4+3)를 보이고, 예비 키 `threshold.json`이 생기면 착석 완료.

## 6. 앱 번들 업데이트

- `apps/wallet/Resources/network.json`을 최종 network.json으로 바꾼 빌드를 심사에 올린다(출시 시점에는 심사가 통과돼 있어야 한다).
- 체인 아이디가 바뀌므로 앱의 표시명·설명에서 "테스트넷" 문구를 뺀다.
- 앱은 faucet이 없는 네트워크임을 사용자에게 그대로 보여 준다(에어드랍 안내 문구 없음).

## 7. 공표 체크리스트

- [ ] 소스 코드 공개(메인넷 시작과 동시 — 12-launch-plan).
- [ ] 발행 규칙 공개: 사전 발행 0, 연 감쇠 발행, 절반은 노드 보상·절반은 증명 보상, 1/16 상한, 미분배 몫은 영구 미발행.
- [ ] 검증자 4대의 운영자 공개(각각 다른 사람/조직), 예비 키 정책 공개(독립 운영자 4명 미만 동안 창업자 Mac의 안전망, 보상 없음).
- [ ] 네트워크 파라미터 공개(체인 아이디, epoch, history v2, 프루닝 기본값).
- [ ] 출시 공지(앱 릴리스 노트, 웹사이트, SNS) — 하루 전에 초안 확정.

## 8. 중단·복구 기준

제네시스 **전**에는 언제든 중단할 수 있다(4단계의 답이 아니오). 제네시스 **후**에는 롤백이 없다. 상황별:

- **합의가 멈춤(파티션, 검증자 다수 다운)**: [consensus-recovery.md](consensus-recovery.md)의 `AETHER_RECOVER_CONSENSUS=<view>@<height>` 복구. 검증자 4대 중 3대만 살아 있으면 체인은 계속 간다(3f+1).
- **예비 키 Mac이 죽음**: 독립 운영자가 4명 이상이면 영향 없다(그 자리는 어차피 비어 있다). 미만이면 창업자 Mac을 최우선으로 복구한다(launchd KeepAlive가 재시작한다).
- **등록기 죽음**: 새 등록·재인증만 멈춘다(기존 Mac의 재인증 유예가 지나면 그 Mac의 보상이 줄어든다). 검증자 1을 최우선으로 복구한다.
- **보상·발행 규칙 불일치 발견**: 즉시 공표하고 합의된 프로토콜 업그레이드로만 고친다. 잘못 발행된 물량을 되돌리는 상태 롤백은 없다.
- **아예 처음부터(새 제네시스)**: 마지막 수단. 체인 아이디를 바꾸고 1단계부터. 기존 체인은 그대로 두고 "이주"로 안내한다. 이미 분배가 시작된 뒤에는 커뮤니티 합의 없이는 불가능하다.

리스크를 줄이는 규칙: 출시 후 첫 24시간은 창업자 전원이 온라인(`scripts/soak/monitor.sh` 알림 켜기), 하루가 지나고 안정이 확인되면 공표를 확대한다.
