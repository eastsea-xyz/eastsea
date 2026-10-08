# EastSea: Legal Notice, Terms of Use, and Limitation of Liability

**Last Updated:** October 8, 2026 (privacy sections)
**Language & Governing Law:** 본 약관은 대한민국 법률을 준거법으로 하며, 국문과 영문 내용이 상충할 경우 대한민국 관할 내에서는 국문 약관이 우선합니다. *(These terms are governed by the laws of the Republic of Korea; where the Korean and English texts conflict, the Korean text prevails within the jurisdiction of the Republic of Korea.)*

---

## 1. What EastSea Is

EastSea is open-source software built for production use: a Mac app that runs a node and a wallet, an iPhone wallet, a browser extension, and the node software itself (the "Software"), for the EastSea network, a blockchain whose validators are Macs.

- **Mainnet has not launched.** The network running today is the public testnet (chain 7780). Mainnet will be a new network with a new genesis.
- **Not yet independently audited.** The Software has not had an independent security audit. It may contain bugs, including bugs that affect funds.
- The Software is licensed under the MIT and Apache 2.0 licenses. By installing, running, or using it, you agree to these terms. If you do not agree, do not use the Software.

---

## 2. DBLN, Value, and No Financial Advice

1. **How DBLN comes into existence.** DBLN is created only by block rewards under the network's public rules. There is no token sale, no premine, and no founder allocation.
2. **Value is set by the market.** Pipln and the authors do not sell DBLN and make no promise or representation about its price, value, liquidity, exchange listing, return, or any way to exchange it for money. Any value DBLN may have is determined by the market, not by us.
3. **Testnet DBLN does not carry over.** Balances and rewards on the testnet do not carry over to mainnet.
4. **Not an offering, not advice.** Nothing in the Software or its documentation is an offer of securities or investment, a financial product, or financial, investment, legal, tax, or accounting advice. Running a node, proving blocks, or holding DBLN is your own decision.
5. **Costs and taxes.** Electricity, hardware wear, network costs, and any taxes on rewards are your responsibility. The app's reward export is a record, not tax advice.

---

## 3. Keys and Funds

1. Wallet keys are created in your device's Secure Enclave (Mac and iPhone) or encrypted in your browser (extension). They are never sent to us. **Pipln does not hold users' keys or funds.**
2. **Nobody can recover your funds for you.** If you lose every device and have not set up a recovery method, the funds cannot be restored by anyone, including Pipln.
3. Transactions on the network are final once confirmed. Check addresses and amounts before you approve.

---

## 4. Networking and Privacy

The Software connects to other nodes over encrypted QUIC connections (iroh). Nodes publish and look up their network addresses on the public BitTorrent Mainline DHT (BEP 44); the Software does not download, share or store files through BitTorrent. When nodes cannot connect directly, public relays carry the traffic while transport encryption is maintained.

**Local keys and the public network**

Private keys stay on your device. Addresses, balances, transactions, rewards, voting public keys, node identifiers and registration events are public on chain and may remain public indefinitely. Leaving the network or deleting the app does not erase earlier blocks, archives or copies held by others.

Peers, relays and the public address-discovery network can see connection IP addresses. RPC nodes can see the addresses you query. The Mac wallet normally asks its own node first; a remote fallback or the public gateway receives the query when used.

**Registration and repeated Apple checks**

Apple does not sponsor or endorse EastSea.

Registering a voting node encrypts the Apple DeviceCheck token to the authenticated registrar key. Relaying validators receive ciphertext and cannot read the token. Pipln's registrar decrypts it and sends it to Apple Inc. (USA) over HTTPS to check genuine hardware and prior registration. This happens at initial registration and during daily re-attestation while participating. Apple also receives a request identifier and time, and the registrar's connection IP address.

Apple keeps a per-device registration bit associated with the developer's apps; it can survive reinstalling the app. We do not control Apple's retention or promise that deleting our records clears this bit. Refusing DeviceCheck prevents voting-node registration and daily eligibility checks; the wallet remains available. Touch ID authorizes a signature and is not itself privacy consent.

**What the registrar keeps, and for how long**

The registrar uses the raw token in request memory and does not write it to its registration store. On your Mac, a protected local token file is refreshed about hourly while the app runs. The registrar persists the voting public key, operator address, node identifier, beacon address and original registration time to prevent duplicate registration and permit re-attestation. This store has no automatic expiry or deletion timer; records remain until an operator removes them or retires the service.

Registration token hashes used to prevent simultaneous requests are removed when the request finishes. Successful daily checks keep voting-key and token-hash rate limits in memory for the current and two preceding periods; older entries are pruned on a later successful check, and all are lost when the process exits. The registration store does not retain client IP addresses. We have not verified a fixed retention period for operational logs, backups or providers' logs, and do not promise deletion after 30 days.

**Optional country and aggregate presence**

The first-launch country screen must be answered before any country preference is sent to the local node. A distribution can preselect sharing with a notice or ask for a choice first; either mode lets you decline without losing wallet or node features. The country chosen from your Mac's Region setting or by you stays on this Mac and only selects a broad region bucket. It is not GPS, IP geolocation or an on-chain record.

Public presence RPC and gossip carry only unverified cohort counts by role, broad region and coarse version. They expose no per-node list, observer identity, exact country or exact observation time. Times use 10-minute buckets; groups smaller than three are folded into broader groups or withheld, including rare quality data. These are observations, not a census of distinct Macs, and aggregation does not guarantee anonymity. Turning off sharing stops future use of the country preference; already received aggregate copies cannot be recalled.

**Website, gateway and updates**

Cloudflare hosts the website and carries public-gateway requests. It can receive your IP address, request time, URL and browser information; gateway requests can include the queried public address. The page adds no analytics or tracking cookies. Your language choice is stored in localStorage on your device.

On macOS, Sparkle checks for updates about hourly and downloads releases from GitHub and its download services. Those services can receive your IP address and app/version information from requests. Sparkle is the updater in the app, not a separate EastSea analytics service; its optional system profiling is disabled, including a previously saved opt-in. Apple, Cloudflare, GitHub and Google are US-based providers and may process data in other countries. Their processing locations and retention depend on their services; EastSea cannot promise to erase provider-controlled logs.

**Diagnostics, email and deletion requests**

The diagnostic report is copied to your clipboard only; the app does not upload it automatically. There is no separate remote usage-analytics upload. If you send a report or privacy request, we receive your email address, message and any attachments. privacy@eastsea.xyz uses Cloudflare Email Routing to forward mail to the operator's Gmail mailbox, so Cloudflare and Google handle those messages.

Email privacy@eastsea.xyz to request access, correction, deletion or a stop to processing of data we control. Identify the relevant public address or node key and the records you mean; never send a private key or DeviceCheck token. We will explain what can be deleted and any reason for retaining it. Removing a registrar binding can prevent future daily checks; it does not reset Apple's bit. We cannot erase public chain history or recall other participants' copies, and cannot promise deletion of provider-controlled records. Local files and preferences can be removed on your device.

**Contact and changes**

Privacy contact: Pipln privacy officer, privacy@eastsea.xyz. Updated policies are published here with an effective date; material changes are also explained in the app.

[Privacy policy / 개인정보 처리방침](https://eastsea.xyz/privacy) · English, 한국어, 日本語, 简体中文, Español.

---

## 5. Disclaimer of Warranties ("AS IS")

THE SOFTWARE IS PROVIDED "AS IS" AND "AS AVAILABLE", WITHOUT WARRANTY OF ANY KIND, EITHER EXPRESS OR IMPLIED, INCLUDING, BUT NOT LIMITED TO:
- THE IMPLIED WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE, AND NON-INFRINGEMENT;
- ANY WARRANTY THAT THE SOFTWARE OR THE NETWORK WILL BE SECURE, UNINTERRUPTED, ERROR-FREE, OR FREE FROM HARMFUL COMPONENTS;
- ANY WARRANTY REGARDING THE ACCURACY, RELIABILITY, OR INTEGRITY OF ANY DATA, BLOCKS, TRANSACTIONS, BALANCES, OR CONSENSUS STATES.

THE ENTIRE RISK ARISING OUT OF THE USE, PERFORMANCE, OR INABILITY TO USE THE SOFTWARE REMAINS WITH YOU. Nothing in these terms excludes any warranty or condition that cannot lawfully be excluded.

---

## 6. Limitation of Liability

These limitations apply only to the extent permitted by applicable law. **They do not apply to liability for intentional misconduct or gross negligence (고의 또는 중대한 과실), and they do not exclude or limit any liability — or any of your rights — that cannot lawfully be excluded or limited, including under the Korean Act on the Regulation of Terms and Conditions and your mandatory rights as a consumer.**

TO THE MAXIMUM EXTENT PERMITTED BY APPLICABLE LAW, IN NO EVENT SHALL THE AUTHORS, MAINTAINERS, CONTRIBUTORS, OR SIGNING ENTITIES (INCLUDING **PIPLN**, HYUN JONG LEE, AND PROJECT CONTRIBUTORS) BE LIABLE FOR ANY INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, PUNITIVE, OR CONSEQUENTIAL CLAIM, DAMAGES, LOSSES, COSTS, OR EXPENSES (INCLUDING LOSS OF FUNDS, LOSS OF PROFITS, LOSS OF DATA, SYSTEM OUTAGES, HARDWARE DAMAGE, OR REGULATORY FINES), ARISING OUT OF OR IN CONNECTION WITH:
1. THE USE OF OR INABILITY TO USE THE SOFTWARE OR THE NETWORK;
2. ANY LOSS OF KEYS, DEVICES, OR FUNDS;
3. ANY NETWORK INTERACTIONS INITIATED BY THE SOFTWARE, OR ATTACKS TARGETING YOUR DEVICE, IP ADDRESS, OR NODE;
4. ANY ACTIONS OF REGULATORY AUTHORITIES, SERVICE PROVIDERS, OR LAW ENFORCEMENT REGARDING YOUR USE OF THE SOFTWARE.

THIS LIMITATION APPLIES REGARDLESS OF THE LEGAL THEORY ASSERTED, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGES. IF ANY PART OF THIS LIMITATION IS HELD UNENFORCEABLE, IT APPLIES ONLY TO THE MAXIMUM EXTENT PERMITTED BY LAW.

---

## 7. Compliance with Local Laws

The legal status of peer-to-peer software, blockchain nodes, and digital assets differs between jurisdictions. You are solely responsible for making sure your use of the Software complies with all laws, regulations, sanctions, and export rules that apply to you.

---

## 8. Governing Law and Dispute Resolution

Any dispute arising out of or in connection with the Software shall be governed by the laws of the Republic of Korea. The parties agree to submit to the exclusive jurisdiction of the Seoul Central District Court for the first instance.

> **TODO-for-lawyer (관할 조항 확인 요망):** 위 전속관할 문구는 리서치 메모(제3.5절 Redline)가 제안한 원문 그대로이다. 다만 약관규제법 제14조(소제기의 금지 등)는 소비자에게 부담이 되는 관할 합의를 제한하므로, 소비자를 상대로 하는 전속관할 조항은 그 범위에서 무효가 될 수 있다. 변호사 확인 전까지 원문을 유지하며, 확인 후 "소비자의 주소지 관할 법원 병기" 또는 "준거법만 명시" 등으로 교정한다.

---
---

# [국문 약관] 법적 고지, 이용 약관 및 책임의 한계

**시행일:** 2026년 10월 4일
*(국문과 영문은 같은 내용입니다. 국문과 영문이 상충하는 경우 대한민국 관할 법원에서는 국문 약관이 우선합니다.)*

### 1. 동해는
동해는 **실제 운영(production)을 목표로 만든 오픈소스 소프트웨어**입니다 — 노드와 지갑을 함께 돌리는 Mac 앱, iPhone 지갑, 브라우저 확장, 노드 소프트웨어(통칭 "소프트웨어"). 이 소프트웨어는 검증자가 Mac인 블록체인, 동해 네트워크를 위한 것입니다.

- **메인넷은 아직 출시되지 않았습니다.** 지금 운영되는 네트워크는 공개 테스트넷(체인 7780)이며, 메인넷은 새 제네시스로 시작하는 새 네트워크입니다.
- **아직 독립적인 보안 감사를 받지 않았습니다.** 자금에 영향을 주는 버그를 포함하여 결함이 있을 수 있습니다.
- 소프트웨어는 MIT와 Apache-2.0 라이선스로 배포됩니다. 설치·실행·이용함으로써 본 약관에 동의하는 것으로 봅니다. 동의하지 않는다면 사용하지 마십시오.

### 2. DBLN과 가치
1. **DBLN의 발행.** DBLN은 네트워크의 공개된 규칙에 따른 블록 보상으로만 생성됩니다. 토큰 세일, 사전 발행(프리마인), 창업자 몫이 없습니다.
2. **가치는 시장이 정합니다.** Pipln과 개발자는 DBLN을 판매하지 않으며, 가격·가치·유동성·거래소 상장·수익·현금화 방법 그 어떤 것도 약속하거나 보증하지 않습니다. DBLN에 가치가 있다면 그것은 시장이 정하는 것입니다.
3. **테스트넷 DBLN은 이관되지 않습니다.** 테스트넷의 잔액과 보상은 메인넷으로 넘어가지 않습니다.
4. **제안도 조언도 아닙니다.** 소프트웨어와 문서의 어떤 내용도 증권·투자의 제안, 금융 상품, 금융·투자·법률·세무·회계 조언이 아닙니다. 노드 실행, 블록 증명, DBLN 보유는 이용자 본인의 결정입니다.
5. **비용과 세금.** 전기요금, 하드웨어 마모, 통신 비용, 보상에 대한 세금은 이용자의 부담입니다. 앱의 보상 내보내기는 기록일 뿐 세무 조언이 아닙니다.

### 3. 키와 자금
1. 지갑 키는 기기의 Secure Enclave(Mac·iPhone)에서 만들어지거나, 브라우저 확장의 경우 브라우저 안에서 암호화되어 보관되며, 우리에게 전송되지 않습니다. **Pipln은 이용자의 키나 자금을 보관하지 않습니다.**
2. **누구도 자금을 되찾아 줄 수 없습니다.** 모든 기기를 잃고 복구 방법을 설정해 두지 않았다면 Pipln을 포함한 그 누구도 자금을 복구할 수 없습니다.
3. 네트워크 거래는 한 번 확정되면 되돌릴 수 없습니다. 승인하기 전에 주소와 금액을 확인하십시오.

### 4. 네트워크와 개인정보

소프트웨어는 다른 노드와 암호화된 QUIC 연결(iroh)로 통신합니다. 노드는 네트워크 주소를 공개 BitTorrent Mainline DHT(BEP 44)에 등록·조회하며, BitTorrent로 파일을 내려받거나 공유·저장하지 않습니다. 직접 연결할 수 없으면 공개 릴레이를 거치며 전송 암호화는 유지됩니다.

**기기의 키와 공개 네트워크**

개인키는 이용자의 기기에 남습니다. 주소, 잔액, 거래, 보상, 투표 공개키, 노드 식별자와 등록 이벤트는 체인에 공개되며 무기한 공개될 수 있습니다. 네트워크를 떠나거나 앱을 삭제해도 과거 블록, 아카이브와 다른 사람이 보관한 사본은 지워지지 않습니다.

피어, 릴레이와 공개 주소 탐색 네트워크는 연결 IP 주소를 볼 수 있습니다. RPC 응답 노드는 조회한 주소를 볼 수 있습니다. Mac 지갑은 보통 자기 노드에 먼저 묻지만, 원격 대체 노드나 공개 게이트웨이를 이용하면 그곳이 조회를 받습니다.

**등록과 반복되는 Apple 확인**

Apple은 동해를 후원하거나 보증하지 않습니다.

투표 노드 등록 시 Apple DeviceCheck 토큰을 인증된 등록기의 키로 암호화합니다. 요청을 중계하는 검증자는 암호문만 받아 토큰을 읽을 수 없습니다. Pipln 등록기가 복호화한 토큰을 HTTPS로 Apple Inc.(미국)에 보내 정품 기기와 기존 등록 여부를 확인합니다. 최초 등록 때와 참여 중 매일 재인증 때 전송합니다. Apple은 요청 식별자, 시각과 등록기의 연결 IP 주소도 받습니다.

Apple은 개발자의 앱에 연결된 기기별 등록 비트를 보관하며, 이 비트는 앱 재설치 후에도 남을 수 있습니다. Apple의 보유기간을 우리가 통제하지 않으며, 우리 기록을 삭제하면 비트도 지워진다고 약속하지 않습니다. DeviceCheck를 거절하면 투표 노드 등록과 일일 자격 확인은 불가능하지만 지갑은 이용할 수 있습니다. Touch ID는 서명을 승인하며 그 자체가 개인정보 동의는 아닙니다.

**등록기가 보관하는 항목과 기간**

등록기는 원토큰을 요청 처리 메모리에서 사용하고 등록 저장소에 쓰지 않습니다. Mac에서는 앱이 실행되는 동안 보호된 로컬 토큰 파일을 약 한 시간마다 갱신합니다. 등록기는 중복 등록 방지와 재인증을 위해 투표 공개키, 운영자 주소, 노드 식별자, 비콘 주소와 최초 등록 시각을 지속 저장합니다. 이 저장소에는 자동 만료나 삭제 타이머가 없으며, 운영자가 삭제하거나 서비스를 종료할 때까지 기록이 남습니다.

동시 요청을 막기 위한 등록 토큰 해시는 요청이 끝나면 제거합니다. 성공한 일일 확인의 투표키·토큰 해시 제한 기록은 현재 기간과 앞선 두 기간에 대해 메모리에 보관합니다. 더 오래된 항목은 이후 확인 성공 때 정리되고 프로세스 종료 시 모두 사라집니다. 등록 저장소는 이용자 IP 주소를 보관하지 않습니다. 운영 로그, 백업과 제공업체 로그의 고정 보유기간은 검증되지 않았으며 30일 후 삭제를 약속하지 않습니다.

**선택적인 국가 설정과 접속 집계**

첫 실행 국가 화면에 응답하기 전에는 어떤 국가 설정도 로컬 노드로 보내지 않습니다. 배포 설정에 따라 안내와 함께 공유가 미리 선택되어 있거나 먼저 선택을 요청할 수 있습니다. 두 방식 모두 거절해도 지갑·노드 기능을 그대로 이용합니다. Mac의 지역 설정 또는 직접 선택한 국가는 이 Mac에만 남고 넓은 지역 버킷만 정합니다. GPS나 IP 기반 위치 추적이 아니며 체인에도 기록하지 않습니다.

공개 presence RPC와 가십에는 검증되지 않은 관측 그룹의 역할별·넓은 지역별·대략적인 버전별 수만 담습니다. 개별 노드 목록, 관찰자 신원, 정확한 국가나 정확한 관측 시각은 공개하지 않습니다. 시각은 10분 단위이며, 3개 미만 그룹과 희소 품질 정보는 더 넓은 그룹으로 합치거나 숨깁니다. 서로 다른 Mac의 총조사가 아닌 관측값이며 집계만으로 익명성을 보장하지 않습니다. 공유를 끄면 앞으로 국가 설정을 쓰지 않지만, 이미 받은 집계 사본을 회수할 수는 없습니다.

**웹사이트, 게이트웨이와 업데이트**

Cloudflare는 웹사이트를 호스팅하고 공개 게이트웨이 요청을 전달합니다. IP 주소, 요청 시각, URL과 브라우저 정보를 받을 수 있으며, 게이트웨이 요청에는 조회한 공개 주소가 포함될 수 있습니다. 이 페이지는 분석 도구나 추적 쿠키를 추가하지 않습니다. 언어 선택은 이용자 기기의 localStorage에 저장합니다.

macOS의 Sparkle은 약 한 시간마다 업데이트를 확인하고 GitHub와 그 다운로드 서비스에서 배포본을 받습니다. 이 서비스는 요청에서 IP 주소와 앱·버전 정보를 받을 수 있습니다. Sparkle은 앱에 포함된 업데이트 도구이며 별도의 동해 분석 서비스가 아닙니다. 선택적인 시스템 프로파일 전송은 이전에 저장된 동의를 포함해 비활성화되어 있습니다. Apple, Cloudflare, GitHub와 Google은 미국 기반 제공업체이며 다른 국가에서도 데이터를 처리할 수 있습니다. 처리 장소와 보유기간은 각 서비스에 따르며, 동해는 제공업체가 관리하는 로그의 삭제를 약속할 수 없습니다.

**진단, 이메일과 삭제 요청**

진단 보고서는 클립보드에만 복사되며 앱이 자동으로 업로드하지 않습니다. 별도의 원격 사용 분석 업로드는 없습니다. 보고서나 개인정보 문의를 보내면 이메일 주소, 메시지와 첨부파일을 받습니다. privacy@eastsea.xyz는 Cloudflare Email Routing을 통해 운영자의 Gmail 사서함으로 전달되므로 Cloudflare와 Google도 해당 메일을 처리합니다.

우리가 관리하는 데이터의 열람·정정·삭제·처리 정지는 privacy@eastsea.xyz로 요청하세요. 관련 공개 주소나 노드 키와 원하는 기록을 알려주시고, 개인키나 DeviceCheck 토큰은 보내지 마세요. 삭제 가능한 항목과 보관해야 하는 이유를 설명하겠습니다. 등록기 연결 기록을 삭제하면 향후 일일 확인이 불가능할 수 있으며 Apple의 비트는 초기화되지 않습니다. 공개 체인 이력이나 다른 참여자의 사본을 지우거나 회수할 수 없으며 제공업체 관리 기록의 삭제도 약속할 수 없습니다. 로컬 파일과 설정은 이용자 기기에서 제거할 수 있습니다.

**연락처와 변경**

개인정보 문의: Pipln 개인정보 보호담당자, privacy@eastsea.xyz. 방침을 바꾸면 시행일과 함께 이 페이지에 게시하고 중요한 변경은 앱에서도 설명합니다.

[Privacy policy / 개인정보 처리방침](https://eastsea.xyz/privacy) · English, 한국어, 日本語, 简体中文, Español.

### 5. 무보증 ("AS IS")
본 소프트웨어는 "있는 그대로(AS IS)", "이용 가능한 상태로(AS AVAILABLE)" 제공되며, 명시적이든 묵시적이든 다음을 포함한 어떠한 보증도 하지 않습니다:
- 상품성, 특정 목적 적합성, 제3자 권리 침해 없음에 관한 묵시적 보증;
- 소프트웨어나 네트워크가 안전하거나, 중단 없이 지속되거나, 오류가 없거나, 유해 구성 요소가 없다는 보증;
- 데이터·블록·거래·잔액·합의 상태의 정확성·신뢰성·무결성에 관한 보증.

소프트웨어의 사용, 성능, 사용 불능에서 비롯되는 모든 위험은 이용자에게 남습니다. 본 약관의 어떤 내용도 법령상 배제할 수 없는 보증·조건을 배제하지 않습니다.

### 6. 책임의 한계
아래의 한계는 법령이 허용하는 범위에서만 적용됩니다. **이 한계는 고의 또는 중대한 과실로 인한 책임에는 적용되지 않으며**, 「약관의 규제에 관한 법률」과 소비자의 강행적 권리 등 법령상 배제하거나 제한할 수 없는 책임 및 권리를 배제·제한하지 않습니다.

법률이 허용하는 최대 범위 내에서, 개발자·유지보수자·기여자·서명 주체(**Pipln**, 이현종 및 프로젝트 기여자 포함)는 다음에서 비롯되는 간접·부수적·특별·징벌적·결과적 청구, 손해, 손실, 비용(자금 손실, 이익 손실, 데이터 손실, 시스템 중단, 하드웨어 손상, 행정 제재 포함)에 대해 책임을 지지 않습니다:
1. 소프트웨어 또는 네트워크의 사용이나 사용 불능;
2. 키·기기·자금의 분실;
3. 소프트웨어가 수행한 네트워크 상호작용, 또는 기기·IP 주소·노드를 겨냥한 공격;
4. 소프트웨어 사용과 관련한 규제 기관·서비스 제공자·법집행기관의 조치.

이 한계는 주장되는 법적 이론과 무관하게 적용되며, 그러한 손해의 가능성을 통보받았더라도 동일합니다. 한계의 일부가 집행 불가능하다고 판단되는 경우, 법률이 허용하는 최대 범위에서만 적용됩니다.

### 7. 현지 법령 준수
P2P 소프트웨어, 블록체인 노드, 디지털 자산의 법적 지위는 관할에 따라 다릅니다. 본 소프트웨어의 사용이 이용자에게 적용되는 모든 법령·규제·제재·수출 규정에 부합하는지 확인할 책임은 전적으로 이용자에게 있습니다.

### 8. 준거법 및 분쟁 해결
본 소프트웨어에서 발생하거나 이와 관련된 분쟁은 대한민국 법률을 준거법으로 합니다. 당사자는 제1심에 관하여 서울중앙지방법원의 전속관할에 따르기로 합의합니다.

> **TODO-for-lawyer (변호사 확인 요망 — 관할 조항):** 위 전속관할 문구는 리서치 메모(제3.5절 Redline)가 제안한 원문 그대로이다. 다만 약관규제법 제14조(소제기의 금지 등)는 소비자에게 부담이 되는 관할 합의를 제한하므로, 소비자를 상대로 하는 전속관할 조항은 그 범위에서 무효가 될 수 있다. 변호사 확인 전까지 원문을 유지하며, 확인 후 "소비자의 주소지 관할 법원 병기" 또는 "준거법만 명시" 등으로 교정한다.

---

## Privacy notice — 日本語 (ja)

Pipln Inc.はEastSeaの投票ノード登録サービスを運営します。この方針はウォレット、ノード、ウェブサイト、公開ゲートウェイ、更新とプライバシーに関する問い合わせを対象とします。施行日：2026年10月8日。

### 1. 端末の鍵と公開ネットワーク

秘密鍵は端末に残ります。アドレス、残高、取引、報酬、投票公開鍵、ノード識別子と登録イベントはチェーン上で公開され、無期限に公開される可能性があります。ネットワークから離れたりアプリを削除しても、過去のブロック、アーカイブや他者のコピーは消えません。

ピア、リレーと公開アドレス探索ネットワークは接続IPアドレスを確認できます。RPCに応答するノードは照会したアドレスを確認できます。Macのウォレットは通常まず自分のノードに問い合わせますが、リモートの代替ノードや公開ゲートウェイを使うと、そこが照会を受け取ります。

### 2. 登録と繰り返されるAppleの確認

投票ノードの登録時、Apple DeviceCheckトークンを認証済み登録サービスの鍵で暗号化します。中継する検証者は暗号文のみを受け取り、トークンを読めません。Piplnの登録サービスが復号し、HTTPSでApple Inc.（米国）に送り、正規ハードウェアと登録済みかどうかを確認します。初回登録時と、参加中の毎日の再認証時に行われます。Appleはリクエスト識別子、時刻と登録サービスの接続IPアドレスも受け取ります。

Appleは開発者のアプリに関連する端末ごとの登録ビットを保持し、アプリの再インストール後も残る場合があります。当社はAppleの保持期間を管理せず、当社の記録の削除でこのビットも消えるとは約束しません。DeviceCheckを拒否すると投票ノード登録と日次の資格確認はできませんが、ウォレットは利用できます。Touch IDは署名を承認するもので、それ自体がプライバシーへの同意ではありません。

### 3. 登録サービスが保持する情報と期間

登録サービスは生のトークンをリクエスト処理のメモリ内で使用し、登録ストアには書き込みません。Macではアプリの実行中、保護されたローカルトークンファイルを約1時間ごとに更新します。重複登録を防ぎ再認証を可能にするため、投票公開鍵、運営者アドレス、ノード識別子、ビーコンアドレスと初回登録時刻を永続保存します。このストアには自動期限や削除タイマーがなく、運営者が削除するかサービスを終了するまで記録が残ります。

同時リクエストを防ぐ登録トークンのハッシュは、リクエスト終了時に除去します。成功した日次確認の投票鍵とトークンハッシュの制限記録は、現在と過去2期間分をメモリ内に保持します。古い項目はその後の確認成功時に整理され、プロセス終了時にはすべて消えます。登録ストアは利用者のIPアドレスを保持しません。運用ログ、バックアップや提供者のログの固定保持期間は検証されておらず、30日後の削除は約束しません。

### 4. 任意の国設定と接続集計

初回起動の国設定画面に回答するまで、国の設定をローカルノードに送りません。配布設定により、説明とともに共有が選択済みの場合と、先に選択を求める場合があります。どちらも拒否してウォレットとノードの全機能を利用できます。Macの地域設定または自分で選んだ国はこのMacに残り、広域の地域区分だけを選びます。GPSやIPによる位置測定ではなく、チェーンにも記録されません。

公開presence RPCとゴシップには、未検証の観測集団の役割別、広域地域別、おおまかなバージョン別の数だけを含めます。個別ノード一覧、観測者の身元、正確な国や観測時刻は公開しません。時刻は10分単位で、3件未満の集団と希少な品質情報はより広い集団にまとめるか非表示にします。異なるMacの全数調査ではなく観測値であり、集計だけで匿名性を保証しません。共有をオフにすると今後の国設定の使用は止まりますが、受信済みの集計コピーは回収できません。

### 5. ウェブサイト、ゲートウェイと更新

Cloudflareはウェブサイトをホストし、公開ゲートウェイのリクエストを中継します。IPアドレス、リクエスト時刻、URLとブラウザ情報を受け取ることがあり、ゲートウェイのリクエストには照会した公開アドレスが含まれる場合があります。このページは分析ツールや追跡Cookieを追加しません。言語の選択は端末のlocalStorageに保存します。

macOSのSparkleは約1時間ごとに更新を確認し、GitHubとそのダウンロードサービスからリリースを取得します。これらのサービスはリクエストからIPアドレスとアプリ・バージョン情報を受け取ることがあります。Sparkleはアプリ内の更新ツールであり、別のEastSea分析サービスではありません。任意のシステムプロファイル送信は、以前の同意設定も含め無効です。Apple、Cloudflare、GitHubとGoogleは米国を拠点とし、他国でデータを処理する場合もあります。処理場所と保持期間は各サービスによるため、EastSeaは提供者管理のログの削除を約束できません。

### 6. 診断、メールと削除請求

診断レポートはクリップボードにコピーするだけで、アプリが自動アップロードすることはありません。別途のリモート利用分析のアップロードもありません。レポートや問い合わせを送ると、当社はメールアドレス、本文と添付ファイルを受け取ります。privacy@eastsea.xyzはCloudflare Email Routingで運営者のGmailに転送されるため、CloudflareとGoogleもメールを処理します。

当社が管理するデータの閲覧、訂正、削除や処理停止はprivacy@eastsea.xyzにメールで請求してください。関連する公開アドレスまたはノード鍵と対象の記録を示し、秘密鍵やDeviceCheckトークンは送らないでください。削除できる情報と保持が必要な理由を説明します。登録サービスの関連付け記録を削除すると、今後の日次確認ができなくなる場合があり、Appleのビットもリセットされません。公開チェーン履歴や他の参加者のコピーを消去・回収できず、提供者管理の記録の削除も約束できません。ローカルファイルと設定は端末で削除できます。

### 7. 連絡先と変更

プライバシー窓口：Piplnプライバシー担当、privacy@eastsea.xyz。変更した方針は施行日とともにこのページに掲載し、重要な変更はアプリ内でも説明します。

[Privacy policy / 개인정보 처리방침](https://eastsea.xyz/privacy)

---

## Privacy notice — 简体中文 (zh-Hans)

Pipln Inc.运营EastSea投票节点注册服务。本政策涵盖钱包、节点、网站、公共网关、更新和隐私请求。生效日期：2026年10月8日。

### 1. 本地密钥与公共网络

私钥保留在你的设备上。地址、余额、交易、奖励、投票公钥、节点标识符和注册事件在链上公开，可能无限期保持公开。退出网络或删除应用不会删除历史区块、存档或他人保存的副本。

对等节点、中继和公共地址发现网络可以看到连接IP地址。响应RPC的节点可以看到你查询的地址。Mac钱包通常先查询本机节点；使用远程备用节点或公共网关时，对方会收到查询。

### 2. 注册与重复的Apple验证

注册投票节点时，Apple DeviceCheck令牌使用经过认证的注册服务密钥加密。中转验证者只能收到密文，无法读取令牌。Pipln注册服务解密后，通过HTTPS将令牌发送给Apple Inc.（美国），以验证正版硬件和既有注册。首次注册时及参与期间每日重新认证时都会发送。Apple还会收到请求标识符、时间和注册服务的连接IP地址。

Apple保存与开发者应用关联的设备注册位，重新安装应用后仍可能保留。我们不控制Apple的保留期限，也不承诺删除我们的记录会清除此位。拒绝DeviceCheck会使投票节点注册和每日资格检查无法进行，但仍可使用钱包。Touch ID用于授权签名，本身不代表隐私同意。

### 3. 注册服务保存的内容与期限

注册服务在请求处理内存中使用原始令牌，不将其写入注册存储。Mac上的受保护本地令牌文件会在应用运行时约每小时刷新。为防止重复注册并允许重新认证，注册服务持久保存投票公钥、运营者地址、节点标识符、信标地址和首次注册时间。该存储没有自动到期或删除计时器，记录会保留至运营者删除或服务终止。

用于阻止并发注册请求的令牌哈希会在请求结束后移除。成功的每日检查将投票密钥和令牌哈希限流记录保存在内存中，涵盖当前及前两个周期；更早的记录在后续检查成功时清理，进程退出时全部消失。注册存储不保存用户IP地址。运营日志、备份和服务提供方日志的固定保留期限尚未验证，我们不承诺30天后删除。

### 4. 可选国家设置与在线汇总

在你回答首次启动的国家设置页面之前，不会将任何国家偏好发送给本地节点。发行设置可在告知后预先选中共享，或先要求你选择；两种模式均可拒绝，且不影响钱包和节点功能。Mac的地区设置或你选择的国家留在本机，仅用于选定大范围地区分组，不使用GPS或IP定位，也不记录在链上。

公开presence RPC和gossip仅包含未经验证的观测群体按角色、宽泛地区和粗略版本划分的数量，不公开单个节点列表、观测者身份、确切国家或精确观测时间。时间按10分钟分组；不足3个的群体及稀少质量数据会合并到更大分组或隐藏。这些是观测值，并非不同Mac的普查，汇总本身不保证匿名。关闭共享会停止今后使用国家偏好，但已收到的汇总副本无法收回。

### 5. 网站、网关与更新

Cloudflare托管网站并传递公共网关请求，可能收到IP地址、请求时间、URL和浏览器信息；网关请求可能包含查询的公开地址。此页面不添加分析工具或追踪Cookie。语言选择存储在你设备的localStorage中。

macOS上的Sparkle约每小时检查更新，并从GitHub及其下载服务获取发行版。这些服务可能从请求中收到IP地址和应用、版本信息。Sparkle是应用内的更新工具，不是独立的EastSea分析服务；可选系统配置资料发送已禁用，包括以前保存的同意设置。Apple、Cloudflare、GitHub和Google是美国服务提供方，也可能在其他国家处理数据。处理地点及保留期限取决于各项服务，EastSea无法承诺删除提供方控制的日志。

### 6. 诊断、邮件与删除请求

诊断报告仅复制到剪贴板，应用不会自动上传，也没有单独的远程使用分析上传。如果你发送报告或隐私请求，我们会收到你的邮件地址、正文和附件。privacy@eastsea.xyz通过Cloudflare Email Routing转发至运营者的Gmail邮箱，因此Cloudflare和Google也会处理这些邮件。

若要请求访问、更正、删除或停止处理我们控制的数据，请发送邮件至privacy@eastsea.xyz。说明相关公开地址或节点密钥及所需记录，切勿发送私钥或DeviceCheck令牌。我们会解释哪些内容可删除及需要保留的原因。删除注册服务的绑定记录可能阻止今后的每日检查，并不会重置Apple的注册位。我们无法删除公共链历史或收回其他参与者的副本，也无法承诺删除提供方控制的记录。本地文件及偏好可在你自己的设备上移除。

### 7. 联系与变更

隐私联系人：Pipln隐私负责人，privacy@eastsea.xyz。更新的政策会在此处发布并注明生效日期，重大变化也会在应用中说明。

[Privacy policy / 개인정보 처리방침](https://eastsea.xyz/privacy)

---

## Privacy notice — Español (es)

Pipln Inc. opera el servicio de registro de nodos de votación de EastSea. Esta política cubre la cartera, el nodo, el sitio web, la pasarela pública, las actualizaciones y las solicitudes de privacidad. Vigente desde el 8 de octubre de 2026.

### 1. Claves locales y red pública

Las claves privadas permanecen en tu dispositivo. Las direcciones, saldos, transacciones, recompensas, claves públicas de votación, identificadores de nodos y eventos de registro son públicos en la cadena y pueden seguir siendo públicos indefinidamente. Salir de la red o borrar la app no elimina bloques anteriores, archivos ni copias de terceros.

Los pares, relés y la red pública de descubrimiento de direcciones pueden ver las direcciones IP de conexión. Los nodos RPC pueden ver las direcciones que consultas. La cartera de Mac suele consultar primero su propio nodo; al usar un nodo remoto alternativo o la pasarela pública, este recibe la consulta.

### 2. Registro y verificaciones repetidas con Apple

Al registrar un nodo de votación, el token de Apple DeviceCheck se cifra con la clave autenticada del registrador. Los validadores intermediarios reciben texto cifrado y no pueden leerlo. El registrador de Pipln lo descifra y lo envía a Apple Inc. (EE. UU.) mediante HTTPS para comprobar el hardware y registros previos. Esto ocurre en el registro inicial y en la verificación diaria mientras participas. Apple también recibe un identificador y la hora de la solicitud, y la IP de conexión del registrador.

Apple conserva un bit de registro por dispositivo asociado a las apps del desarrollador; puede sobrevivir a una reinstalación. No controlamos su plazo de conservación ni prometemos que borrar nuestros registros elimine ese bit. Rechazar DeviceCheck impide registrar el nodo de votación y verificar su elegibilidad diaria; la cartera sigue disponible. Touch ID autoriza una firma y no constituye por sí mismo consentimiento de privacidad.

### 3. Qué conserva el registrador y durante cuánto tiempo

El registrador usa el token original en la memoria de la solicitud y no lo escribe en su almacén de registros. En tu Mac, un archivo local protegido se actualiza aproximadamente cada hora mientras la app está abierta. Para impedir registros duplicados y permitir verificaciones posteriores, el registrador conserva de forma persistente la clave pública de votación, dirección del operador, identificador del nodo, dirección de baliza y hora del primer registro. Este almacén no tiene caducidad ni borrado automático: los registros permanecen hasta que el operador los elimina o retira el servicio.

Los hashes de tokens para impedir solicitudes de registro simultáneas se eliminan al terminar la solicitud. Las verificaciones diarias satisfactorias guardan límites por clave y hash del token en memoria para el período actual y los dos anteriores; las entradas más antiguas se depuran tras otra verificación satisfactoria, y todas desaparecen al cerrar el proceso. El almacén de registros no conserva IP de clientes. No hemos verificado un plazo fijo para registros operativos, copias de seguridad o registros de proveedores, y no prometemos borrarlos a los 30 días.

### 4. País opcional y presencia agregada

Debes responder la pantalla de país del primer inicio antes de que se envíe cualquier preferencia de país al nodo local. Una distribución puede preseleccionar el uso compartido con un aviso o pedir primero tu elección; ambos modos permiten rechazarlo sin perder funciones de la cartera o el nodo. El país de la configuración regional del Mac o el que elijas permanece en este Mac y solo selecciona una región amplia. No usa GPS ni geolocalización por IP y no se registra en la cadena.

La RPC pública de presencia y el gossip solo contienen recuentos de grupos observados sin verificar, por función, región amplia y versión aproximada. No exponen listas de nodos individuales, identidad del observador, país exacto ni hora exacta de observación. Los tiempos se agrupan en intervalos de 10 minutos; los grupos de menos de tres y los datos de calidad poco frecuentes se agrupan de forma más amplia o se ocultan. Son observaciones, no un censo de Macs distintos, y la agregación no garantiza anonimato. Desactivar el uso compartido detiene el uso futuro de la preferencia de país, pero no recupera copias agregadas ya recibidas.

### 5. Sitio web, pasarela y actualizaciones

Cloudflare aloja el sitio web y transporta las solicitudes de la pasarela pública. Puede recibir tu IP, hora, URL e información del navegador; las solicitudes de la pasarela pueden incluir la dirección pública consultada. La página no añade herramientas de análisis ni cookies de seguimiento. Tu elección de idioma se guarda en el localStorage del dispositivo.

En macOS, Sparkle busca actualizaciones aproximadamente cada hora y descarga versiones desde GitHub y sus servicios de descarga. Estos servicios pueden recibir tu IP e información de la app y su versión en las solicitudes. Sparkle es el actualizador de la app, no un servicio separado de análisis de EastSea; el envío opcional del perfil del sistema está desactivado, incluso si había una autorización anterior. Apple, Cloudflare, GitHub y Google son proveedores con sede en EE. UU. y pueden procesar datos en otros países. La ubicación y conservación dependen de sus servicios; EastSea no puede prometer borrar registros controlados por proveedores.

### 6. Diagnósticos, correo y solicitudes de borrado

El informe de diagnóstico solo se copia al portapapeles; la app no lo sube automáticamente. No hay una carga separada de análisis remoto de uso. Si envías un informe o solicitud, recibimos tu correo, mensaje y adjuntos. privacy@eastsea.xyz usa Cloudflare Email Routing para reenviar los mensajes al Gmail del operador, por lo que Cloudflare y Google también los procesan.

Escribe a privacy@eastsea.xyz para solicitar acceso, corrección, borrado o cese del tratamiento de datos que controlamos. Indica la dirección pública o clave del nodo y los registros afectados; nunca envíes una clave privada ni un token de DeviceCheck. Explicaremos qué se puede borrar y cualquier motivo para conservarlo. Borrar un vínculo del registrador puede impedir futuras verificaciones diarias y no restablece el bit de Apple. No podemos borrar el historial público de la cadena ni recuperar copias de otros participantes, y no prometemos borrar registros de proveedores. Los archivos y preferencias locales se pueden eliminar en tu dispositivo.

### 7. Contacto y cambios

Contacto: responsable de privacidad de Pipln, privacy@eastsea.xyz. Publicamos las políticas actualizadas aquí con su fecha de entrada en vigor; los cambios sustanciales también se explican en la app.

[Privacy policy / 개인정보 처리방침](https://eastsea.xyz/privacy)
