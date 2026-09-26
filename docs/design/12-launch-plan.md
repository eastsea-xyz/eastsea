# 12. 출시 계획: 공개 testnet부터 mainnet 판단까지 (AI 시대 기준)

작성: 2026-09-26. 순서대로 진행합니다. 각 단계는 **완료 기준**을 충족해야 다음으로 넘어갑니다. 🧑 표시는 사람(사용자)이 해야 하는 일입니다.

## 원칙

- **시간 대신 증거.** 몇 달짜리 실제 운영 대신 결정론적 시뮬레이션으로 수년 분량의 네트워크 시간을 재현 가능하게 돌리고, 실제 운영(soak)은 짧게 보조로 둡니다.
- **사람 감사 대신 상시 적대 검증.** 여러 모델의 교차 감사, 퍼징, 모델 체크, 레드팀 에이전트를 매 커밋·상시로 돌립니다. 사람 감사는 마지막 확인입니다.
- **AI가 대신할 수 없는 것은 따로 관리합니다.** 독립 운영자 확보, 법적 판단, 실사용 증거가 여기에 해당합니다.
- **mainnet은 testnet을 승격하지 않고 새 제네시스로 엽니다.** 토큰에 가치를 붙일지는 🧑가 결정합니다. 붙이지 않으면 안정된 testnet이 최종 형태입니다.

## 단계

| # | 단계 | 완료 기준 | 상태 |
|---|---|---|---|
| 1 | **결정론적 시뮬레이션 soak** (`crates/node/tests/sim.rs`): Commonware deterministic runtime과 simulated p2p로 검증자 N대, 패킷 손실·지연·파티션·크래시 재시작·비잔틴(침묵) 주입 | 시드별 재현 가능. 안전성(확정 충돌 0), 활동성(장애 해소 후 진행), 모든 노드 상태 루트 일치. 기본 시드 묶음이 CI에서 통과. 긴 soak(수십만 블록)은 ignored 테스트 | ✅ 기본 4개 시나리오 통과(가상 90초가 약 7초). 긴 soak: `AETHER_SIM_SEEDS`·`AETHER_SIM_SECS` |
| 2 | **적대 감사 파이프라인** (`scripts/audit.sh`): 변경분을 Claude·Codex 등 여러 모델이 독립 검토 후 교차 검증, 퍼징(cargo-fuzz: 트랜잭션 디코드, BAL, 증명), Quint 모델 체크 | 스크립트 한 번으로 실행. 확인된 결함 0이어야 머지 | ✅ `scripts/audit.sh`, `tests/robustness.rs`. Quint 명세는 아직 없음(설계 문서에만 언급) → 12단계 전에 작성 |
| 3 | **testnet 제네시스 정리**: 공개 dev 키 제거, faucet 계정 키는 운영자 Mac Secure Enclave, faucet 속도 제한, 체인 ID 확정 | 공개 키로 인출할 수 있는 잔액 0. faucet은 주소·기기당 제한 | ✅ `aether faucet-key`, `network --faucet`, RPC `aether_faucet`(주소당 24시간, 전역 초당 1회). 제네시스는 faucet만 충전. 로컬 devnet만 공개 dev 키. 기기당 제한은 7단계. faucet 키는 파일(0600)이고 Secure Enclave 이전은 8단계 Mac 노드 앱에서 |
| 4 | **지갑 복구 강화**: 복수 가디언(k-of-n), 48시간 타임락, 주인 취소, 새 소유 키 추가(주소 유지) | 컨트랙트·FFI·앱 테스트. 탈취 시나리오(가디언 단독 즉시 인출 불가) 테스트 | ✅ AetherAccount v2 + 세션 키(에이전트 한도 온체인 강제). 감사 결함 11건 수정 |
| 5 | **업그레이드 매니페스트**: 버전·활성화 높이·바이너리 해시를 위원회 임계 서명으로 확정, 노드는 서명이 유효할 때만 해당 높이에서 규칙 전환 | 운영자 단독으로 규칙을 바꿀 수 없음. 시뮬레이션에서 무중단 전환 확인 | ✅ `upgrade-sign/combine/verify`, 노드·팔로워가 서명된 업그레이드를 읽고 구버전이면 활성 전에 정지. 무중단 전환(새 바이너리가 높이에서 규칙 전환)은 첫 실제 업그레이드 때 |
| 6 | **증명 체인 연결**: D6 해시 결정(BLAKE3 + Jolt Metal), 노드에 Jolt 검증기, R3 에스크로 지급 연결 | 제출된 청크 증명을 검증자가 검증하고 첫 유효 증명에 지급. 증명 지연 지표 노출 | |
| 7 | **기기 신원**: 기기 1대 = 1개 신원(순위 C4, 등록) | 기기당 1회 등록, 재설치로 우회 불가 | 🔶 확인: Mac의 App Attest는 Developer ID(DMG) 배포에서 불가(프로파일에 권한 없음, 실행 시 강제 종료). **DeviceCheck는 Developer ID 앱에서 동작**(토큰 생성 확인) → Mac은 DeviceCheck, iPhone 앱은 App Attest. 🧑 DeviceCheck 키(.p8) 필요 |
| 8 | **Mac 노드 앱**: 메뉴바 앱으로 검증자·prover 원클릭 참여, 전원·발열 인지, Notarize | 새 Mac에서 설치부터 합의 참여까지 5분 이내 | |
| 9 | **공개 testnet 출시** 🧑: 호스트 준비(poc-m3 디스크 또는 poc-cuda), 새 제네시스 DKG, 지갑·에이전트 배포 | 외부 경로로 지갑 송금·복구·에이전트 결제 동작 | 🔶 2026-09-26 이 Mac에서 새 제네시스로 가동(체인 7778, 검증자 4, faucet). 같은 날 `scripts/testnet-reset.sh`로 재시작: R1′ 수수료, 투표 노드 레지스트리, DeviceCheck 등록기(검증자 1), 네 검증자 모두 `aether run`(launchd). 앱 0.3.0이 새 network.json을 싣고, 노드 스위치를 켠 뒤 Join으로 투표 노드 후보가 된다. DHT 발견·faucet·팔로워 검증 확인. 지갑 송금·에이전트 결제는 사용자 Touch ID 필요. 상시 가동은 이 Mac이 켜져 있을 때만 → 외부 검증자 합류 필요(11) |
| 10 | **레드팀 에이전트 상시 운영**: 이중지불·검열·수수료 조작·복구 탈취·스팸 시나리오를 testnet에 계속 실행 | 발견 결함은 1·2단계 회귀 테스트로 고정 | 🔶 `tests/redteam.rs`(실제 네트워크 대상): 재전송·타 체인·서명 변조·수수료 미달·증명 예산 0·오버플로·잔액 초과·스팸·faucet 반복 10종 거부, 검증자 합의 유지 확인. 상시 실행 스케줄은 남음 |
| 11 | **외부 검증자 온보딩** 🧑: 서로 다른 운영자·ISP 7대 이상 | 한 운영자가 1/3 이상을 갖지 않음 | 🧑 |
| 12 | **R4·R5**: 참여 기록(threshold 인증서에는 서명자가 드러나지 않으므로 투표 수집 방식 설계 필요), 비양도 작업 영수증 | 영수증으로 기여 조회 가능 | |
| 13 | **실제 운영 soak**: 1~2주, 가용성·확정 지연·증명 지연 SLO 측정 | SLO 충족, 리셋 없음 | |
| 14 | **go/no-go** 🧑: 가치 부여 여부와 법적 검토(증권성, 관할 규제, 스테이블코인 인가) | 🧑 결정 | 🧑 |

## 패키징: 완성된 제품으로 배포 (Transmission 방식)

목표: **Aether.app 하나**를 DMG로 받아 Applications에 끌어다 놓으면 끝. 지갑과 노드가 한 앱이고, 노드는 스위치로 켜고 끈다. 끄면 흔적 없이 멈춘다.

| # | 단계 | 완료 기준 |
|---|---|---|
| P1 | **팔로워 노드 모드**: 검증자가 아닌 Mac도 확정 블록과 인증서를 받아 전부 재실행·검증하고, 지갑에 로컬로 응답 | 새 Mac이 제네시스부터 따라잡고, 상태 루트가 검증자와 일치. 지갑이 이 로컬 노드만으로 검증 ✅ `aether follow` |
| P2 | **앱 번들 통합**: 지갑 앱 안에 `aether`(노드)와 `aether-agent`를 Helpers로 포함. 노드 켜기/끄기, 전원 연결 시에만 실행, 로그인 시 시작 옵션, "명령줄 도구 설치" 메뉴 | 앱을 끄면 노드도 정리되어 종료. 데이터는 `~/Library/Application Support/Aether` ✅ 노드 스위치, 동기화 후 지갑 전환, 부모 종료 시 노드 종료, 설정: 전원 연결 시만 실행(배터리에서 일시정지), 로그인 시 열기 |
| P3 | **서명·공증·DMG**: Developer ID 서명, Hardened Runtime, `notarytool` 공증과 staple, 배경 이미지와 Applications 바로가기가 있는 DMG | 다른 Mac에서 Gatekeeper 경고 없이 설치·실행 ✅ Pipln Developer ID 서명, Apple 공증 Accepted, staple, `spctl`: Notarized Developer ID |
| P4 | **자동 업데이트**: Sparkle 2(EdDSA 서명 appcast, GitHub Releases 호스팅) | 이전 버전이 새 버전을 감지하고 설치 | ✅ Sparkle 2(EdDSA), `scripts/release-mac.sh`, 2026-09-26 `app-v0.2.0` 게시(공증 DMG + appcast) |
| P5 | **배포 채널**: GitHub Releases, Homebrew cask, iOS는 TestFlight | `brew install --cask aether` 동작 |

## 생태계 트랙: 누구나 토큰·컨트랙트·DeFi

| # | 단계 | 완료 기준 |
|---|---|---|
| E1 | **이더리움 JSON-RPC 호환**: eth_sendRawTransaction(secp256k1, EIP-1559/7702), eth_call, eth_estimateGas, eth_getTransactionReceipt, eth_getLogs, eth_getBlockByNumber | MetaMask·Foundry·Hardhat·Remix로 배포·호출 |
| E2 | **토큰 발행**: ERC-20 팩토리 제네시스 배포, 지갑 "토큰 만들기" | 코드 없이 민트, 지갑에 표시 |
| E3 | **DEX**: 블록 단위 배치 경매(단일 청산가, MEV 차단) + 집중 유동성 AMM, 자체 구현(라이선스) | 스왑·유동성 공급을 지갑에서 |
| E4 | **익스플로러·TS SDK** (브라우저에서 인증서 검증) | 누구나 조회·개발 |

## 노드 역량 트랙: Mac 전용의 이점

| # | 단계 | 완료 기준 |
|---|---|---|
| C1 | **노드 스펙·회선 측정**: 칩, GPU 코어, 통합 메모리, Metal·CPU 벤치, 업·다운 대역폭. App Attest로 기기에 묶어 보고 | 노드 등록에 스펙, 표준 증명 과제로 교차 확인 |
| C2 | **스펙 기반 배정**: 청크 크기를 Mac 등급별로, prove gas 한도는 가장 느린 위원의 실측으로 | 증명 지연 SLO 충족 |
| C3 | **느린 회선 대응**: erasure-coded 블록 전파(Commonware coding), 대역폭 기반 검증자 선정, 느린 회선 Mac은 prover·팔로워로 | 업로드가 약한 검증자가 있어도 1초 블록 유지 |
| C4 | **연속 기여 순위**(토크노믹스 §7): streak, 유예, 최대 2배 | 끊기면 순위 초기화 |
| C5 | **열린 투표 노드**(07-consensus.md "열린 위원회"): 후보 등록·생존 신호 트랜잭션, 에포크별 결정적 선출, 운영자당 1/3 미만 상한, 자동 재공유 (`aether run`) | ✅ devnet에서 제네시스 집합이 후보 집합으로 사람 없이 교체됨. 남은 것: 앱의 DeviceCheck 등록 연결, testnet을 `aether run`으로 전환, 겹침(무정지) 재공유 |

## 사람이 결정·수행할 것 (🧑)

1. testnet 호스트 준비(9번).
2. `aether-agent init` 실행(Touch ID)과 에이전트 도구 등록 여부.
3. 외부 검증자 운영자 섭외(11번).
4. 토큰 가치 부여와 법적 판단(14번).
5. 앱 서명 주체(P3): 이 Mac에는 Developer ID 인증서가 Pipln(45WU468FZE) 것만 있음. Pipln 명의로 배포할지, 개인 팀(LBKUT88RTX)용 Developer ID를 새로 만들지. forecast-network처럼 가치 0을 유지하면 14번은 "안 함"으로 끝납니다.
