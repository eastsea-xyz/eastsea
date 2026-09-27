# Aether Node: Legal Disclaimer, Terms of Use, and Limitation of Liability

**Last Updated:** September 28, 2026  
**Primary Language:** English (Authoritative Version)  
*(한국어 번역본은 본 문서 하단에 첨부되어 있습니다)*

---

## 1. Experimental Open-Source Software & Research Notice

**Aether Node** (the "Software") is an open-source, decentralized, experimental peer-to-peer (P2P) research implementation of a distributed ledger architecture (a validator committee with BLS threshold finality, EVM-compatible execution, and on-device verification of balances). 

The Software is distributed under the MIT / Apache 2.0 open-source licenses for scientific research, educational exploration, and software engineering development purposes. By downloading, compiling, executing, distributing, or interacting with the Software, you explicitly acknowledge and agree to all terms, disclaimers, and liability waivers set forth in this document. If you do not agree with these terms, do not install, run, or use the Software.

---

## 2. No Financial, Investment, or Securities Advice; Utility of Test Tokens

1. **Simulated Research Tokens:** Any digital assets, rewards, balances, tokens (including "AETH"), gas units, or smart contract states displayed, minted, transferred, or calculated within the Software are strictly experimental computational artifacts designed to benchmark consensus rounds, validate throughput, and test state transitions.
2. **No Monetary Value:** Tokens and validator rewards generated or recorded by the Software **possess no monetary value, no intrinsic financial value, and cannot be redeemed for fiat currency, securities, or legal tender**.
3. **No Financial Offering:** The Software does not constitute an Initial Coin Offering (ICO), securities offering, collective investment scheme, banking service, remittance service, or Virtual Asset Service Provider (VASP) activity under any jurisdiction (including US SEC, FinCEN, EU MiCA, and South Korea's Virtual Asset User Protection Act).
4. **No Financial Advice:** Nothing contained in the Software, its source code, user interfaces, or associated documentation constitutes financial, investment, legal, tax, or accounting advice. No representation or guarantee of future profit, capital appreciation, or economic yield is expressed or implied.

---

## 3. P2P Networking & Open-Standard BitTorrent Mainline DHT Usage

1. **Protocol-Level Usage Only:** The Software utilizes the public, open-standard **BitTorrent Mainline DHT (BEP 5 Kademlia Distributed Hash Table)** protocol solely and exclusively as a decentralized, serverless routing mechanism to discover IP and port network endpoints of active Aether peer nodes.
2. **Absolute Zero Copyrighted Content:** 
   - The Software **DOES NOT** download, upload, store, host, index, mirror, cache, or distribute any copyrighted materials, media files, movies, music, video games, torrent payload files, or proprietary digital content.
   - All network packets exchanged over the DHT network consist solely of 20-byte cryptographic infohashes (`sha1("aether-mesh-network-v1")`) and compact 6-byte network socket addresses (IPv4:Port).
   - The Software is technically incapable of serving as a BitTorrent file-sharing client or participating in any copyright-infringing file swarms.
3. **Transparent Network Operations:** The user is fully informed that the Software initiates standard UDP and TCP socket communication with open distributed nodes across the global internet.

---

## 4. UPnP, NAT Traversal, and Network Configuration

1. **Local Router Port Mapping:** To facilitate decentralized peer-to-peer consensus without relying on centralized relay servers, the Software optionally implements Universal Plug and Play (UPnP IGD) and STUN to establish a listening port on the user's local gateway.
2. **User Control:** Port forwarding and external connectivity may be inspected, configured, or disabled at any time through command-line parameters (e.g., `--port <PORT>`) or standard operating system firewall settings.
3. **Bandwidth & ISP Terms:** The user assumes full and sole responsibility for any data bandwidth consumption, local network security, and compliance with the terms of service of their Internet Service Provider (ISP).

---

## 5. Disclaimer of Warranties ("AS IS")

THE SOFTWARE IS PROVIDED "AS IS" AND "AS AVAILABLE", WITHOUT WARRANTY OF ANY KIND, EITHER EXPRESS OR IMPLIED, INCLUDING, BUT NOT LIMITED TO:
- THE IMPLIED WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE, AND NON-INFRINGEMENT;
- ANY WARRANTY THAT THE SOFTWARE WILL BE SECURE, UNINTERRUPTED, ERROR-FREE, OR FREE FROM HARMFUL COMPONENTS OR NETWORK VULNERABILITIES;
- ANY WARRANTY REGARDING THE ACCURACY, RELIABILITY, OR INTEGRITY OF ANY DATA, BLOCKS, TRANSACTIONS, OR CONSENSUS STATES RECORDED OR TRANSMITTED BY THE SOFTWARE.

THE ENTIRE RISK ARISING OUT OF THE USE, PERFORMANCE, OR INABILITY TO USE THE SOFTWARE REMAINS SOLELY WITH YOU.

---

## 6. Limitation of Liability

TO THE MAXIMUM EXTENT PERMITTED BY APPLICABLE LAW:
IN NO EVENT SHALL THE AUTHORS, MAINTAINERS, CONTRIBUTORS, INTELLECTUAL PROPERTY HOLDERS, OR SIGNING ENTITIES (INCLUDING BUT NOT LIMITED TO **PIPLN**, HYUN JONG LEE, AND INDIVIDUAL PROJECT COLLABORATORS) BE LIABLE FOR ANY CLAIM, DAMAGES, LIABILITIES, LOSSES, COSTS, OR EXPENSES OF ANY KIND, WHETHER DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, PUNITIVE, OR CONSEQUENTIAL (INCLUDING, WITHOUT LIMITATION, DAMAGES FOR LOSS OF PROFITS, LOSS OF BUSINESS, LOSS OF DATA, SYSTEM OUTAGES, NETWORK PENETRATIONS, HARDWARE DAMAGE, OR REGULATORY FINES), ARISING OUT OF OR IN ANY WAY CONNECTED WITH:
1. THE USE OF OR INABILITY TO USE THE SOFTWARE;
2. ANY NETWORK INTERACTIONS, P2P PACKETS, OR DHT DISCOVERY TRANSACTIONS INITIATED BY THE SOFTWARE;
3. ANY ATTACKS, DDOS EVENTS, EXPLOITS, OR UNAUTHORIZED NETWORK ACTIVITIES TARGETING THE USER'S IP ADDRESS OR PORT;
4. ANY ACTIONS TAKEN BY REGULATORY AUTHORITIES, TELECOMMUNICATIONS PROVIDERS, OR LAW ENFORCEMENT BODIES REGARDING THE USER'S OPERATION OF A P2P NODE.

THIS LIMITATION OF LIABILITY APPLIES REGARDLESS OF THE LEGAL THEORY ASSERTED (WHETHER IN CONTRACT, TORT, NEGLIGENCE, STRICT LIABILITY, OR OTHERWISE), EVEN IF THE AUTHORS OR SIGNING ENTITIES HAVE BEEN ADVISED OF THE POSSIBILITY OF SUCH DAMAGES.

---

## 7. User Compliance with Local Laws

The regulatory status of peer-to-peer networking, cryptographic software, and distributed ledger nodes varies significantly across jurisdictions. It is the sole and exclusive responsibility of the user to determine whether running, compiling, or interacting with the Software complies with all applicable local, national, and international laws, regulations, sanctions, and export restrictions.

---
---

# [국문 번역본] 법적 고지, 이용 약관 및 책임의 한계 (요약)

**시행일:** 2026년 9월 16일  
*(본 국문 번역본은 이용자의 이해를 돕기 위한 참고용이며, 법적 분쟁이나 해석의 불일치가 발생할 경우 상단의 영문 공식 원본이 법적으로 우선합니다.)*

### 1. 연구 및 오픈소스 목적 명시
Aether Node는 분산 원장, 검증자 위원회의 BLS 임계 서명 확정과 기기 내 잔액 검증을 연구·실증하기 위한 **비상업적 오픈소스 실험 소프트웨어**입니다. 본 소프트웨어를 다운로드, 설치, 실행하는 모든 이용자는 본 약관 및 면책 조항에 전적으로 동의한 것으로 간주됩니다.

### 2. 가상자산 / 금융 투자 상품 비인가 고지
* 본 소프트웨어 내에서 표시되는 "AETH", 검증 보상, 잔고 및 스마트 계약 상태는 합의 처리량과 트랜잭션 동시성을 측정하기 위한 **컴퓨팅 테스트 데이터에 불과하며, 어떠한 금전적·재산적 가치도 가지지 않습니다**.
* 본 소프트웨어는 가상자산공개(ICO), 투자 유치, 유사수신 행위, 또는 금융 상품 서비스와 무관하며, 어떠한 원금 보장이나 재산상 수익도 약속하거나 보장하지 않습니다.

### 3. BitTorrent Mainline DHT 프로토콜 사용 및 저작권 침해 부존재
* 본 소프트웨어는 중앙 서버 없이 피어(노드) 간의 IP 및 포트 주소를 교환하기 위한 순수 라우팅 목적으로만 **BitTorrent의 공개 표준 Mainline DHT(BEP 5) 프로토콜**을 사용합니다.
* 본 소프트웨어는 영화, 음악, 게임 등 **어떠한 저작권 보호 저작물도 다운로드, 업로드, 저장, 스트리밍하거나 중계하지 않으며**, 통신하는 데이터는 20바이트 암호화 식별자와 IP:포트 번호에 한정됩니다.

### 4. 포트포워딩(UPnP) 및 네트워크 자원 사용
* P2P 분산 합의에 참여하기 위해 사용자의 공유기 환경에 따라 UPnP를 통한 포트(기본 8080) 개방이 발생할 수 있으며, 이용자는 언제든지 실행 옵션이나 방화벽 설정을 통해 이를 통제하거나 비활성화할 수 있습니다. 인터넷 대역폭 사용에 따른 책임은 전적으로 이용자에게 있습니다.

### 5. 무보증 및 전면적 책임 면제 (AS IS)
본 소프트웨어는 "있는 그대로(AS IS)" 제공되며, 상품성, 특정 목적에의 적합성, 무오류성 및 보안성에 대한 어떠한 명시적·묵시적 보증도 제공하지 않습니다.  
법률이 허용하는 최대한의 범위 내에서, **개발자, 기여자, 지적재산권자 및 서명 주체(Pipln, 이현종 및 프로젝트 기여자 일체)는 본 소프트웨어의 사용 또는 사용 불능으로 인해 발생하는 데이터 손실, 하드웨어 장애, 네트워크 침투, 통신사 과금, 행정 제재 등을 포함한 일체의 직·간접적 손해에 대하여 민·형사상 책임을 부담하지 않습니다.**
