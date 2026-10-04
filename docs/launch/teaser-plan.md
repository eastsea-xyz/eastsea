# 홍보 밑밥 계획: 공개 전 14일 (2026-10-04 초안)

결정 기록: 이전 결정은 "홍보는 소스 공개와 함께"([demo-and-show-hn.md](demo-and-show-hn.md))였고, 2026-10-04 사용자 지시로 **메인넷 전에 밑밥을 깐다**로 넓혔습니다. 법적 선은 그대로입니다: [법률 메모](../research/legal-opinion-memo-2026-10-04.md) 3.6절. 이 문서는 초안이며, **아무것도 게시하지 않았습니다.**

## 먼저 정할 것 (게시 전 조건)
1. **상표 출원이 먼저**([trademark-filing.md](../ops/trademark-filing.md)): 이름을 공개하면 선점자가 먼저 출원할 수 있습니다. 출원 접수증을 받은 뒤에 첫 게시를 합니다.
2. **계정 이름 선점은 지금 해도 됩니다**(게시 없이). 아래 표.
3. **창업자 공시**: 창업자·기여자 계정에서 올리는 글은 프로필 소개와 글에 "Founder"를 밝힙니다(FTC 2023 가이드라인, 메모 3.6).
4. 사이트(`eastsea.xyz`)가 열려 있어야 링크를 걸 수 있습니다. 호스팅은 상표 출원 뒤.
5. 이메일 대기자 목록은 만들지 않습니다(개인정보 처리 의무가 생기고, 처리방침과 동의 화면이 먼저 필요).

## 계정 이름 (2026-10-04 브라우저로 확인한 결과)
| 플랫폼 | 후보 | 상태 | 비고 |
|---|---|---|---|
| X | `@eastsea` | **사용 중**(개인 계정) | 쓸 수 없음 |
| X | `@eastseaxyz`, `@eastsea_xyz` | 없음(사용 가능) | `@eastseaxyz`를 권장 |
| GitHub | `eastsea` | **사용 중**(EastSea라는 조직·사용자) | 쓸 수 없음. 소스 공개 저장소 이름 재검토 필요 |
| GitHub | `eastsea-xyz` | 없음(사용 가능) | 조직으로 선점 권장 |
| YouTube | `@eastsea`, `@eastseaxyz` | 페이지 없음(사용 가능으로 보임) | 채널 생성 후 핸들 변경 가능 여부 확인 |
| Instagram | `eastseaxyz` | 없음으로 보임 | 코인 이미지용(선택) |
| Telegram | `@eastsea` | 사용 중 | `@eastseaxyz`는 판별 불가, 생성 시 확인 |
| Bluesky | `eastsea.xyz` | 도메인 인증 핸들, 사이트가 열린 뒤 가능 | |
| Reddit | `r/eastsea` | 확인 못 함 | |
**중요한 발견**: GitHub `eastsea`와 X `@eastsea`가 이미 쓰이고 있습니다. 상표 출원 전 KIPRIS 검색(`docs/ops/trademark-search-2026-10-04.md`)과 별개로, 소스 공개 주소와 SNS 정체성이 다르게 갈 수 있다는 점을 짚어 둡니다.

## 훅(첫 문장) 원칙
- 한 문장으로 **사람이 얻는 것**을 말한다. 기술 용어(합의, DKG, zk)는 뒤로.
- **없는 것**으로 시작하는 훅이 강하다: 시드 문구 없음, 서버에 묻지 않음, 토큰 세일 없음. 모두 코드와 문서로 확인되는 사실이라 그대로 쓸 수 있다.
- 숫자는 **실측한 것만**: 확정 약 1초(영상으로 보여 준 것), 검증 시간 등은 게시 당일 다시 잰다.
- 금지(법률 메모 3.6): 수익·패시브 인컴·"밤새 번다"·초기 선점 혜택·가격·상장·"감사 통과/안전 보장"·Apple 보증·"SEC 승인". 보상은 규칙 설명으로만, "테스트넷 코인은 가치 없음"을 같은 화면에 둔다.

## 훅 후보 (한국어 / 영어)
| # | 한국어 | English | 쓰는 곳 |
|---|---|---|---|
| H1 | 12단어를 외울 필요 없는 지갑. 키는 맥 안에서 태어납니다. | A wallet with no seed phrase. Your key is born inside your Mac. | 첫 글, 소개 페이지 상단 |
| H2 | 내 잔액, 서버한테 안 물어봅니다. 내 맥이 직접 확인해요. | We don't ask a server for your balance. Your Mac checks it itself. | 기술 스레드 첫 글 |
| H3 | 결제는 엄지 하나(Touch ID). 확정은 1초. | Pay with your thumb. Final in about a second. | 영상 1의 첫 장면 자막 |
| H4 | 토큰 세일도, 프리마인도 없습니다. 이게 왜 중요한지 3줄로. | No token sale. No premine. Why that matters, in three lines. | 신뢰 스레드 |
| H5 | AI 비서에게 용돈 지갑을 줬습니다. 한도는 체인이 지킵니다. | We gave an AI agent an allowance wallet. The chain enforces the limit. | 영상 2, 개발자 대상 |
| H6 | 해적단이 만들고 있습니다: 맥 한 대로 시작하는 블록체인. | A small pirate crew building a blockchain that starts with one Mac. | 팀 소개, 브랜드 톤 |
| H7 | 도블룬(Doubloon)이라는 금화를 새겼습니다. (코인 렌더 이미지) | We struck a doubloon. | 이미지 글. 가격·상장 언급 없이 이미지와 한 줄만 |

## 14일 캘린더 (게시는 상표 출원 접수증을 받은 날을 D0로)
| 날 | 채널 | 내용 | 훅 |
|---|---|---|---|
| D0 | X, 사이트 | 첫 글: 이름과 한 문장 소개, 코인 이미지, 사이트 링크. 상태 표시: 공개 테스트넷, 메인넷 출시 전, 토큰 세일 없음 | H1 + H7 |
| D2 | X | 스레드 "내 맥이 직접 확인한다" 5개 글 | H2 |
| D4 | X, YouTube | 영상 1(60초) | H3 |
| D6 | X | 스레드 "세일도 프리마인도 없는 이유" (규칙 설명, 가격 이야기 없음) | H4 |
| D8 | X, YouTube | 영상 2(에이전트 지갑) | H5 |
| D10 | X | 팀·브랜드 글, 해적단 톤, 코인 아트 제작기 | H6 |
| D12 | X | 자주 묻는 질문 요약(사이트 FAQ 8개를 글 3개로) | — |
| D14 | X | 소스 공개 일정 예고 또는 소스 공개(상표·개명·감사 상황에 따라) | — |
각 글에는 사이트 링크, 필요하면 "테스트넷 코인은 가치가 없습니다"를 넣고, 창업자 계정이면 공시를 넣는다.

## 하지 않는 것
- 가격, 상장, 수익 예측, 초기 참여 이익 언급.
- 다른 프로젝트나 인물 비교 비방.
- 인플루언서에게 대가를 주고 글 요청(규제와 공시 부담이 크다).
- 메인넷 날짜 약속(감사와 소크가 끝나기 전).
- 이메일·텔레그램 대기자 모집(개인정보 의무).

## 측정(지표)
팔로워·조회는 참고만 합니다. 판단 기준은 **다운로드 문의 수**나 **설치 후 첫 송금 성공률**처럼 제품 신뢰와 연결되는 것입니다(메인넷 전에는 테스트넷 이용자 수).

## 다음 단계 (결정이 필요)
1. 계정 이름 선점을 어느 플랫폼에서 할지(X `@eastseaxyz`, GitHub `eastsea-xyz`, YouTube, Instagram). 새 계정 생성은 본인 인증(이메일·전화)이 필요할 수 있어 사용자가 확인해야 하는 단계가 나옵니다.
2. 상표 출원 일정(출원 접수증이 D0 조건).
3. 첫 글 문구와 이미지 최종 승인(아래 초안을 고쳐 주세요).

## 첫 글 초안 (D0, 승인 전)
한국어: "동해(EastSea)를 만들고 있습니다. 시드 문구 없는 지갑, 내 잔액을 서버에 묻지 않고 내 맥이 직접 확인하는 체인. 지금은 공개 테스트넷이고, 메인넷은 아직입니다. 토큰 세일은 없습니다. eastsea.xyz (창업자 계정)"
English: "We're building EastSea: a wallet with no seed phrase, on a chain your Mac verifies itself instead of asking a server. Public testnet today, mainnet not launched, no token sale. eastsea.xyz (founder account)"
