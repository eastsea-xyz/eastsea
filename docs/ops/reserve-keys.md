# 창업자 예비 키 운영

규칙은 [15-node-rewards.md](../design/15-node-rewards.md) "창업자 예비 키"에 있다. 이 문서는 창업자 Mac 한 대에서 예비 키 3개를 실제로 띄우는 방법이다.

## 동작

- 예비 키마다 `aether run` 프로세스가 하나씩 돈다. 데이터 폴더와 포트가 따로다.
- 투표 세트 밖일 때: 검증하는 팔로워로 체인을 따라간다. 등록 후보가 아니라 비콘을 보내지 않는다. 보상도 없다.
- 규칙이 예비 키를 넣을 때(위원회가 4석에 못 미치고 독립 운영자도 4명 미만): 제안된 세트에 자기 키가 있으면 팔로워인 채로 재공유에 참여한다. 4석에 모자란 자리만 들어온다. 인계가 확정되면 전환 높이부터 검증자로 투표한다. 사람이 할 일은 없다.
- 규칙이 예비 키를 뺄 때(4명 이상, 또는 위원회가 4석으로 서면): 몫(`threshold.json`)을 지우고 다시 팔로워가 된다.
- 시험: `crates/node/tests/devnet.rs`의 `founder_reserve_keys_stay_followers_over_a_full_committee`. 등록하지 않은 예비 키 3개가 `aether run`으로만 돌아도 제네시스 4석이 서 있는 한 하나도 들어오지 않고 팔로워로 남으며, 체인은 멈추지 않는다. 자리를 채우는 규칙 자체는 유닛·beacons 시험(`crates/node/src/rotation.rs`, `crates/node/tests/beacons.rs`)이 확인한다.

## 절차

| 단계 | 명령 | 비고 |
|---|---|---|
| 1. 키 만들기 | `scripts/reserve-keys.sh init` | `~/aether-reserve/{1,2,3}`. 기존 키는 덮어쓰지 않는다. 공개 항목과 `aether network … --reserve` 줄을 출력한다 |
| 2. 제네시스 | `aether network … --node-rewards --registrar <hex> --reserve-operator <창업자 주소> --reserve ~/aether-reserve/1/validator.pub.json …` | 그 뒤 제네시스 DKG |
| 3. 백업 | `~/aether-reserve/*/validator.key`를 오프라인에 보관 | 키를 잃으면 그 자리는 투표하지 못한다 |
| 4. 설치 | `scripts/reserve-keys.sh install <DKG가 쓴 network.json>` | LaunchAgent `com.pipln.aether.reserve.{1,2,3}`. `--print`는 plist만 보여 주고 설치하지 않는다 |
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
