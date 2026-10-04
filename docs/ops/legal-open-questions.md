# 법률 자문 질의서 및 확인 필요 사항 (한·미 변호사 발송용)

- **출처:** `docs/research/legal-opinion-memo-2026-10-04.md` 제5장의 질의서를 그대로 옮겼다(원문 유지 — 다듬지 않는다). 아래 §C·§D가 이 저장소에서 추가한 것이다.
- **사용법:** §A는 한국 가상자산 전문 변호사에게, §B는 미국 변호사(US counsel)에게 각각 그대로 복사해 발송한다. §C 추가 질문은 양쪽 모두에, §D 가정은 각 질의서 말미에 함께 붙여 확인을 받는다.
- **배경:** 초기 자금이 부족한 단계에서 1~2시간의 최소 유료 자문으로 핵심 리스크를 해소하고 결정을 잠금 해제하는 것이 목적이다(메모 제5장 취지).

---

## A. 한국 변호사 발송용 질의서 (메모 §5.1 원문)

```markdown
[질의서: EastSea 체인 및 Doubloon 코인 메인넷 출범 관련 특금법 및 자본시장법 검토 요청]

수신: 변호사님
발신: 주식회사 핀(Pipln) 대표이사 이현종

당사는 Apple Silicon Mac 전용 레이어-1 블록체인 'EastSea(동해)' 및 네이티브 코인 'Doubloon(DBLN)'의 출시를 앞두고 있습니다. 효율적인 상담을 위해 당사의 핵심 사실관계를 정리하여 아래 3가지 질문을 드립니다.

[당사 핵심 사실관계]
1. 사전 발행(Premine), 사전 판매(ICO), 파우셋, 창업자 사전 할당이 일절 없음 (0개).
2. 코인은 오직 블록 생성 시점에 켜져 있는 Mac 노드(50%)와 zkVM 연산 증명자(50%)에게 프로토콜 규칙에 따라 자동 분배됨.
3. 1인당 시간당 보상 상한은 1/16로 고정되며, 미달 시 미발행 소각됨. 창업자도 동일한 1/16 규칙으로 참여함.
4. 사용자 지갑 키는 기기 Secure Enclave에만 존재하며 당사는 개인키를 보관·복구할 수 없음.
5. Mac 등록 시 당사가 Apple DeviceCheck 토큰을 검증하는 단일 P-256 서명을 제공하나, 메인넷 전 2-of-3 다중서명으로 전환 예정임.

[변호사 질의 사항]
질문 1 (특금법 VASP 해당 여부):
FIU의 2026.8.20 개정 매뉴얼상 비수탁 지갑 4대 기준을 충족하는 당사의 비수탁 지갑 및 P2P 노드 클라이언트 배포 행위, 그리고 수수료 0원의 불변 DEX 스마트 컨트랙트 배포 행위가 특정금융정보법 제2조의 '가상자산사업자(VASP)' 신고 대상에서 명백히 제외되는지 확인 부탁드립니다.
▶ [잠금 해제되는 결정]: 한국 가상자산사업자 미신고 형사 리스크 없이 예정대로 클라이언트 소프트웨어 공개 DMG 배포 확정.

질문 2 (자본시장법 증권성 및 이용자보호법):
사전 판매나 투자금 유치 없이 순수 연산·가동 대가로만 분배되는 DBLN 코인이 자본시장법 제4조 제6항의 '투자계약증권'에 해당하지 않는다는 의견이 타당한지, 그리고 창업자가 채굴한 DBLN을 장래 매도할 때 가상자산이용자보호법 제10조 제5항(자기발행 가상자산 거래제한)의 수범자에 해당하는지 여부를 검토해 주십시오.
▶ [잠금 해제되는 결정]: DBLN의 비증권성 법적 확인 및 창업자 개인 보유 코인의 적법한 장기 매도 가이드라인 확립.

질문 3 (본딩커브 런치패드 무수수료 제공 시 VASP 의율 리스크):
당사가 0% 수수료, 랭킹/추천 기능 배제, 불변 컨트랙트 조건으로 밈코인 본딩커브 런치패드를 웹 프론트엔드로 제공할 경우, 대법원 2024도10710 판결(영업으로 거래 대행)에 비추어 미신고 VASP 중개·알선으로 포섭될 위험이 여전히 존재하는지, 아니면 오픈소스 정적 페이지만 공개하고 호스팅을 하지 않는 것이 필수적인지 고견을 구합니다.
▶ [잠금 해제되는 결정]: 본딩커브 런치패드 공식 웹 UI 호스팅 강행 여부 또는 순수 오픈소스 코드 릴리즈로의 격리 확정.
```

---

## B. 미국 변호사(US Attorney / Crypto Counsel) 발송용 질의서 (메모 §5.2 원문)

```markdown
[Legal Inquiry: Regulatory Status of EastSea Network & Doubloon (DBLN) under U.S. Federal Securities & Commodities Laws]

To: Counsel
From: Hyun Jong Lee, Founder & CEO, Pipln (Republic of Korea)

Pipln is launching "EastSea," a Mac-first Layer-1 blockchain with its native coin "Doubloon (DBLN)." We seek a focused consultation on U.S. regulatory characterization based on the following verified facts:

[Key Facts]
1. Zero pre-mine, zero token sales (no ICO/SAFT/airdrops), zero founder reserve allocations.
2. 100% of DBLN is minted via block rewards: 50% to registered online Mac nodes, 50% to on-device zkVM block provers.
3. Strict hard cap: No single operator can earn more than 1/16 of an hour's issuance; excess supply is never minted. The founder participates under the exact same 1/16 cap.
4. Non-custodial client architecture: Private keys generated inside Apple Secure Enclave / WebAuthn, never transmitted to Pipln.
5. Mac registration utilizes Apple DeviceCheck via Pipln's registrar, which is transitioning from a single P-256 signer to a 2-of-3 multi-signature scheme before mainnet genesis.
6. The protocol maintains a fee-free, immutable constant-product DEX (x*y=k) where 100% of 0.3% pool fees go to LPs.

[Specific Legal Questions]
Question 1 (Securities Characterization under Howey & Rel. 33-11412):
Under the SEC’s March 17, 2026 Interpretive Release (Release Nos. 33-11412 / 34-105020) and the Staff Statement on Proof-of-Work Mining Activities, does DBLN qualify as a non-security digital commodity given the absence of an investment of money and the reliance on bona fide hardware computation? Specifically, does transitioning the DeviceCheck registrar to a 2-of-3 independent multi-sig sufficiently defeat the argument of "managerial efforts" under Howey prong 4?
▶ [Unlocks Decision]: Confirmation of non-security stance in U.S. markets; unlocks marketing and distribution to U.S. developers.

Question 2 (FinCEN MSB & Exchange Act Broker-Dealer Status):
Does distributing an open-source, non-custodial wallet application (integrated with macOS Secure Enclave) and deploying immutable, fee-free smart contracts (TokenFactory, DEX, and multisig Vault) subject Pipln to registration as a Money Services Business (MSB) under FinCEN FIN-2019-G001 or as an unregistered broker under Section 15(a) of the Exchange Act, following the dismissal of SEC v. Consensys?
▶ [Unlocks Decision]: Authorization to ship macOS DMG and Chrome Web Store extension without U.S. money transmitter licenses.

Question 3 (Civil RICO Exposure from Bonding-Curve Launchpad):
In light of the S.D.N.Y. ruling in Aguilar v. Baton Corporation (Pump.fun, Aug 31, 2026, Doc. 184), where RICO claims survived a motion to dismiss based on fee extraction and market manipulation, does operating a zero-fee, immutable, uncurated bonding-curve launchpad with geoblocking of U.S. persons protect Pipln from civil RICO and unlicensed money transmission liability under 18 U.S.C. § 1960? Or is publishing only the static client code to GitHub the required legal posture?
▶ [Unlocks Decision]: Final determination on whether to host the launchpad web UI or restrict it to an open-source repository release.
```

---

## C. 추가 질문 — 등록기 단독 서명자의 출시 시점 적격성 (양쪽 변호사에게)

**한국 변호사용:**

> 질문 4 (등록기 단독 서명자의 출시 시점 적격성):
> 현재 Mac 등록 검증은 Pipln이 단독 운영하는 P-256 서명자(등록기)로 이루어집니다. 이 서명자는 (a) 공개된 온체인 규칙에 따라 최소 7일 예고 후 위원회 서명 업그레이드로 교체·폐지할 수 있고 (b) 그 규칙 자체는 공개 문서로 게시되어 있습니다. 메인넷 출시 시점에 이 단독 서명 구조를 유지하는 것이 허용 가능한지, 아니면 제네시스 전에 독립 당사자를 포함한 다중 서명(예: 2-of-3)으로 전환해야 하는지 검토를 요청합니다. 특히, 단독 서명자가 검증자 진입을 선별·차단할 수 있다는 사실이 자본시장법상 '공동사업 관여' 또는 특금법상 문제로 이어질 수 있는지가 궁금합니다.
> ▶ [잠금 해제되는 결정]: 제네시스 시점 등록기 서명 구조 확정(단독 유지 vs 다중서명 전환 완료 후 출시).

**미국 변호사용:**

> Question 4 (Registrar single-signer acceptability at launch):
> Mac registration is currently validated by a single P-256 signer controlled by Pipln. That signer can be replaced or revoked through an on-chain committee-signed upgrade with at least 7 days' public notice, and those rules are published. Is retaining this single-signer structure at mainnet genesis acceptable, or must it move to a multi-party scheme (e.g., 2-of-3 with independent parties) before launch? In particular, does the operator's technical ability to select or exclude validators undermine the "no managerial efforts" position under Howey prong 4 even with the published 7-day revocation rule in place?
> ▶ [Unlocks Decision]: Final registrar signing structure at genesis.

---

## D. 우리가 세운 가정 — 확인 요망 ("assumptions we made, please confirm")

리드가 이미 결정하여 반영한 사항이다. 각 질의서에 함께 붙여 "이대로 충분한지" 확인을 받는다.

1. **DISCLAIMER.md 개정 방향(2026-10-04 반영).** 전면 면책 대신 고의·중대한 과실 및 법령상 배제 불가능한 책임(약관규제법, 소비자 강행 권리)에 대한 단서를 두고, 국문·영문 동일 내용에서 한국 관할 내 국문 우선을 선언하며, 준거법을 대한민국으로 하고 제1심 관할을 서울중앙지방법원으로 명시했다. 관할 조항은 약관규제법 제14조(소비자 부담 관할 합의 제한)와의 충돌 가능성 때문에 문서 안에 TODO-for-lawyer로 표시해 두었다. → 전속관할 문구를 유지할 수 있는지, 아니면 소비자 주소지 관할 병기 등으로 교정해야 하는지 확인 필요.
2. **웹사이트 카피 원칙.** 어떤 문구도 수익·이익 프레임("잠자는 동안에도 일한다", "먼저 온 사람이 더 큰 몫"), 'AI 모델 감사 = 안전' 암시, 가격·상장 언급을 담지 않게 했고, 1/16 분배 규칙은 약속 없는 규칙 설명으로만 남겼으며, 페이지 상단에 "테스트넷 전용·코인 무가치·토큰 세일 없음·투자 조언 아님"을 노출했다. → 표시광고법 제3조·가상자산이용자보호법 제10조 제3항·FTC Act §5 관점에서 이 수준이 충분한지 확인 필요.
3. **런치패드 비호스팅 결정.** Pipln은 메인넷 출시 시점에 공개 런치패드 UI를 자사 도메인에서 호스팅하거나 홍보하지 않는다. 컨트랙트와 오픈소스 정적 페이지만 공개하고 호스팅은 커뮤니티에 맡긴다(메모 3.8의 '최소 대안'). 사이트·README 어디에도 Pipln 운영 런치패드를 소개하는 문구가 없도록 점검했다. → 이 격리 구조가 질문 3의 답변과 일치하는지, 추가 표기(예: "커뮤니티 자율 호스팅")가 필요한지 확인 필요.

---

## 답변 후 처리

- 답변이 오면 이 문서에 결과를 요약해 기록하고, `DISCLAIMER.md`(TODO-for-lawyer 주석)와 `site/` 카피, `docs/ops/privacy-policy.md`에 반영한다.
- 등록기 서명 구조 결정은 `docs/design/14-registration.md`·`docs/ops/registrar.md`와 메인넷 체크리스트(`docs/ops/mainnet-launch.md`)로 넘긴다.
