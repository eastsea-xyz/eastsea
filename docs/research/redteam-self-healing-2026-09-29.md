# 자가 회복 설계 레드팀 검토

지정된 설계 문서와 소스 파일을 모두 읽었다. 현재 [자가 회복 설계](/Volumes/workspace/aether-node/docs/design/24-self-healing.md:8)는 출시 조건을 제시하지만, 아래 항목 중 상당수는 아직 구현되지 않았다. 복구할 수 없는 장애에서 지갑의 검증된 원격 조회를 유지하고 사용자에게 한 문장으로 조치를 안내한다는 원칙은 적절하다. 다만 **검증자 키의 영구 소실이나 macOS가 앱 실행 자체를 차단하는 상황은 앱 혼자 원상복구할 수 없다.** 이 경우에도 잘못된 신원으로 계속 투표하지 않고 안전하게 기능을 낮추는 것이 출시 조건이어야 한다.

## 1. 자식 노드의 무제한 재시작

**심각도: High.** **구체적 시나리오:** 설계는 "10분에 3번 넘게 죽으면 멈춤"을 요구하지만, 앱이 실행하는 `aether run`은 자식 `node` 또는 `follow`가 종료될 때마다 2초 뒤 재시작한다. 부모 프로세스는 살아 있으므로 앱의 종료 처리에는 잡히지 않는다. 영구적인 DB 오류나 잘못된 바이너리로 자식이 계속 죽으면 전력과 로그 공간을 소모한다. 앱도 부모가 종료되면 30초 전원 타이머에서 다시 시작할 수 있다. [설계](/Volumes/workspace/aether-node/docs/design/24-self-healing.md:16), [Supervisor](/Volumes/workspace/aether-node/crates/node/src/supervisor.rs:164), [NodeController](/Volumes/workspace/aether-node/apps/wallet/Sources/NodeController.swift:97)

**구체적 개선안:** 종료 원인을 자식→Supervisor→앱까지 전달하고, 역할별 재시작 횟수와 지수 백오프를 영속화한다. 디스크 부족·키 오류·프로토콜 부족은 조건이 바뀔 때까지 재시작하지 않는다. 한도 도달 시 로컬 노드를 중지하고 지갑은 원격 노드로 전환한다.

## 2. 정상적으로 느린 검증자를 죽이는 60초 감시 기준

**심각도: Critical.** **구체적 시나리오:** 설계의 "네트워크 높이는 오르는데 내 노드가 60초 제자리면 재시작"은 아직 앱에 구현되지 않았다. 구현 시 스냅샷 검증·대량 재실행 중인 검증자도 멈춘 것으로 판정할 수 있다. 현재 검증자는 투표 전 따라잡기를 시도하지만 120초가 지나면 경고 후 계속 시작하며, 팔로워는 합류 인계 시 최대 120초 동안 의도적으로 진행을 보류한다. 이때 강제 재시작이 반복되면 따라잡기를 끝내지 못해 합의 활동성이 악화된다. [설계](/Volumes/workspace/aether-node/docs/design/24-self-healing.md:17), [main.rs](/Volumes/workspace/aether-node/crates/node/src/main.rs:1495), [follow.rs](/Volumes/workspace/aether-node/crates/node/src/follow.rs:301)

**구체적 개선안:** 높이 하나가 아니라 `동기화·스냅샷 검증·합의 대기·저장소 쓰기` 단계별 진행 신호와 마지막 성공 시각을 노출한다. 검증된 복수 피어의 높이를 비교하고, 정당한 장기 작업에는 단계별 기한을 준다. 재시작 전에 해당 역할의 투표 가능 상태와 합의 정족수 영향을 확인하며, 감시 자체의 반복 재시작에도 상한을 둔다.

## 3. 프로토콜 업그레이드 뒤 구버전으로 되돌리기

**심각도: High.** **구체적 시나리오:** 설계는 새 바이너리의 반복 종료 시 이전 바이너리로 되돌리자고 한다. 체인은 높이 기반 업그레이드를 사용한다. **현재 코드에는 구버전이 새 규칙으로 조용히 투표하지 않도록 종료 코드 3으로 멈추는 보호가 있어, 이 경로만으로 합의 분열이 발생한다고 단정할 수는 없다.** 그러나 활성화 뒤 롤백하면 구버전 자식이 종료하고 Supervisor가 2초마다 다시 띄워 영구적으로 투표하지 못한다. [설계](/Volumes/workspace/aether-node/docs/design/24-self-healing.md:18), [출시 계획](/Volumes/workspace/aether-node/docs/design/12-launch-plan.md:20), [main.rs](/Volumes/workspace/aether-node/crates/node/src/main.rs:1756)

**구체적 개선안:** 롤백 허용 여부를 설치 시점이 아니라 **체인의 현재·예정 프로토콜과 이전 바이너리의 지원 범위**로 결정한다. 이전 버전이 활성 규칙을 지원하지 않으면 롤백을 금지하고 투표를 중단한 채 서명된 호환 업데이트를 재시도한다. 새 버전의 기동 성공뿐 아니라 저장소 열기·RPC 준비·체인 따라잡기까지 확인한 뒤 업데이트를 확정한다.

## 4. redb 상태와 Commonware 합의 저널의 서로 다른 손상

**심각도: Critical.** **구체적 시나리오:** 설계는 "데이터 손상 시 옆으로 옮기고 스냅샷 점프"라고 한데 묶는다. redb는 시작 시 상태 루트를 재계산하지만, 투표 저널은 별도 Commonware 파티션이다. 합의 저널이 손상됐는데 상태 DB와 함께 폐기하고 같은 검증자 키로 재시작하면 과거 투표 기록을 잃는다. 반대로 상태 DB만 재구성했을 때도 저널의 확정 높이와 맞는지 검사가 필요하다. 현재 `Engine::new`는 아카이브 초기화에 `expect`를 쓰고, 재실행 오류가 나면 경고 후 중단한다. 명시적 합의 복구 경로도 운영자 지정 환경변수에 의존한다. [설계](/Volumes/workspace/aether-node/docs/design/24-self-healing.md:13), [store.rs](/Volumes/workspace/aether-node/crates/node/src/store.rs:644), [engine.rs](/Volumes/workspace/aether-node/crates/node/src/engine.rs:296), [engine.rs](/Volumes/workspace/aether-node/crates/node/src/engine.rs:316)

**구체적 개선안:** 상태 DB, 확정 블록 아카이브, 투표 저널을 별도 장애 등급으로 진단한다. 상태만 손상되면 인증된 스냅샷에서 복구하되 투표 저널은 보존한다. 투표 저널의 안전한 복원이 불가능하면 그 키로 투표를 재개하지 말고 팔로워로 내려가 다음 안전한 키·에포크 절차를 기다린다. 세 저장소의 높이·해시·에포크 일치 검사를 재시작 게이트로 둔다.

## 5. 검증자 키 파일 소실 또는 부분 손상

**심각도: Critical.** **구체적 시나리오:** "키 파일은 절대 지우지 않는다"는 예방책일 뿐, 이미 사라진 키는 복구하지 못한다. `CandidateKeys::load_or_create`는 기존 키를 읽는 데 실패해도 새 키 생성을 시도한다. 파일이 통째로 사라지면 기존 온체인 등록과 다른 검증자 신원이 만들어진다. `threshold.json`이 없거나 맞지 않는 경우 검증자 시작 경로는 오류 또는 패닉으로 끝날 수 있다. [설계](/Volumes/workspace/aether-node/docs/design/24-self-healing.md:13), [candidate.rs](/Volumes/workspace/aether-node/crates/node/src/candidate.rs:47), [main.rs](/Volumes/workspace/aether-node/crates/node/src/main.rs:2151)

**구체적 개선안:** "최초 설치"를 영속 표식으로 구분해 등록된 신원의 키가 사라지거나 파싱에 실패하면 **자동 생성 금지**한다. 검증자 기능을 중지하고 지갑·팔로워 기능만 유지한다. 복구 가능한 백업 여부를 확인하고, 불가능하면 사용자에게 새 신원 등록과 기존 등록 해제 절차를 안내한다. 키 저장은 임시 파일·동기화·원자적 교체로 부분 쓰기를 막는다.

## 6. 시스템 시계 오차와 시간 점프

**심각도: Medium.** **구체적 시나리오:** 60초 정지 감시는 설계에만 있고 시간 기준이 정의되지 않았다. 현재 앱은 `Date`로 투표 상태 조회 간격과 네트워크 정지 표시를 계산한다. 사용자가 시계를 크게 앞당기거나 뒤로 돌리면 향후 감시·백오프를 벽시계로 구현할 경우 즉시 재시작하거나 영원히 기다릴 수 있다. 다만 현재 지갑의 정지 표시는 오래된 블록 시각만으로 발동하지 않고 높이 정지도 함께 요구하므로, **시계 오차만으로 네트워크 정지를 표시한다는 주장은 해당하지 않는다.** [설계](/Volumes/workspace/aether-node/docs/design/24-self-healing.md:17), [NodeController](/Volumes/workspace/aether-node/apps/wallet/Sources/NodeController.swift:277), [WalletModel](/Volumes/workspace/aether-node/apps/wallet/Sources/WalletModel.swift:372)

**구체적 개선안:** 감시 기한·재시작 백오프에는 단조 시계를 사용하고, 잠자기·깨우기 뒤 관측 기준을 재설정한다. 벽시계 기반 정보는 별도 진단으로 표시하며, 시간 오차가 의심될 때는 사용자에게 시스템 시간 확인을 안내한다.

## 7. 디스크가 가득 찬 상태에서 스냅샷 복구

**심각도: High.** **구체적 시나리오:** 설계는 남은 공간 5GB 미만에서 쓰기·증명을 멈춘다고 하지만 지정된 코드에는 그 검사나 쓰기 정지가 없다. 스냅샷은 최대 1GiB를 메모리에 받은 뒤 redb에 기록하고, 기존 체인으로 점프할 때는 이전 키 삭제와 새 키 쓰기를 한 트랜잭션으로 수행한다. redb는 시작 시 압축까지 시도한다. 공간 부족 상태에서 재시작·압축·스냅샷 설치를 반복하면 원래 장애를 악화시킨다. 스냅샷 커밋 자체는 원자적이므로 **부분 설치가 정상 체크포인트로 보인다고 주장할 근거는 없다.** [설계](/Volumes/workspace/aether-node/docs/design/24-self-healing.md:14), [follow.rs](/Volumes/workspace/aether-node/crates/node/src/follow.rs:82), [snapshot.rs](/Volumes/workspace/aether-node/crates/node/src/snapshot.rs:178), [store.rs](/Volumes/workspace/aether-node/crates/node/src/store.rs:308)

**구체적 개선안:** 복구 전 예상 스냅샷·redb 복사 쓰기·임시 파일 크기를 합산한 여유 공간을 확인하고, 고정 복구 여유분을 예약한다. 부족하면 다운로드·압축·증명을 중단하고 원격 지갑으로 유지한다. 사용자가 공간을 확보한 뒤 충분한 여유가 **연속해서** 확인될 때만 재개하며, ENOSPC에서는 짧은 재시도를 금지한다.

## 8. macOS 업데이트 뒤 Keychain·Secure Enclave 접근 실패

**심각도: Critical.** **구체적 시나리오:** 지갑 키는 Secure Enclave에 있고 디스크에는 불투명 핸들이 저장된다. 앱은 핸들을 읽거나 복원하는 데 실패하면 오류 원인을 구분하지 않고 새 Secure Enclave 키를 만든 뒤 같은 파일에 쓴다. macOS 업데이트 직후 잠금·권한·Keychain의 일시적 접근 오류라면 기존 주소의 핸들을 새 것으로 덮을 수 있다. `WalletModel`이 키 로드를 재시도하는 점은 좋지만 이 생성 경로를 막지 못한다. [EnclaveKey](/Volumes/workspace/aether-node/apps/wallet/Sources/EnclaveKey.swift:34), [WalletModel](/Volumes/workspace/aether-node/apps/wallet/Sources/WalletModel.swift:106)

**구체적 개선안:** 핸들 파일이 **존재하는 한** 읽기·복원 실패 시 새 키를 절대 생성하지 않는다. 잠금·사용자 승인 거절·영구 키 무효화를 구분해 재시도하거나 복구 절차를 안내한다. 새 키 생성은 최초 설치임이 확인됐거나 사용자가 명시적으로 새 지갑을 선택했을 때만 허용한다.

## 9. 잠자기·깨우기와 전원 전환

**심각도: High.** **구체적 시나리오:** 앱은 검증자일 때 idle sleep 방지 assertion을 잡고, 기본 설정에서는 배터리로 바뀌면 노드를 멈춘다. 이는 일부 상황을 이미 완화한다. 그러나 뚜껑 닫기·강제 잠자기는 별개이고, 전원 확인은 30초 타이머에 의존한다. 깨어난 뒤 검증자 따라잡기는 120초 제한을 넘기면 "시작 anyway"로 진행한다. 네트워크 단절 후에도 오래된 투표 상태가 앱에 남을 수 있다. [NodeController](/Volumes/workspace/aether-node/apps/wallet/Sources/NodeController.swift:97), [NodeController](/Volumes/workspace/aether-node/apps/wallet/Sources/NodeController.swift:370), [main.rs](/Volumes/workspace/aether-node/crates/node/src/main.rs:1495)

**구체적 개선안:** 시스템 sleep/wake·전원 변경 알림에서 즉시 연결과 건강 상태를 무효화한다. 깨운 검증자는 인증된 최신 높이와 상태를 확인할 때까지 투표 준비 상태를 거짓으로 표시한다. 따라잡기 제한이 끝나도 자동으로 정상 검증자라고 표시하지 말고 관측·재시도 상태를 유지한다.

## 10. 앱 전치 실행과 격리 속성

**심각도: Medium.** **구체적 시나리오:** 출시 계획은 공증된 DMG를 Applications에 복사하는 경로를 가정한다. 앱 코드는 현재 번들 안 `Contents/Helpers/aether`의 실행 가능 여부만 검사하고 `Process.run()` 오류를 일반 실패 문구로 보여 준다. 사용자가 DMG에서 직접 실행하거나 격리·전치 상태에서 Helper 실행이 막힐 때 특별한 복구 안내가 없다. **앱 자체가 Gatekeeper에 의해 실행되지 않는 경우에는 앱 내부 감시기로 해결할 수 없다.** [출시 계획](/Volumes/workspace/aether-node/docs/design/12-launch-plan.md:31), [NodeController](/Volumes/workspace/aether-node/apps/wallet/Sources/NodeController.swift:126), [NodeController](/Volumes/workspace/aether-node/apps/wallet/Sources/NodeController.swift:179)

**구체적 개선안:** 설치·첫 실행 검사에서 앱 위치, 서명·공증, Helper 실행 가능성을 확인한다. Helper만 막히면 지갑을 원격 모드로 두고 "Applications에 앱을 설치한 뒤 다시 열어 주세요"처럼 실행 가능한 조치를 표시한다. 배포 시험에는 DMG 직접 실행과 격리된 다운로드를 포함한다.

## 11. Sparkle 업데이트의 다운로드·설치 실패

**심각도: High.** **구체적 시나리오:** 업그레이드가 감지되면 앱은 Sparkle의 백그라운드 확인을 호출하고, `upgradeAsked`를 한 번 참으로 바꾼다. 실패 결과를 받아 재시도하는 상태 기계는 지정 코드에 없다. 프로토콜이 활성화되면 노드는 종료 코드 3으로 멈추지만 Supervisor는 같은 구버전 자식을 반복 실행한다. Sparkle 서명 검증은 출시 계획에 있어도 네트워크 단절·디스크 부족·설치 권한 오류를 자동 해결하지는 않는다. [NodeController](/Volumes/workspace/aether-node/apps/wallet/Sources/NodeController.swift:331), [AetherWalletApp](/Volumes/workspace/aether-node/apps/wallet/Sources/AetherWalletApp.swift:179), [main.rs](/Volumes/workspace/aether-node/crates/node/src/main.rs:1790), [출시 계획](/Volumes/workspace/aether-node/docs/design/12-launch-plan.md:40)

**구체적 개선안:** 업데이트를 `발견→다운로드→서명·프로토콜 검증→설치→새 노드 건강 확인`의 영속 상태로 관리한다. 실패 원인별 제한된 재시도와 대체 다운로드 경로를 두고, 성공 전에는 업데이트 요구 알림을 재발행할 수 있게 한다. 활성 프로토콜을 지원하지 않는 동안 검증자 재시작은 중단하고 지갑만 원격 모드로 유지한다.

## 12. 앱 두 인스턴스의 동시 실행

**심각도: High.** **구체적 시나리오:** `NodeController`의 `process == nil` 검사는 자기 앱 인스턴스 안에서만 유효하다. 두 앱이 같은 데이터 디렉터리와 고정 RPC·P2P 포트로 각각 `aether run`을 실행할 수 있다. 두 번째 프로세스는 포트 또는 redb 열기에서 실패할 수 있고, 그 실패를 재시작 루프가 증폭한다. 여기서 두 프로세스가 실제로 같은 저널에 이중 투표한다고 단정할 자료는 없지만, 단일 소유권을 보장하는 코드도 없다. [NodeController](/Volumes/workspace/aether-node/apps/wallet/Sources/NodeController.swift:116), [NodeController](/Volumes/workspace/aether-node/apps/wallet/Sources/NodeController.swift:144), [main.rs](/Volumes/workspace/aether-node/crates/node/src/main.rs:643)

**구체적 개선안:** 앱 시작 전에 데이터 디렉터리 단위의 배타적 프로세스 잠금을 획득하고, 실행 중인 기존 인스턴스에는 IPC로 창 열기 요청만 보낸다. 자식 실행 전에도 동일한 잠금을 검증한다. 잠금 충돌은 "이미 실행 중"으로 분류해 재시작하지 않는다.

## 13. 파일 디스크립터 고갈

**심각도: Medium.** **구체적 시나리오:** 이 문제는 **일부 이미 처리됐다.** 노드는 시작 시 soft `RLIMIT_NOFILE`을 hard limit와 macOS 상한 안에서 올리고, 투표 저널 섹션이 128개를 넘으면 경고한다. 다만 저널은 정지한 체인에서 view마다 파일이 늘며 시작 시 각각을 열고, 한도를 올릴 수 없거나 섹션이 계속 증가하면 경고만으로 고갈을 막지 못한다. [main.rs](/Volumes/workspace/aether-node/crates/node/src/main.rs:1284), [engine.rs](/Volumes/workspace/aether-node/crates/node/src/engine.rs:62), [engine.rs](/Volumes/workspace/aether-node/crates/node/src/engine.rs:335)

**구체적 개선안:** 시작 전 예상 FD 수를 실제 한도와 비교해 여유가 없으면 투표 시작을 거부하고 원인을 앱에 전달한다. 안전한 저널 정리·보존 정책과 정지 시 view 증가 상한을 설계한다. hard limit가 낮은 환경을 출시 시험에 포함한다.

## 14. 살아 있지만 멈춘 프로세스

**심각도: High.** **구체적 시나리오:** Supervisor는 자식이 종료됐는지만 확인한다. RPC·합의 엔진·저장소 작업이 멈춰도 프로세스가 살아 있으면 계속 기다린다. 앱의 2초 검사는 로컬 높이 조회에 실패하면 아무 상태 변경 없이 반환하며, 이미 한 번 로컬 노드로 전환했다면 높이 상승 여부와 무관하게 `running`으로 표시한다. [Supervisor](/Volumes/workspace/aether-node/crates/node/src/supervisor.rs:189), [NodeController](/Volumes/workspace/aether-node/apps/wallet/Sources/NodeController.swift:345)

**구체적 개선안:** 자식에 RPC 응답, 이벤트 루프, 마지막 저장 커밋, 마지막 합의 처리, 마지막 피어 통신의 독립적인 heartbeat를 둔다. 특정 구성요소만 멈췄다면 먼저 그 기능을 제한하고, 기한이 지난 경우에만 재시작한다. RPC 무응답 즉시 지갑의 로컬 경로를 해제한다.

## 15. DB 마이그레이션의 중간 실패

**심각도: High.** **구체적 시나리오:** 지정된 `Store`에는 현재 명시적 스키마 버전 마이그레이션이 없다. 시작 시 테이블을 열고 압축한 다음, 새 버전의 저장소를 이전 바이너리로 되돌릴 수 있다는 설계가 별도 조건 없이 제시된다. 향후 새 릴리스가 테이블·인코딩을 변경하고 디스크 부족으로 중간에 실패하면, 재시작과 바이너리 롤백 모두 같은 DB를 열지 못할 수 있다. 따라서 **현존하는 마이그레이션 버그가 아니라 출시 설계의 빠진 조건**이다. [store.rs](/Volumes/workspace/aether-node/crates/node/src/store.rs:287), [store.rs](/Volumes/workspace/aether-node/crates/node/src/store.rs:135), [설계](/Volumes/workspace/aether-node/docs/design/24-self-healing.md:18)

**구체적 개선안:** 스키마 버전·최소 읽기 가능 바이너리 버전을 DB에 기록한다. 마이그레이션은 공간 사전검사와 원자적 단계 표식을 갖추고, 재실행 가능하게 만든다. 롤백 바이너리가 새 스키마를 읽지 못하면 실행을 금지한다. 각 단계에서 전원 차단·ENOSPC를 주입해 재시작 시험을 한다.

## 16. 저장소 쓰기 오류 뒤 같은 블록 재시도

**심각도: High.** **구체적 시나리오:** 문서의 발단은 redb 쓰기 실패 뒤 같은 블록 288회 재시도다. 현재 팔로워 경로는 `chain.finalize` 오류 시 그 블록을 채택하지 않고 반환하지만, 바깥 루프가 400ms 뒤 다시 시도한다. 저장소 핸들이 "다시 열어야 함" 상태라면 오류가 계속된다. 설계가 제안한 "닫고 다시 열기"도 체인 메모리 상태와 영속 체크포인트를 함께 재구성한다는 조건이 빠져 있다. [설계](/Volumes/workspace/aether-node/docs/design/24-self-healing.md:6), [follow.rs](/Volumes/workspace/aether-node/crates/node/src/follow.rs:341), [follow.rs](/Volumes/workspace/aether-node/crates/node/src/follow.rs:462)

**구체적 개선안:** 저장 오류를 일시적 네트워크 오류와 분리해 즉시 동일 높이 재시도를 중단한다. DB 객체와 체인 메모리 상태를 폐기하고 디스크 체크포인트에서 새로 열어 일치성을 확인한다. ENOSPC 등 지속 조건이면 여유 공간이 돌아올 때까지 대기하고 원격 지갑으로 전환한다.

## 17. 로컬 노드가 뒤처져도 지갑이 계속 붙잡는 문제

**심각도: High.** **구체적 시나리오:** 앱은 로컬 높이가 네트워크와 2블록 이내가 되면 지갑을 로컬로 전환한다. 그 뒤에는 `switched`가 참이라는 사실만으로 `running`을 유지하고 네트워크 높이도 더 이상 조회하지 않는다. 노드가 살아 있으나 추적을 멈추면 지갑은 낡은 로컬 노드를 계속 사용한다. 지갑 쪽의 "확정 블록은 뒤로 가지 않는다" 검사는 이미 본 잔액의 후퇴를 막지만 새 확정 블록을 놓치는 문제까지 해결하지는 않는다. [NodeController](/Volumes/workspace/aether-node/apps/wallet/Sources/NodeController.swift:350), [WalletModel](/Volumes/workspace/aether-node/apps/wallet/Sources/WalletModel.swift:360)

**구체적 개선안:** 전환 뒤에도 검증된 원격 높이와 로컬 높이를 주기적으로 비교한다. 허용 지연 또는 RPC 실패 기한을 넘으면 즉시 로컬 경로를 해제하고, 충분한 기간 다시 따라잡았을 때만 복귀한다. UI에는 마지막 검증 높이와 동기화 상태를 구분해 표시한다.

## 18. 검증되지 않은 원격 높이가 복구 판단을 흔드는 문제

**심각도: Medium.** **구체적 시나리오:** 팔로워의 `net_height`는 첫 응답의 `aether_status.height` 숫자를 읽는다. 블록·스냅샷 자체는 인증서로 검증하므로 **거짓 높이만으로 잘못된 상태를 채택하지는 않는다.** 하지만 결함 있는 피어가 과장된 높이를 보내면 불필요한 스냅샷 점프 시도와 반복 다운로드를 유발할 수 있다. 이 값을 설계의 60초 감시 기준에도 그대로 쓰면 정상 노드를 재시작시킬 수 있다. [follow.rs](/Volumes/workspace/aether-node/crates/node/src/follow.rs:174), [follow.rs](/Volumes/workspace/aether-node/crates/node/src/follow.rs:397), [follow.rs](/Volumes/workspace/aether-node/crates/node/src/follow.rs:400)

**구체적 개선안:** 높이 판단은 여러 독립 피어에서 받은 **인증된 최근 블록**에 묶고, 비정상적으로 앞선 응답은 격리한다. 스냅샷 점프와 감시 재시작의 근거를 단일 미인증 상태 응답으로 삼지 않는다.

## 19. 위원회 인계 파일의 중간 설치 실패

**심각도: Critical.** **구체적 시나리오:** 자동 역할 전환은 새 `threshold.json`을 쓰거나 기존 share를 지운 뒤 `network.json`을 쓴다. 중간에 전원 차단·ENOSPC가 나면 새 share와 옛 네트워크, 또는 share가 없는 옛 네트워크가 남을 수 있다. 다음 실행에서 키 라운드 불일치로 종료하거나, 새 위원회에 뽑힌 노드가 투표하지 못한다. 이는 설계의 일반 DB 손상 복구와 별개의, 합의 구성 전환 장애다. [Supervisor](/Volumes/workspace/aether-node/crates/node/src/supervisor.rs:368), [Supervisor](/Volumes/workspace/aether-node/crates/node/src/supervisor.rs:414), [main.rs](/Volumes/workspace/aether-node/crates/node/src/main.rs:2165)

**구체적 개선안:** share·network·anchor를 하나의 세대별 디렉터리에 준비하고 각각 동기화한 뒤 활성 세대 포인터를 원자적으로 교체한다. 시작 시 완료 표식과 온체인 인계 높이·라운드를 비교해 미완료 설치를 안전하게 재개하거나 팔로워로 내린다. 이전 share 폐기는 새 세대 활성화 확인 뒤 수행한다.

## 20. 증명 사이드카의 오류가 잘못된 증명 거부로 바뀌는 문제

**심각도: High.** **구체적 시나리오:** 검증 사이드카는 무응답이면 죽이고 다시 띄우는 장치가 있다. 다만 `verify`는 "프로세스 종료"나 쓰기 실패로 분류되지 않은 응답 오류를 `false`로 바꾼다. 예를 들어 응답 JSON 파싱 오류나 임시 출력 파일 문제는 유효한 증명의 거부처럼 처리될 수 있다. 프로토콜 2에서 검증자가 증명 포함 블록을 계속 거부하면 활동성이 떨어진다. [prover.rs](/Volumes/workspace/aether-node/crates/node/src/prover.rs:97), [prover.rs](/Volumes/workspace/aether-node/crates/node/src/prover.rs:132), [prover.rs](/Volumes/workspace/aether-node/crates/node/src/prover.rs:202)

**구체적 개선안:** `유효`, `무효`, `검증기 장애`를 구조화된 결과로 분리한다. 파싱·I/O·타임아웃은 무효 증명으로 간주하지 않고 제한된 재기동 후 검증자를 일시적으로 투표 불가 상태로 둔다. 시작 정보 파싱에 실패한 자식도 반드시 종료·회수한다.

## 21. 증명 작업의 반복 실패와 자원 압박

**심각도: Medium.** **구체적 시나리오:** 설계는 메모리 압박에서 증명을 멈추고 메모리 초과 시 백오프한다고 하나, 현재 증명 서비스는 실패 뒤 5초 쉬고 다시 시도한다. 증명 요청 제한은 30분 고정이며, 장시간 정상 작업과 메모리 압박을 구분하는 상태가 보이지 않는다. 같은 작업이 환경 문제로 계속 실패하면 GPU·전력·디스크를 반복 소비할 수 있다. [설계](/Volumes/workspace/aether-node/docs/design/24-self-healing.md:14), [prover.rs](/Volumes/workspace/aether-node/crates/node/src/prover.rs:44), [prover.rs](/Volumes/workspace/aether-node/crates/node/src/prover.rs:303)

**구체적 개선안:** 증명은 합의·지갑보다 낮은 우선순위로 두고 메모리·전원·디스크 압박 시 작업을 보류한다. 실패 원인별 지수 백오프와 작업별 재시도 한도를 적용한다. 장시간 정상 증명에는 진행 신호를 받아 정해진 절대 타임아웃만으로 종료하지 않는다.

출시 시험 목록은 현재 ENOSPC·DB 손상·증명기 종료 등 일부 사례만 열거한다. 위 항목을 **장애 주입 후 검증된 높이 회복, 투표 안전성, 원격 지갑 지속, 재시작 횟수와 전력 상한**으로 판정하는 시험으로 확장해야 한다. 이번 검토는 읽기 전용으로 수행했으며 파일 수정이나 실행 시험은 하지 않았다.