# 창업자 예비 키 운영

규칙은 [15-node-rewards.md](../design/15-node-rewards.md) "창업자 예비 키"에 있다. 이 문서는 창업자 Mac 한 대에서 예비 키 3개를 실제로 띄우는 방법이다.

## 동작

- 예비 키마다 `aether run` 프로세스가 하나씩 돈다. 데이터 폴더와 포트가 따로다.
- **독립 운영자가 4명 이하일 때는 상시 대기다.** 여기서 독립 운영자는 추첨 자격(24시간 연속 생존)이 있는 Mac의 서로 다른 운영자 주소이며 창업자 주소는 제외한다. 제네시스 예비 목록에 키를 계속 유지하고 은퇴시키지 않는다. 좌석이 없거나 좌석에서 내려온 뒤에도 검증하는 팔로워로 계속 실행하며, 빈자리에 자동 합류할 자격을 유지한다. 등록 후보가 아니라 비콘을 보내지 않고, 좌석 밖에서는 보상도 없다.
- 빈자리가 생길 때: 자격 있는 Mac이 운영자당 한 자리씩 먼저 들어가고, 예비 키는 **4석에 모자란 자리만** 채운다. 제안된 세트에 자기 키가 있으면 팔로워인 채로 재공유에 참여하고, 인계가 확정되면 전환 높이부터 검증자로 투표한다. 사람이 할 일은 없다. **이미 4석이면 생존 규칙으로도 예비 키를 덧붙이지 않는다**(레드팀 1.1).
- 4석 중 한 위원이 두 에포크 내내 비콘을 보내지 않거나 퇴장을 예고할 때: 그 자리 하나만 교체한다. 자격 있는 대기 Mac을 먼저 쓰고, 없으면 예비 키 하나를 쓴다. 옛 위원회의 남은 3석이 3-of-4 정족수로 체인을 계속 확정하는 동안 다음 에포크 안에 재공유·인계를 마친다.
- 야간 생존: 독립 운영자가 4명 이상이고 5석 이상인 위원회는 최악 시간대 정족수 확률이 **0.99 미만**이면 필요한 예비 키만 자동 합류하고, **0.995 이상**이면 내려온다. 중간 구간은 기존 배석을 유지한다. 한 Mac의 예비 키 3개는 함께 꺼지는 한 덩어리(가용 확률 0.9999)로 계산한다. 4석 위원회에는 이 규칙으로도 좌석을 추가하지 않는다.
- 내려올 때: 자격 있는 Mac이 불필요한 예비 좌석을 채우거나, 독립 운영자가 **5명 이상**이 되어 생존 규칙에 필요하지 않은 예비 좌석을 돌려주면 인계 완료 뒤 몫(`threshold.json`)을 지우고 팔로워 대기로 돌아간다. **봉사 몫 만료 카운터도 5명 이상일 때만 증가한다.** 두 에포크 유예 뒤 끊기는 것은 봉사 몫이며, 예비 목록이나 키를 지우지 않는다. 4명 이하로 내려가면 카운터는 0이고 합류 자격은 계속 유지된다.
- 시험: `crates/node/tests/devnet.rs`에서 후보 등록을 하지 않은 제네시스 예비 키 3개가 4명 위원회의 팔로워로 대기하고, 위원 한 명이 두 에포크 내내 오프라인이면 다음 에포크 안에 빈자리 하나만 채워 체인이 계속 확정하는지 확인한다. 자리를 채우는 규칙 자체와 4명 대기·5명 퇴장 경계는 유닛·beacons 시험(`crates/node/src/rotation.rs`, `crates/node/tests/beacons.rs`)이 확인한다. 야간 생존과 봉사 몫 만료는 `crates/node/tests/liveness.rs`, `mainnet_rules.rs`가 확인한다.

## 절차

| 단계 | 명령 | 비고 |
|---|---|---|
| 1. 키 만들기 | `scripts/reserve-keys.sh init` | `~/aether-reserve/{1,2,3}`. 기존 키는 덮어쓰지 않는다. 공개 항목과 `aether network … --reserve` 줄을 출력한다 |
| 2. 제네시스 | `aether network … --node-rewards --registrar <hex> --reserve-operator <창업자 주소> --reserve ~/aether-reserve/1/validator.pub.json …` | 그 뒤 제네시스 DKG |
| 3. 백업 | `~/aether-reserve/*/validator.key`를 오프라인에 보관. 예비 키가 좌석에 앉아 있으면 **`threshold.json`도**(재공유가 성공할 때마다 다시): 아래 "창업자 Mac을 영구히 잃으면" | 키를 잃으면 그 자리는 투표하지 못한다 |
| 4. 설치 | `scripts/reserve-keys.sh install <DKG가 쓴 network.json>` | LaunchAgent `com.pipln.eastsea.reserve.{1,2,3}`. `--print`는 plist만 보여 주고 설치하지 않는다 |
| 5. 확인 | `scripts/reserve-keys.sh status` | 에이전트, 높이, `voting`/`following` |
| 제거 | `scripts/reserve-keys.sh uninstall` | 키와 체인 데이터는 남는다 |

- `install`은 network.json에 committee identity가 있는지, 세 키가 모두 `"reserve"`에 있는지 먼저 확인한다.
- 바이너리는 `~/aether-reserve/bin/aether`로 복사하고 Developer ID로 서명한다. launchd가 외장 볼륨(`/Volumes`)의 파일을 못 읽는 경우가 있어서다.
- 포트: 키 i의 p2p는 `19200 + 2i`, 재공유는 그 다음 번호, RPC는 `18700 + i`다. `RESERVE_P2P_BASE`와 `RESERVE_RPC_BASE`로 바꾼다. 앱 노드(18545, 19101)나 테스트넷(8601~, 9101~)과 겹치지 않는다.
- 로그: `~/aether-reserve/reserve{1,2,3}.log`.

## 잠자기 방지

- plist는 `/usr/bin/caffeinate -s aether run … --exit-with-parent`를 실행한다. 전원이 연결된 동안 Mac이 잠들지 않는다. 디스플레이는 꺼져도 된다. `pmset` 설정은 바꾸지 않는다.
- caffeinate가 죽으면 `--exit-with-parent` 때문에 노드도 1초 안에 끝난다. launchd(KeepAlive)가 둘 다 다시 띄운다.
- 테스트넷 검증자도 같다. `scripts/testnet-launchagent.sh`는 같은 래퍼를 쓰고, `scripts/testnet.sh start`는 노드마다 `caffeinate -s -w <pid>`를 붙인다.
- 배터리로 돌 때는 잠들 수 있다. 예비 키 Mac은 전원을 연결해 둔다.
- 앱은 이 Mac이 검증자인 동안 `PreventUserIdleSystemSleep` assertion을 잡는다. "Only while on the power adapter"를 켰다면 전원이 연결된 동안만 잡는다. 설정 창에 한 줄 안내가 있다.

## 멈춤 알림

- 앱: 이 Mac이 검증자이고 60초 동안 확정된 블록이 없으면 "The network is paused" 알림을 한 번 보낸다. 재개되면 한 번 더 보낸다.
- soak 모니터(`scripts/soak/monitor.sh`): 마지막 확정 블록이 30초(`AETHER_SOAK_FINALITY`)보다 오래되면 알린다. 문제마다 시작할 때와 풀릴 때 한 번씩만 알린다. 열린 문제는 `~/aether-soak/open/`에 있다.

## 창업자 Mac을 영구히 잃으면

예비 키가 좌석을 채우는 동안 이 Mac은 확정 정족수의 일부다(최대 3석 = 4석 위원회의 3-of-4 정족수 전부를 혼자 쥔다). 이 절은 그 Mac을 영구히 잃었을 때의 개요다(2026-09-29, [12-launch-plan.md](../design/12-launch-plan.md) 빠른 메인넷 조건 (8), [15-node-rewards.md](../design/15-node-rewards.md) "한계"). `AETHER_RECOVER_CONSENSUS`([consensus-recovery.md](consensus-recovery.md))는 과거 복구의 투표 기록을 재시작하는 것일 뿐(2026-10-04 감사 4 A4-3 이후 새 복구 시작은 거부된다) 위원회나 share를 바꾸지 못하므로 여기서는 소용이 없다.

| 상황 | 복구 |
|---|---|
| 예비 키가 1석이거나 앉아 있지 않았다 | 할 일이 없다. 남은 3석으로 정족수(3-of-4)가 나므로 체인은 계속 확정하고, 다음 에포크 경계의 로스터가 자격 Mac으로 그 자리를 다시 채운다 |
| 예비 키가 2석 이상(남은 검증자만으로 정족수 없음) | 체인이 멈춘다. 복구는 백업뿐이다: 새 Mac에 그 키들의 `validator.key`와 **앉아 있던 마지막 재공유의 `threshold.json`**을 옮기고 `install <network.json>`로 세 노드를 띄운다. 예비 키가 사라진 상태로는 재공유가 완료될 수 없었으므로(옛 위원회 딜러 정족수가 안 나온다), 마지막 재공유 때의 share는 여전히 유효하다 — 노드들이 같은 마지막 확정 높이에 서면 정족수가 살아나 체인이 이어진다 |
| 백업(`validator.key`·`threshold.json`)도 없다 | 옛 위원회 딜러 정족수를 채울 수 없어 재공유도 불가능하다. 마지막 확정 상태를 앵커로 새 DKG(새 identity)로 다시 여는 수밖에 없는데, 지갑이 고정 신뢰하는 위원회 identity가 바뀌므로 사실상 네트워크 재시작이다. 이것이 위 백업을 출시 조건으로 둔 이유다 |

- 백업은 **재공유가 성공할 때마다** 다시 떠야 한다: 앉아 있는 동안 `threshold.json`이 그때마다 바뀐다. 지금은 수동이다 — `scripts/reserve-keys.sh`가 아직 `threshold.json` 백업을 다루지 않는다(남은 일).
- 봉사 몫이 들어가는 창업자 주소는 이 Mac이 아니어도 된다. 여기서 복구하는 것은 검증자 키뿐이다.
- 예방: 이 Mac은 전원을 연결해 두고(위 "잠자기 방지"), 잃어버릴 일이 생기면(분실·수리) 그때의 `validator.key`·`threshold.json` 백업이 마지막 상태인지 먼저 확인한다.
