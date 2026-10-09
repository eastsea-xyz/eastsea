# EastSea: Legal Notice, Terms of Use, and Limitation of Liability

**Last Updated:** October 9, 2026
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

1. **Peer-to-peer connections.** The Software connects to other nodes over encrypted QUIC connections (iroh). Nodes publish and look up their network addresses as records on the public BitTorrent Mainline DHT (BEP 44). The DHT is used only for addresses; the Software does not download, share, or store any files through BitTorrent.
2. **Relays.** When two nodes cannot connect directly, traffic passes through public relay servers. It stays end-to-end encrypted.
3. **What others can see.** Other nodes can see your IP address. Nodes that answer your wallet's requests can see which addresses you look up. On a Mac, the wallet asks its own node first.
4. **Voting-node registration.** Joining as a voting node sends an Apple DeviceCheck token to the registration service, currently run by Pipln, which checks it with Apple. Apple does not sponsor or endorse EastSea. What the registration service receives and how it is handled is described in the Privacy Policy (https://eastsea.xyz/privacy).
5. The Software contains no analytics. Other than the registration data described in item 4 and in the Privacy Policy, we collect no personal data through the Software.

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

Any dispute arising out of or in connection with the Software shall be governed by the laws of the Republic of Korea. Proceedings shall be brought before a court that has jurisdiction under the Civil Procedure Act of the Republic of Korea. Nothing in these terms limits your right to bring proceedings in the court of your domicile or in any other court competent under mandatory law.


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
1. **P2P 연결.** 소프트웨어는 다른 노드와 암호화된 QUIC 연결(iroh)로 통신합니다. 노드는 자신의 네트워크 주소를 공개 BitTorrent Mainline DHT(BEP 44) 레코드로 등록·조회합니다. DHT는 주소에만 쓰이며, BitTorrent로 파일을 내려받거나 공유·저장하지 않습니다.
2. **중계.** 두 노드가 직접 연결되지 않으면 공개 중계 서버를 거치며, 종단 간 암호화는 유지됩니다.
3. **다른 노드가 볼 수 있는 것.** 다른 노드는 이용자의 IP 주소를 볼 수 있습니다. 지갑 조회에 응답하는 노드는 조회된 주소를 볼 수 있습니다. Mac에서는 지갑이 자기 노드에 먼저 묻습니다.
4. **투표 노드 등록.** 투표 노드로 참여하면 Apple DeviceCheck 토큰이 현재 Pipln이 운영하는 등록 서비스로 전송되어 Apple에 확인을 받습니다. Apple은 동해를 후원하거나 보증하지 않습니다. 등록 서비스가 받는 정보와 그 처리 방법은 개인정보 처리방침(https://eastsea.xyz/privacy)에 적혀 있습니다.
5. 소프트웨어에는 분석 도구가 없습니다. 위 4항의 등록 데이터와 개인정보 처리방침에 적힌 정보 외에, 소프트웨어를 통해 수집하는 개인정보는 없습니다.

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
본 소프트웨어에서 발생하거나 이와 관련된 분쟁은 대한민국 법률을 준거법으로 합니다. 소송은 대한민국 민사소송법에 따른 관할 법원에 제기합니다. 이 약관은 이용자가 자신의 주소지 관할 법원 등 강행법규상 인정되는 법원에 소를 제기할 권리를 제한하지 않습니다.

