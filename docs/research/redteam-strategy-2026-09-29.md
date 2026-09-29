# 레드팀: 성공 방안과 소비자·에이전트 지갑 포지션 (Codex, 2026-09-29)

팀장 제안(성공 방안)과 창업자 입력("해적팀, 소비자에게 답", "에이전트에게 맡기는 지갑")에 대한 두 차례 검토 원문.

## 1차: 성공 방안
# 1. 진단이 맞는가

**심각도: 높음.** 팀장안의 “범위가 너무 넓은데 사용자가 0명”이라는 진단은 방향이 맞다. 출시 계획에는 체인과 지갑뿐 아니라 DEX, 런치패드, 금고, 이름 서비스, 교차체인까지 올라와 있다. 반면 테스트넷 7780의 검증자 네 개는 모두 창업자 Mac 한 대에서 돌고 있으며, 외부 후보 등록과 실제 외부 검증자 운영은 구별해야 한다. 지금 입증된 것은 제품의 폭과 기술적 진척이지, 독립 운영자나 결제 고객의 수요가 아니다. [팀장안](/private/tmp/claude-501/-Volumes-workspace-aether-node/2274cf73-7af2-437d-8a06-ab75e6c24a76/scratchpad/success-plan.md:3), [로드맵](/Volumes/workspace/aether-node/docs/design/13-roadmap.md:10), [출시 계획](/Volumes/workspace/aether-node/docs/design/12-launch-plan.md:43)

다만 “가장 큰 위험은 기술이 아니다”는 결론은 이르다. 저장소의 분석 문서는 메인넷 제네시스에서 예정한 프로토콜 규칙이 켜지지 않는 **G1**, 잔액 0인 새 지갑이 고정 팁 때문에 등록 거래를 못 하는 **G2**를 치명적 결함으로 분류한다. 이는 분석 문서의 코드 점검 결과이며 이 검토에서 코드를 재검증한 판정은 아니다. 별도로 자가 회복 설계에는 디스크 부족 때 노드가 같은 블록을 288번 재시도한 실제 사건이 적혀 있다. 제 판단으로 사업 수요와 핵심 안전성은 둘 다 출시 차단 요인이다. [구현 분석](/Volumes/workspace/aether-node/docs/03-analysis/aether-node.analysis.md:5), [자가 회복 설계](/Volumes/workspace/aether-node/docs/design/24-self-healing.md:3)

# 2. ‘Mac as validator’ 및 가치제안 검증 — 누가, 왜 참여하는가

**심각도: 치명적.** “집에 있는 Mac이 곧 검증자”는 기억하기 좋은 제품 설명이지만 참여 이유까지 설명하지는 못한다. 실제 규칙은 DeviceCheck 등록, 24시간 연속 참여, 위원회 선출을 거친다. Mac을 켜 둔다고 곧바로 투표석을 얻는 것은 아니다. 토큰 판매·사전 발행·창업자 배분이 없다는 점은 신뢰의 출발점이지만, 가치와 현금화 수단을 약속하지 않는다는 [README의 고지](/Volumes/workspace/aether-node/README.md:97) 아래에서는 전기·기기·운영 부담을 감수할 이유를 따로 찾아야 한다. [팀장안](/private/tmp/claude-501/-Volumes-workspace-aether-node/2274cf73-7af2-437d-8a06-ab75e6c24a76/scratchpad/success-plan.md:5), [출시 계획](/Volumes/workspace/aether-node/docs/design/12-launch-plan.md:22), [면책 문서](/Volumes/workspace/aether-node/DISCLAIMER.md:18)

제 시장 판단으로, 가격이 없는 현재 자발적으로 참여할 가능성이 높은 사람은 **Mac 분산 시스템을 시험하고 싶은 개발자**, **검증자 운영 경험 자체를 원하는 홈서버 사용자**, **창업자를 신뢰하는 초기 협력자** 정도다. 이들은 1,000명 규모의 보상 고객층과 다르다. DePIN과 비교해도 “Mac을 보유했다”는 공급 능력일 뿐, 네트워크가 제공하는 희소한 유료 서비스는 아직 입증되지 않았다. 저장소의 인센티브 조사도 켜져 있음만 보상하기보다 실제 제공한 작업을 보상하는 사례를 짚는다. [팀장안](/private/tmp/claude-501/-Volumes-workspace-aether-node/2274cf73-7af2-437d-8a06-ab75e6c24a76/scratchpad/success-plan.md:7), [노드 인센티브 조사](/Volumes/workspace/aether-node/docs/research/node-incentives-2026.md:46)

특히 “1 Mac = 1표”와 “운영자당 1/16”을 사람 단위 분산으로 홍보하면 안 된다. 보상 설계는 **운영자를 지갑 주소로 센다**고 명시한다. 한 사람이 실제 Mac 여러 대와 주소 여러 개를 쓰면 여러 몫과 ‘독립 운영자’ 수를 확보할 수 있다. DeviceCheck가 물리적 기기 확보 비용을 높일 수는 있어도 실소유자 독립성을 증명하지는 않는다. 대안은 초기 모집 문구를 보상보다 실험 목적과 운영 비용에 맞추고, 참여 의향뿐 아니라 **타인 소유 기기에서 2주 이상 실제 운영한 비율**로 가치제안을 검증하는 것이다. [보상 설계](/Volumes/workspace/aether-node/docs/design/15-node-rewards.md:30), [예비 키 규칙](/Volumes/workspace/aether-node/docs/design/15-node-rewards.md:256)

# 3. 런칭 조건 현실성 및 적합성 검토

**심각도: 높음.** 팀장안의 “외부 운영자 16명+, 30일 사람 개입 0, 독립 보안 감사 1회+”는 현재의 외부 검증자 0명 상태에서 한 번에 달성할 근거가 없다. 게다가 기존 문서의 ‘빠른 메인넷’ 조건은 **서로 다른 제네시스 기계 3곳 이상, 최종 코드 테스트넷 7일, 교차 *모델* 감사 1회**다. 같은 출시 계획 안에도 30일 테스트넷, 감사 2회, 출시 후 버그 바운티가 서로 다른 결정으로 남아 있다. 팀장안이 기준을 강화하려는 것은 이해되지만, 어떤 출시를 뜻하는지부터 통일해야 한다. [팀장안](/private/tmp/claude-501/-Volumes-workspace-aether-node/2274cf73-7af2-437d-8a06-ab75e6c24a76/scratchpad/success-plan.md:6), [출시 계획](/Volumes/workspace/aether-node/docs/design/12-launch-plan.md:94), [로드맵](/Volumes/workspace/aether-node/docs/design/13-roadmap.md:66)

**16은 그 자체로 보안 증거가 아니다.** 후보 수가 아니라 실제 투표석의 실소유자, 전원·회선·관리 키의 독립성이 필요하다. 문서는 16석에서 “체인을 멈추려면 6명, 잘못 확정하려면 11명이 공모”라고 적지만, 11표씩인 두 정족수의 최소 교집합은 `11 + 11 − 16 = 6`이다. 따라서 상충하는 확정에 필요한 악의적 서명자 수를 11로 설명하는 것은 잘못된 안전성 메시지다. 한편 창업자 예비 키 세 개가 한 Mac에 있는 동안에는 세 좌석이 하나의 장애 영역이다. [출시 계획](/Volumes/workspace/aether-node/docs/design/12-launch-plan.md:104), [예비 키 규칙](/Volumes/workspace/aether-node/docs/design/15-node-rewards.md:262)

**30일 ‘무개입’도 잘못된 단일 지표다.** 변경이 없어서 안정된 30일은 장애 후 안전한 복구 능력을 입증하지 못한다. 반대로 검증자가 위험하게 자동 재시작해도 겉보기 가동률은 좋아질 수 있다. 자가 회복 레드팀은 투표 저널 손상 후 같은 키로 재투표하거나, 느린 정상 검증자를 감시자가 재시작하는 시나리오를 치명적으로 본다. 먼저 장애 주입에서 **안전한 투표 중단, 검증된 높이 회복, 지갑 지속 사용, 복구 시간**을 통과시키고, 이후 서로 다른 실소유자·회선의 운영 기록을 측정해야 한다. 가치가 오갈 메인넷이라면 독립 감사도 보고서 한 장이 아니라 범위 확정, 수정, 재검증까지 완료해야 한다. [자가 회복 설계](/Volumes/workspace/aether-node/docs/design/24-self-healing.md:26), [자가 회복 레드팀](/Volumes/workspace/aether-node/docs/research/redteam-self-healing-2026-09-29.md:23)

# 4. AI 에이전트 결제 유스케이스의 신뢰성

**심각도: 높음.** 기술 데모로서는 설득력이 있다. `aether-agent`에는 송금·일괄 지급·영수증 도구가 있고, Secure Enclave 키와 Touch ID로 설정한 온체인 한도를 사용한다. 그러나 팀장안의 “수수료 0, 1초 확정”을 곧바로 상업 결제 가치로 옮길 수는 없다. README는 에이전트 가스가 **별도 소액 잔액**에서 나온다고 설명하고, 수수료 0은 목표 부하 이하의 기본료 조건에 달려 있다. 혼잡 시 대납 풀은 아직 설계이며, 현재 다중 Mac 인터넷 환경의 처리량·확정 지연 측정도 로드맵의 다음 일이다. [AGENTS.md](/Volumes/workspace/aether-node/AGENTS.md:1), [README](/Volumes/workspace/aether-node/README.md:60), [가스 풀 설계](/Volumes/workspace/aether-node/docs/design/22-gas-pool.md:8), [로드맵](/Volumes/workspace/aether-node/docs/design/13-roadmap.md:28)

더 큰 문제는 **누가 AETH를 받아 무엇에 쓰는가**다. HTTP 402형 API 결제는 출시 계획에서 장래 항목이고, 현재 문서에는 반복 결제를 받겠다는 외부 서비스 제공자의 증거가 없다. 시장 가치와 교환 경로가 없는 AETH를 받은 판매자는 비용을 회수하기 어렵다. 제 판단으로 첫 검증 대상은 “에이전트 지갑을 만들 수 있다”가 아니라 “외부 API 제공자 한 곳이 실제 가격·환불·영수증 조건으로 이 결제를 반복 수락한다”여야 한다. 그 전에는 **테스트넷의 지출 한도 실험**으로 정직하게 위치시켜야 한다. [출시 계획](/Volumes/workspace/aether-node/docs/design/12-launch-plan.md:57), [팀장안](/private/tmp/claude-501/-Volumes-workspace-aether-node/2274cf73-7af2-437d-8a06-ab75e6c24a76/scratchpad/success-plan.md:8), [README](/Volumes/workspace/aether-node/README.md:9)

# 5. 아토믹 스왑과 유동성 — 실질적 exit이 있는가

**심각도: 치명적.** 없다. HTLC는 **이미 합의한 상대방과 두 자산을 안전하게 교환하는 절차**다. AETH를 살 상대, 실행 가능한 호가, 재고, 거래 규모가 없으면 현금화 출구가 생기지 않는다. 교차체인 설계 자체도 ETH·SOL·BTC 아토믹 스왑을 **메인넷 후 첫 확장**으로 놓고, 메이커 봇을 상대방으로 가정한다. 관련 조사 문서는 HTLC 단독 경로의 온라인 대기, 자본 잠김, 그리핑을 이유로 보조 수단으로만 평가한다. 따라서 팀장안의 “첫 출구”는 설계 일정과 시장 조건 양쪽에서 성립하지 않는다. [팀장안](/private/tmp/claude-501/-Volumes-workspace-aether-node/2274cf73-7af2-437d-8a06-ab75e6c24a76/scratchpad/success-plan.md:8), [교차체인 설계](/Volumes/workspace/aether-node/docs/design/21-crosschain.md:19), [교차체인 조사](/Volumes/workspace/aether-node/docs/research/crosschain-2026.md:62)

대안은 스왑 구현 전에 잠재 메이커에게 **왜 AETH 재고를 보유할지, 어느 가격·수량·스프레드에서 호가를 낼지** 확인하는 것이다. 재고를 댈 상대가 없으면 “exit” 표현을 삭제해야 한다. 이는 README의 “현금화 방법을 약속하지 않는다”는 고지와도 일치한다. [README](/Volumes/workspace/aether-node/README.md:99)

# 6. 법·규제 및 Apple 플랫폼 리스크

**심각도: 치명적.** 전면 면책은 사업 허가나 법적 분류를 대신하지 않는다. DISCLAIMER는 “법이 허용하는 최대 범위”에서 책임을 제한하고, 운영자에게 법규 준수 책임을 둔다. 동시에 출시 계획에는 “변호사 대신 Codex 자문”이라는 결정이 있다. 저장소의 법률 문구 검토는 이를 메인넷 전 변호사 확인이 필요한 공백으로 지목한다. 무판매·무프리마인·무수수료는 유리한 사실일 수 있지만, 등록기 운영, 공식 DEX 화면, 런치패드, 보상 홍보가 한데 묶인 실제 운영 형태에 대한 판단은 별개다. 이는 **법률 결론이 아니라 이 저장소 문서들 사이의 위험 평가**다. [DISCLAIMER](/Volumes/workspace/aether-node/DISCLAIMER.md:46), [출시 계획](/Volumes/workspace/aether-node/docs/design/12-launch-plan.md:109), [법률 문구 검토](/Volumes/workspace/aether-node/docs/research/legal-copy-review-2026-09.md:92)

문서 내부에서도 법적 입장이 충돌한다. 한 법률 검토는 공식 본딩커브 런치패드 제외를 권하지만, 더 늦은 출시 계획은 메인넷에 직접 제공하겠다고 적는다. 검증 보고서는 AETH의 비증권 가능성을 매우 강하게 표현하는 반면, 원 검토는 창업자 승인권과 홍보 방식 때문에 확정 의견을 내기 어렵다고 한다. 이 AI 작성 문서들을 외부 법률의견이나 규제 승인처럼 사용해서는 안 된다. 특히 “초기 참여자 이점”을 강조하는 팀장안의 모집 문구는 자체 보상 설계가 금지한 “지금 들어오면 유리” 표현에 가까워진다. 관할권과 기능 범위를 좁힌 정식 자문을 홍보 및 가치가 오가는 출시보다 앞에 두는 편이 안전하다. [법률 검토](/Volumes/workspace/aether-node/docs/research/legal-review-2026.md:25), [추가 검토](/Volumes/workspace/aether-node/docs/research/legal-review-2026-verified.md:174), [런치패드 결정](/Volumes/workspace/aether-node/docs/design/12-launch-plan.md:114), [보상 문구](/Volumes/workspace/aether-node/docs/design/15-node-rewards.md:62)

**Apple 위험은 별도로 높음.** Mac 앱을 Developer ID로 배포하면 App Store 심사 경로를 피할 수는 있다. 그러나 등록은 Pipln의 서비스가 Apple DeviceCheck 토큰을 확인하는 구조이고, 문서 스스로 DeviceCheck가 어느 Mac과 투표 키가 계속 결합돼 있는지 증명하지 못한다고 인정한다. Apple의 API·권한·배포 정책 변경은 후보 등록에 직접 영향을 준다. iPhone 지갑에는 별도의 App Store 심사 조건이 있으며, 저장소의 심사 조사도 조직 계정과 기기 내 채굴 금지를 주요 조건으로 든다. 공증은 승인 보증이 아니다. 따라서 Apple 의존성을 “하드웨어로 해결된 탈중앙성”으로 홍보하기보다 **등록 중단 시 기존 위원회가 얼마나 유지되는지, 새 후보를 어떻게 받을지** 운영상 실패 시나리오로 다뤄야 한다. [README](/Volumes/workspace/aether-node/README.md:103), [보상 설계](/Volumes/workspace/aether-node/docs/design/15-node-rewards.md:237), [App Store 조사](/Volumes/workspace/aether-node/docs/research/app-store-review-2026.md:31)

# 7. 단일 창업자·키맨 리스크

**심각도: 치명적.** 현재 네 검증자가 한 Mac에 있으므로 그 기계의 손실이 곧 체인 중단이다. 계획은 서명 전용 Mac과 예비 키로 위험을 줄이려 하지만, 예비 키 세 개도 한 사람의 통제·운영 절차에 남는다. 출시 계획은 예비 키가 정족수에 걸린 상태에서 창업자 Mac을 영구히 잃으면 체인이 멈출 수 있음을 인정한다. 등록 서비스, 앱·업데이트 서명, 예비 검증자 키를 옮기는 것은 개발용 Mac의 침해 범위를 줄이는 조치이지 키맨 위험의 해소는 아니다. [로드맵](/Volumes/workspace/aether-node/docs/design/13-roadmap.md:24), [출시 계획](/Volumes/workspace/aether-node/docs/design/12-launch-plan.md:105)

솔로 창업자와 AI 코딩 팀이라는 운영 구조에서는 **누가 키를 복구하고, 결함을 판단하고, 공지를 내고, 비용을 댈지**가 비어 있다. 사전 발행·판매·개발 기금도 없으므로 독립 감사와 장기 유지비의 재원은 팀장안에 제시되지 않았다. 대안은 창업자 부재·계정 잠김·기기 손실을 가정한 실제 복구 리허설, 외부 공동 유지관리자와 권한 분리, 공개된 사고 대응 책임자와 감사 예산이다. 독립 운영자 모집은 이 통제권 이전까지 확인해야 한다. [팀장안](/private/tmp/claude-501/-Volumes-workspace-aether-node/2274cf73-7af2-437d-8a06-ab75e6c24a76/scratchpad/success-plan.md:9), [출시 계획](/Volumes/workspace/aether-node/docs/design/12-launch-plan.md:115), [구현 분석](/Volumes/workspace/aether-node/docs/03-analysis/aether-node.analysis.md:153)

# 8. 빠진 것, 잘라낼 것, 작업 순서

**심각도: 높음.** 팀장안에는 성공을 판정할 고객 지표가 없다. “첫 1,000명”보다 먼저 필요한 것은 외부 운영자의 **설치 완료율·2주 잔존율·실제 투표석 독립성**, 결제 상대방의 **반복 수락**, 그리고 스왑 상대방의 **실행 가능한 호가**다. 공개 대시보드는 이 수치를 보여 줄 수 있지만 수요를 만들어 주지는 않는다. 1분 설치도 기존 노드 앱 목표인 5분보다 공격적이며, DeviceCheck·동기화·위원회 선출까지 포함한 실측 정의가 필요하다. [팀장안](/private/tmp/claude-501/-Volumes-workspace-aether-node/2274cf73-7af2-437d-8a06-ab75e6c24a76/scratchpad/success-plan.md:7), [출시 계획](/Volumes/workspace/aether-node/docs/design/12-launch-plan.md:22)

당장은 **공식 런치패드와 DEX 확장, 금고·이름 서비스, 브리지·아토믹 스왑, 그룹 분할, 공용 대납 풀의 3층**을 메인넷 필수 범위에서 빼는 것이 맞다. 이는 영구 폐기 권고가 아니라, 외부 수요와 기본 안전성을 입증할 때까지의 순서 변경이다. 런치패드의 법적 위험과 계획 간 충돌, 아직 설계인 대납 실행 경로, 후속 단계로 적힌 교차체인이 근거다. 먼저 **외부 운영자가 안전하게 참여할 수 있는 단일 체인과 지갑**을 완성해야 한다. [출시 계획](/Volumes/workspace/aether-node/docs/design/12-launch-plan.md:68), [가스 풀 설계](/Volumes/workspace/aether-node/docs/design/22-gas-pool.md:46), [교차체인 설계](/Volumes/workspace/aether-node/docs/design/21-crosschain.md:19)

마지막으로 홍보 초안을 출시 근거로 사용하지 말아야 한다. `launch-posts`의 머리말은 App Attest 서술이 틀렸다고 정정하지만 본문에는 여전히 App Attest 기반 Mac 증명과 사람 단위 1/16 상한이 남아 있다. `device-node-landscape`도 하드웨어 결합과 산업용 팜 차단을 강하게 단정하지만, 최신 보상 설계는 기기·키 결합과 실소유자 판별의 한계를 인정한다. 공개 전에는 최신 설계와 일치하는 주장만 남겨야 한다. [홍보 초안](/Volumes/workspace/aether-node/docs/research/launch-posts-2026.md:1), [시장 조사](/Volumes/workspace/aether-node/docs/research/device-node-landscape-2026.md:190), [보상 설계](/Volumes/workspace/aether-node/docs/design/15-node-rewards.md:237)

## 향후 4주 Top 5

1. **실수요를 먼저 확인한다.** 창업자와 무관한 Mac 운영자 후보, API 제공자, 잠재 스왑 메이커를 각각 만나 실제 설치·결제 수락·호가 제공 의사를 조건과 함께 기록한다. 관심 표명과 실제 운영을 따로 센다.
2. **메인넷 핵심 결함과 주장 불일치를 닫는다.** 분석 문서의 G1·G2를 재현·수정·회귀 검증하고, 16석 안전성 설명, 출시 조건표, 홍보 초안을 하나의 최신 규칙으로 맞춘다.
3. **외부 테스트넷을 소수의 독립 운영자로 시험한다.** 소스·비밀정보 스캔 후 검토 가능한 배포물을 제공하고, 서로 다른 실소유자와 장애 영역의 Mac에서 투표, 기기 손실, 인계, 재시작을 측정한다.
4. **안전과 법률의 외부 검토 범위를 확정한다.** 합의·등록기·발행·업데이트·복구에 대한 독립 감사 범위와 수정 재검증 예산을 잡고, 실제 제공할 기능과 관할권에 맞는 변호사 검토를 받는다.
5. **출시 결정을 단계화한다.** 연구용 공개 테스트넷, 독립 정족수를 갖춘 제한적 파일럿, 가치가 오가는 메인넷의 기준을 각각 명시한다. 외부 운영 지속성과 결제 수락이 확인되기 전에는 스왑을 ‘출구’로 발표하지 않는다.

## 2차: 소비자·에이전트 지갑 포지션
## 배경 요약

[기존 성공 계획](/private/tmp/claude-501/-Volumes-workspace-aether-node/2274cf73-7af2-437d-8a06-ab75e6c24a76/scratchpad/success-plan.md:3)은 Mac 운영 보상과 에이전트 결제를 AETH의 사용처로 묶었다. 새 방향은 소비자에게 기술이 아니라 완성된 답을 주고, 지갑을 “AI 비서에게 안심하고 맡길 수 있는 돈”으로 설명하자는 것이다. 실제 [에이전트 구현](/Volumes/workspace/aether-node/apps/agent/Sources/Tools.swift:17)은 한도가 있는 **AETH 송금**과 DEX **조회**를 제공한다.

## 핵심 판단

**현재 일반 소비자에게 성립하는 단일 유료 제품 답은 없다.** 에이전트 지갑은 가격·출구·가맹점 문제를 우회하지 않고 결제 순간으로 옮긴다. 비서에게 맡긴 AETH가 얼마짜리인지 알 수 없고, 잘못 충전했을 때 빠져나올 경로가 입증되지 않았으며, 비서가 지불해 실제로 끝낼 구매도 제시되지 않았다. 계획서의 원자적 스왑은 제안이고, 현재 `dex_quote`는 견적만 낸다. [앱의 현행 고지](/Volumes/workspace/aether-node/apps/wallet/Sources/Onboarding.swift:21)도 테스트넷 AETH는 이월되지 않으며 가격·수익·현금화 경로를 약속하지 않는다고 밝힌다.

세 문제를 넘어설 **조건부 소비자 답**은 “내 AI 비서가 내가 정한 달러 한도 안에서 필요한 유료 서비스를 대신 사고, 나는 내역을 이해하고 즉시 멈추고 문제를 해결할 수 있다”이다. 이는 현재 기능 설명이 아니라 출시 목표다. 제안 문구의 **“Mac이 번 돈”**은 지금 테스트 보상을 돈으로 오인하게 하고, **“안전하게 쓴다”**는 지출 상한을 구매 보호로 오인하게 한다. 홈 화면의 `번 돈 / 비서에게 맡긴 돈 / 보내기`도 테스트 보상과 실제 지불 가능한 잔액을 분리하지 않으면 같은 혼동을 만든다.

## 경쟁 비교

**Coinbase AgentKit/x402.** 일반인은 이미 USDC로 충전하고 유료 서비스를 찾아 결제할 수 있는 Coinbase의 에이전트 지갑 쪽을 고를 이유가 크다. 동반 지갑 화면에는 충전, 서비스 탐색, 거래 내역, 호출·세션별 한도가 있다. Aether의 우위는 Mac 안의 비반출 키와 계정 계약이 강제하는 한도다. 특히 **AgentKit SDK 자체**는 송금 승인·한도·수취인 제한을 기본 제공하지 않는다고 Coinbase가 명시한다. 그러나 Coinbase의 **동반 지갑 제품**까지 그 SDK와 동일시하면 Aether의 차별성을 과장하게 된다. [Coinbase 지갑](https://docs.cdp.coinbase.com/agentic-wallet/mcp/mcp-tools/show-wallet-app), [AgentKit 위험 고지](https://github.com/coinbase/agentkit#️-managing-risk).

**Stripe.** 소비자는 Stripe를 직접 고르기보다 Stripe를 쓰는 상점과 결제 방식을 선택한다. 그 경로에는 카드 또는 USDC 결제, 달러 가격, 구매 영수증, 상점의 환불 처리가 있다. Aether는 특정 결제 사업자에 자금을 맡기지 않는 Mac 중심 통제를 제시할 수 있지만, 현재는 상점·주문·환불 연결이 없다. Stripe를 쓴다는 사실만으로 AI의 오구매 배상까지 보장되는 것은 아니다. [Stripe 기계 결제 문서](https://docs.stripe.com/payments/machine).

**Skyfire.** 일반인이 원하는 것이 “비서가 실제 웹사이트에서 내 카드로 물건을 사는 것”이라면 Skyfire의 제시 방식이 더 직접적이다. Skyfire는 카드·스테이블코인 지갑, 사람의 구매 위임, 사이트 접근부터 결제까지의 흐름을 내세운다. 이는 회사의 제품 설명이며 실제 가맹 범위와 소비자 보호 성과는 별도 검증이 필요하다. Aether는 로컬 키와 체인 강제 한도로 차별화할 수 있지만, 현재는 같은 구매 흐름이 없다. [Skyfire 제품 설명](https://skyfire.xyz/).

**Privy/Turnkey.** 이들은 보통 소비자가 직접 고르는 지갑보다 앱 사업자가 채택하는 지갑 기반이다. 그 기반 위의 앱은 기존 네트워크 자산과 서비스에 접근할 수 있고, Privy는 송금·수취인·계약 정책을, Turnkey는 다중 승인과 에이전트 접근의 즉시 철회를 문서화한다. Aether의 고유점은 **사용자 Mac의** 비반출 키와 **자체 계정 계약의** 한도 결합이다. “안전한 키와 정책”만으로는 이들과 차이가 충분하지 않다. [Privy 정책](https://docs.privy.io/security/wallet-infrastructure/policy-and-controls), [Turnkey 에이전트 지갑](https://docs.turnkey.com/solutions/company-wallets/agentic-wallets).

**Apple Pay.** 보통 소비자는 이미 아는 상점에서 카드로 결제하고, 구매 문제가 생기면 상점에 환불을 요청할 수 있어 Apple Pay를 고른다. 환불은 **Apple이 보장하는 것이 아니라 상점이 처리해 카드로 돌려보내는** 구조다. Aether가 가진 차이는 비서에게 사전에 정한 범위의 자동 지출을 맡길 수 있다는 점이다. 반대로 Apple Pay의 일상적 구매처, 거래별 사람 인증, 상점·카드 발급사와 이어지는 해결 경로는 Aether에 없다. [Apple Pay 환불 안내](https://support.apple.com/en-us/118270).

## 신뢰 메커니즘 평가

- **Critical — 프롬프트 인젝션에 의한 한도 소진.** 에이전트 키는 결제마다 사람 확인 없이 서명한다. [초기 정책](/Volumes/workspace/aether-node/apps/agent/Sources/Owner.swift:18)은 1회 1 AETH·하루 10 AETH이지만 **수취인 누구나, 만료 없음**이다. 외부 문서에 속은 비서는 키를 훔치지 않고도 허용된 전액을 여러 번 보낼 수 있다. 필요 요소는 기본 수취인·용도 제한, 짧은 위임 기간, 구매 대상과 금액의 사람 확인 기준, 이상 지출 차단이다. Secure Enclave는 키 탈취 위험을 줄이지, 속은 비서의 합법적 서명을 막지 않는다.

- **Critical — 사고 책임의 공백.** 계약은 한도 초과를 거절하지만 한도 *안*에서 발생한 오구매의 손실을 누가 부담할지 정하지 않는다. “안전하게 맡긴다”를 소비자에게 말하려면 사고 접수, 책임 기준, 보상 재원 또는 보험 여부가 먼저 명확해야 한다. 검토한 제품 흐름에서는 이를 확인하지 못했다.

- **High — 환불·분쟁 경로 부재.** 현재 결제 도구는 [주소와 AETH 금액](/Volumes/workspace/aether-node/apps/agent/Sources/Tools.swift:24)을 보내며, [영수증](/Volumes/workspace/aether-node/apps/agent/Sources/Tools.swift:215)은 체인 확정 여부를 말할 뿐 무엇을 샀고 제공받았는지 증명하지 않는다. 오송금은 수취인의 자발적 반환 외에 이 흐름에서 해결할 방법이 없다. 상점 식별, 주문·인도 기록, 환불 요청과 분쟁 처리 주체가 필요하다.

- **High — 소비자용 즉시 중지와 설명 가능한 내역 부재.** 계약에는 [세션 제거 함수](/Volumes/workspace/aether-node/contracts/src/AetherAccount.sol:318)가 있지만 현재 `aether-agent`의 소유자 명령은 [정책 조회·변경](/Volumes/workspace/aether-node/apps/agent/Sources/Owner.swift:27)뿐이다. “지금 비서를 멈춰”라는 한 동작이 없다. [지출 내역](/Volumes/workspace/aether-node/apps/agent/Sources/Tools.swift:187)은 확정 전에 로컬 파일에 기록되고 수취 주소·총액·해시만 남겨, 실패 여부나 구매 목적을 설명하지 못한다. 긴급 철회 화면, 알림, 가맹점 이름이 있는 확정 내역이 필요하다.

따라서 **Secure Enclave + Touch ID + 온체인 한도는 필요한 손실 제한 장치이지만 소비자가 돈을 맡길 충분조건은 아니다.** Touch ID도 매 결제 승인이 아니라 [소유자 정책 변경의 인증](/Volumes/workspace/aether-node/apps/agent/Sources/Keys.swift:27)이며, 코드에는 로그인 암호 대체도 명시돼 있다.

## 스테이블코인 필요성 판단

**Aether가 독립적인 에이전트 지갑을 팔려면 USDC 같은 널리 통용되는 안정적 결제 단위가 사실상 필수다.** USDC라는 특정 상표가 논리적으로 유일한 답은 아니다. 카드 기반 위임 결제도 가능하다. 하지만 **AETH만으로는** 달러 가격의 구매 한도를 표시하거나, 서비스 가격과 잔액을 비교하거나, 판매자가 받을 금액을 확정하거나, 소비자가 지출·환불액을 이해할 수 없다. Coinbase는 x402 유료 서비스에 USDC를, Stripe는 카드와 USDC를 지원한다. [Coinbase USDC 결제](https://docs.cdp.coinbase.com/agentic-wallet/cli/skills/pay-for-service), [Stripe 지원 수단](https://docs.stripe.com/payments/machine).

단순히 Aether 체인에 USDC라는 토큰을 올리는 것으로도 부족하다. 현재 [세션 계약](/Volumes/workspace/aether-node/contracts/src/AetherAccount.sol:358)은 **데이터가 없는 기본 코인 송금만** 허용하므로, 토큰 전송 호출은 에이전트가 실행할 수 없다. 판매자가 받는 네트워크와의 결제 경로, 충전·인출, 수수료, 달러 기준 한도까지 갖춰야 한다. 따라서 정직한 첫 유료 버전은 **“USDC로 결제하고, AETH는 별도의 실험적 보상”**에 가깝다. AETH 단독 출시는 테스트 경험일 수는 있어도 일반 소비자용 구매 제품이라고 부르기 어렵다.

## 기술팀처럼 사고하는 지점들

- [성공 계획 5–7행](/private/tmp/claude-501/-Volumes-workspace-aether-node/2274cf73-7af2-437d-8a06-ab75e6c24a76/scratchpad/success-plan.md:5)은 “Mac 검증자”, 운영자 16명, 가동률, 초기 1/16을 앞세운다. 소비자의 첫 질문인 **오늘 무엇을 얻고 전기료를 빼면 얼마이며 어디에 쓰는가**에는 답하지 않는다. 첫 1,000명도 일반 소비자보다 Mac mini·홈서버 애호가로 정의돼 있다.
- [성공 계획 8행](/private/tmp/claude-501/-Volumes-workspace-aether-node/2274cf73-7af2-437d-8a06-ab75e6c24a76/scratchpad/success-plan.md:8)은 스왑을 “출구”, 에이전트 송금을 “쓸 곳”으로 취급한다. 실제 유동성과 실제 판매자가 없는 한 둘 다 소비자에게 완결된 행동이 아니다.
- [AGENTS.md 3–11행](/Volumes/workspace/aether-node/AGENTS.md:3)은 키·계약·MCP 등록 방법을 설명하지만, 비서가 첫날 살 수 있는 **이름 붙은 상품이나 서비스**를 말하지 못한다. `setup all --apply`는 개발 환경 설치 명령이지 소비자 온보딩이 아니다.
- [지갑 스킬 16행과 51–54행](/Volumes/workspace/aether-node/agents/skills/aether-wallet/SKILL.md:16)은 “환불”까지 사용 사례로 쓰면서 실제 예시는 `0x` 주소에 AETH를 보내고 해시를 보고하는 것이다. 소비자가 이해할 상대방, 주문, 환불 요청은 빠져 있다.
- [지갑 스킬 11행](/Volumes/workspace/aether-node/agents/skills/aether-wallet/SKILL.md:11)의 “에이전트가 하는 어떤 일도 한도를 넘을 수 없다”는 말은 보호 범위를 과하게 느끼게 한다. **한도 안의 피해**가 핵심 위험이고, 저장소의 [기존 문구 검토](/Volumes/workspace/aether-node/docs/research/legal-copy-review-2026-09.md:84)도 절대적 표현을 지적했다.
- 새 홈 화면의 **“비서에게 맡긴 돈”**은 예산 설정인지 별도 계정에 이미 옮긴 잔액인지 구분해야 한다. [초기화 코드](/Volumes/workspace/aether-node/apps/agent/Sources/Owner.swift:9)는 실제 에이전트 계정에 먼저 자금을 넣도록 요구한다. `번 돈`은 현 테스트넷 보상을 현금처럼 부르지 않아야 한다.

## 최종 순위

아래는 **출시 후보의 순위**다. 현재 완성된 소비자 제품 세 가지라는 뜻은 아니다.

1. **AI 비서의 용돈 지갑 — “정해 둔 돈으로 필요한 유료 일을 대신 끝내고, 내가 언제든 멈춘다.”** USDC 결제처와 사고 해결 경로를 갖추면 가장 강한 답이다.
   - “나는 비서에게 이번 주 5달러를 주고, 필요한 유료 자료 한 건을 찾아 사오게 한다.”
   - “나는 결제 전에 누구에게 무엇을 얼마에 사는지 보고, 끝나면 받은 결과와 영수증을 확인한다.”
   - “나는 수상한 지출을 보자마자 비서를 멈추고 그 구매의 환불을 요청한다.”

2. **Mac 참여 보상의 실사용 — “켜 둔 Mac이 받은 보상을 가치가 보이는 잔액으로 받아 쓴다.”** 가격·유동성·출구가 입증되기 전에는 출시 문구로 쓸 수 없다.
   - “나는 Mac을 켜 둔 뒤 전기료보다 실제로 얼마나 남았는지 확인한다.”
   - “나는 받은 보상을 비서의 작은 구매 예산으로 옮겨 실제 서비스에 쓴다.”
   - “나는 원할 때 남은 보상을 팔거나 인출한다.”

3. **씨드 문구 없는 Mac 지갑 — “복잡한 복구 문구 없이 Mac에서 내 돈을 보내고 통제한다.”** 현재 기술적 토대는 있으나, 일상 송금에서는 Apple Pay보다 선택할 이유가 약하다.
   - “나는 받는 사람 이름과 금액을 확인하고 돈을 보낸다.”
   - “나는 Mac을 잃어도 미리 설정한 방법으로 잔액 접근을 회복한다.”
   - “나는 잘못 보낸 거래의 상대와 해결 방법을 내역에서 찾는다.”

이번 검토는 읽기 전용으로 진행했다. 파일을 수정하지 않았고, 제품 동작을 실행해 검증한 결과로 해석해서는 안 된다.
