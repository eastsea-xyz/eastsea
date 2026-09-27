# 14. 투표 노드 등록 기준

투표 노드 후보가 되려면 레지스트라의 등록 증명이 필요합니다. 이 문서는 레지스트라가 무엇을 확인하고 무엇을 거부하는지 공개합니다(12-launch-plan.md 결정 기록 "레지스트라", 13-protocol-2.md §4). 코드: `crates/node/src/devicecheck.rs`(레지스트라), `contracts/src/CommitteeRegistry.sol`(온체인 기록), `crates/execution/src/registry.rs`.

## 1. 원칙

- **무료입니다.** 레지스트라는 돈을 받지 않습니다. 등록 트랜잭션도 블록이 혼잡하지 않으면 수수료가 0입니다(R1′).
- **기계적입니다.** 사람이 심사하지 않습니다. 아래 조건을 모두 채우면 등록되고, 하나라도 어기면 거부됩니다. 신원·국적·잔액은 묻지 않습니다.
- **기기 1대 = 투표 키 1개.** Mac 여러 대를 가진 사람은 여러 후보를 가질 수 있습니다. 대신 Mac마다 따로 등록해야 합니다.
- 레지스트라는 후보 풀의 입구만 지킵니다. 블록 확정, 송금, 체인 정지, 추첨 결과에는 권한이 없습니다.

## 2. 절차

1. Aether 앱이 Apple DeviceCheck 토큰을 만듭니다. 이 토큰은 Apple이 서명하고, 그 Mac과 우리 팀의 앱에 묶여 있습니다.
2. 앱(또는 `aether` CLI)이 레지스트라 노드의 RPC `aether_registerDevice`로 보냅니다: DeviceCheck 토큰, 운영자 주소, 투표 키(ed25519), iroh 노드 ID, 생존 신호 계정(beaconer), 소유 증명 서명.
3. 레지스트라가 3절을 확인하고, 통과하면 P-256 등록 증명(r, s)을 돌려줍니다. 서명 대상은 `sha256(abi.encode(chainid, CommitteeRegistry 주소, 운영자, 투표 키, 노드 ID, beaconer))`입니다.
4. 운영자 지갑이 이 증명을 담아 `CommitteeRegistry.register`를 호출합니다. 호출한 주소가 그 후보의 운영자가 됩니다. 컨트랙트가 증명을 레지스트라 공개키로 검증합니다(P256VERIFY).
5. 그 뒤 노드는 에포크마다 beaconer 계정으로 생존 신호를 보냅니다. 추첨 자격은 07-consensus.md "열린 위원회"를 따릅니다.

## 3. 레지스트라가 확인하는 것

| 조건 | 확인 방법 |
|---|---|
| 소유 증명 | 요청이 등록하려는 투표 키 자신의 ed25519 서명이어야 합니다(네임스페이스 `aether-candidate-ownership`, 3절의 서명 대상과 같은 데이터). 남의 투표 키를 선점할 수 없습니다. |
| 노드 ID | 32바이트가 유효한 iroh 노드 ID여야 합니다. |
| DeviceCheck 토큰 | Apple이 이 토큰을 유효하다고 답해야 합니다. 진짜 Apple 기기와 우리 팀 앱에서 나온 토큰만 통과합니다. |
| 기기당 1회 | Apple의 기기별 비트(재설치해도 남음)로 이 Mac이 이미 등록했는지 봅니다. 처음이면 비트를 켭니다. |
| 같은 키의 재등록 | 이미 등록한 투표 키는 처음과 같은 운영자·노드 ID·beaconer로만 다시 증명을 받습니다. 이때는 DeviceCheck를 다시 부르지 않습니다. |

## 4. 체인이 거는 한도

- **에포크당 신규 등록 16건**(`MAX_PER_EPOCH`). 프로토콜 2부터 CommitteeRegistry v2에 적용됩니다. 레지스트라 키를 도난당해도 후보 풀을 한꺼번에 채울 수 없습니다.
- 추첨은 최소 streak(기본 24에포크)과 가동률 95%를 요구하고, 한 번에 바뀌는 자리는 1/3 미만입니다. 새로 등록한 후보가 위원회 과반에 이르려면 여러 추첨 주기를 살아 있어야 합니다.
- **레지스트라 키 교체:** 위원회가 임계 서명한 업그레이드에 `registrar` 필드를 넣으면, 활성화 블록에서 키가 바뀝니다. 창업자가 없어지거나 키를 도난당했을 때 씁니다. 레지스트라 혼자서는 키를 바꿀 수 없습니다.
- **등록 중지:** `registrar`를 0으로 두면 어떤 증명도 검증되지 않아 새 등록이 멈춥니다. 이미 등록한 후보는 그대로 남습니다.

## 5. 체인에 기록되는 것

- 후보마다: 운영자 주소, 투표 키, 노드 ID, beaconer 주소, 등록 에포크, 마지막 생존 신호 에포크, streak, 놓친 에포크 수.
- 이벤트: `Registered(index, operator, validatorKey, nodeId)`, `Beacon(index, epoch, streak)`.
- 레지스트라 공개키와 에포크별 등록 수.
- 기록하지 않는 것: DeviceCheck 토큰, 기기 식별 정보, IP 주소. 레지스트라는 자기 데이터 폴더의 `registrations.json`에 (투표 키 → 등록 시각, 운영자, 노드 ID, beaconer)만 남깁니다.

## 6. 거부 사유

레지스트라(RPC 오류 메시지):

| 사유 | 메시지 |
|---|---|
| 투표 키 서명이 없거나 틀림 | `not signed by the voting key being registered` |
| 노드 ID가 iroh ID가 아님 | `device token rejected by Apple: node id is not a valid iroh id` |
| Apple이 토큰을 거부 | `device token rejected by Apple: …` |
| 이 Mac이 이미 등록함, 또는 등록된 키를 다른 운영자·노드·beaconer로 다시 요청 | `this Mac already has a registered node` |
| Apple 서버 오류 | `DeviceCheck: …` (나중에 다시 시도) |
| 이 노드가 레지스트라가 아님 | `this node does not register devices` |

컨트랙트(`register` 되돌림):

| 사유 | 오류 |
|---|---|
| 이미 등록된 투표 키 | `Known` |
| 증명이 지금 레지스트라 키로 검증되지 않음(다른 체인·다른 운영자 주소로 제출, 키 교체·중지 이후 등) | `BadAttestation` |
| 이번 에포크 등록 16건이 참 | `TooManyThisEpoch` (다음 에포크에 다시) |

## 7. 한계

- 레지스트라는 지금 창업자가 운영합니다(DeviceCheck 키 .p8이 필요). 레지스트라가 등록을 거부하면 새 후보가 들어올 수 없습니다. 이 경우의 대응은 위원회 업그레이드로 키를 바꾸는 것입니다.
- 운영자는 지갑 주소로 구분합니다. Mac 여러 대와 주소 여러 개를 가진 한 사람은 운영자 상한을 피할 수 있습니다. 막는 것은 DeviceCheck(Mac 1대 = 1표)와 교체 속도 제한입니다.
- 로컬 devnet(`--dev-registrar`)은 Apple 확인 없이 모든 기기를 등록합니다. testnet과 mainnet에서는 쓰지 않습니다.
