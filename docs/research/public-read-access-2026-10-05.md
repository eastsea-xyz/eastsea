# 공용 읽기 RPC 노드가 정말 필요한가: 브라우저 무신뢰 읽기의 차세대 접근 (2026-10-05)

> 질문(창업자): "공용 읽기 전용 RPC 노드가 정말 필요한가? 차세대 접근법과 연구 논문을 조사하라."
> 범위: 브라우저(익스플로러·확장)가 EastSea 앱 없이 체인을 읽는 방법. 모든 외부 사실은 링크와
> 확인 날짜(2026-10-05) 또는 버전을 붙였다. 저장소 사실은 파일 경로로 근거를 단다.
> 이 문서는 이전 초안을 대체한다. 초안 내용 중 검증된 것만 다시 썼다.

## 0. 결론 먼저

1. **"신뢰하는" 공용 RPC는 필요 없다. 하지만 "데이터를 내주는 무언가"는 반드시 필요하다.**
   2026년 현재 브라우저에서 실제로 돌아가는 모든 무신뢰 라이트 클라이언트(Helios, Colibri,
   Nimbus verified proxy, Kevlar)도 결국 *신뢰하지 않는* 서버에서 증명을 받아 온다(§2).
   목표는 "서버 없음"이 아니라 "아무 서버나 써도 되는 구조(멍청하고 교체 가능한 서버)"다.
2. **우리 체인은 이 구조에 이더리움보다 훨씬 유리하다.** 위원회 그룹 키가 고정이라 블록 하나의
   확정 인증서는 고정 identity에 대한 BLS 검증 한 번으로 끝난다(`crates/light/src/lib.rs`).
   이더리움 라이트 클라이언트는 신뢰 체크포인트에서 출발해 약 27시간마다 sync committee
   교체 증명을 따라가야 한다(§2.2). 우리에겐 그 사슬도, ZK 래핑도 필요 없다.
3. **"읽기 서버"는 이미 있다.** 모든 팔로워 Mac이 iroh `aether/rpc/1`로 같은 JSON-RPC를
   공개 제공하고 있다(`crates/node/src/main.rs`의 `aether_net::serve(... rpc::handle_value ...)`).
   빠진 것은 **브라우저용 전송 계층**과 **브라우저 측 검증 범위**다.
4. **정적 호스팅으로 대체되는 것은 "이력"뿐이다.** 블록·인증서·era 파일은 불변이라 CDN/R2/토렌트에
   올리고 브라우저가 스스로 검증할 수 있다. **상태(잔액·스토리지)는 요청마다 증명이 달라
   미리 계산해 둘 수 없으므로**, 살아 있는 (신뢰하지 않는) 증명 서버가 계속 필요하다(§4.4).
5. **익스플로러 화면의 상당 부분은 지금 검증이 불가능하다.** 블록에 receipts/logs 커밋이 없어서
   (`crates/light/src/block.rs` `Payload`에 receipts root 없음) 거래 성공 여부, 이벤트, `eth_getLogs`,
   `eth_call` 결과는 어떤 전송 방식을 쓰든 "노드 말을 믿는" 데이터다(§4.5).
6. 권고: **(a) 베타**에는 읽기 전용 허용 목록을 둔 팔로워 1대를 Cloudflare Tunnel 뒤에 두는
   "얇은 공개 게이트웨이"를 익스플로러 전용으로 열고 "검증 안 됨" 표기를 유지한다(확장 기본값에는 넣지 않음).
   **(b) 차세대**는 "정적 인증 이력 피드(R2+토렌트 webseed) + iroh-wasm(릴레이)로 팔로워 Mac에서
   상태 증명 + 브라우저 wasm 검증 + 메인넷 제네시스 전 receipts 커밋 추가"다(§8).

---

## 1. 출발점: 저장소에 이미 있는 것 (2026-10-05, 브랜치 `phase1-nextgen`)

| 구성 요소 | 현재 상태 | 근거 |
|---|---|---|
| 블록 확정 증명 | 임계 BLS12-381(MinSig) 확정 인증서 1개/블록, 고정 committee identity로 검증. 조상 블록은 다음 인증 블록까지 `links`(최대 64개)로 증명 | `crates/light/src/lib.rs` (`verify_finalized_chain`, `MAX_LINKS = 64`) |
| 상태 증명 | EIP-7864식 이진 트리 증명. `verify_account`, `verify_storage`, `verify_code_hash` | `crates/light/src/lib.rs` |
| 이력 증명 | 모든 블록에 `history_root`(BLAKE3 MMR). `verify_history`, `verify_old_block`, `verify_era_root` | `crates/light/src/lib.rs`, `docs/research/history-compression-2026.md` §Status B2 |
| era 파일 | 8,192블록, zstd-19, era MMR 서브루트가 identity. **era 집계 BLS 서명은 아직 없음**(포맷에 자리만 있음) | `crates/node/src/era.rs` 모듈 주석 |
| 브라우저 wasm 검증 | `verifyAccount`(인증서+링크+상태 증명+체인 ID+높이 단조+10분 신선도) 하나만 JS로 노출. 블록 단독 검증용 JS 함수는 없음. wasm 408 KB | `crates/wasm/src/lib.rs`, `apps/extension/wasm/aether_wasm_bg.wasm` |
| 확장 전송 | `DEFAULT_RPCS = ['http://127.0.0.1:18545']`. WebTransport 미구현("not yet available in this build") | `apps/extension/src/lib/rpc.js`, `apps/extension/README.md` |
| 노드 HTTP RPC | loopback 전용 바인딩, CORS `*`, 메서드 허용 목록 없음(faucet·sendTransaction·snapshot도 같은 라우터) | `crates/node/src/rpc.rs` `serve`, `handle_value` |
| 공개 P2P 읽기 | iroh QUIC `aether/rpc/1`로 같은 `handle_value` 제공, 전역 256/피어당 16 동시성, 피어당 토큰 버킷(64, 32/s). 팔로워는 `aether_announceWalletServer`로 등록 키 서명 광고 | `crates/net/src/lib.rs`, `docs/design/08-network.md` |
| iroh 릴레이 | 공개 엔드포인트는 `RelayMode::Default`(n0 공개 릴레이 사용) | `crates/net/src/lib.rs` |
| 익스플로러 | 정적 파일, 기본 `127.0.0.1:18545`, "노드 제공, 라이트 클라이언트 검증 안 됨" 표기. 사용 RPC 12종 | `apps/explorer/README.md` |
| 이전 결정 | 팀장 검토(2026-09-29): 확장 기본값으로 우리 운영 중앙 도메인(WSS)을 두지 않는다. 팔로워 WebTransport + 인증서 해시 방향 | `docs/research/ext-remote-2026.md` 머리말, `docs/design/08-network.md` |

---

## 2. 브라우저 무신뢰 라이트 클라이언트의 현재 (2026-10)

### 2.1 프로젝트별 상태

| 프로젝트 | 상태(확인 2026-10-05) | 브라우저 | 데이터 출처 |
|---|---|---|---|
| **Helios** (a16z, Rust) | 최신 `0.12.0-rc.1`(2026-10-01), 안정 `0.11.1`(2026-02-27), npm `@a16z/helios` 0.11.1. README: "It has not been audited." | wasm + TS 바인딩(`helios-ts`), CDN 로드 예시 있음 | **execution RPC(`eth_getProof` 지원 필수, 예: Alchemy)** + consensus RPC + 2주 이내 체크포인트 [H1][H2][H3] |
| Helios **verifiable-api** | 같은 저장소에 "검증 가능한 응답을 1급 기능으로" 제공하는 REST 서버 추가 | 서버측 | 우리 `aether_getAccount`(값+증명 동봉)와 같은 발상 [H4] |
| **Colibri Stateless** (corpus.core, C) | npm `@corpus-core/colibri-stateless` 3.0.0(2026-09-17). 요청 기반, 상시 동기화 없음, `eth_getLogs` 완전성 증명, 27시간 이전 이력 검증 | JS/TS 바인딩, "small enough to run in browsers" | **prover 서버**가 응답별 증명 묶음 생성, 클라이언트는 sync committee로 검증 [C1] |
| **Lodestar light-client / prover** (ChainSafe, TS) | 2026-05-14 PR #9346로 **Lodestar 모노레포에서 분리**, 이슈 #8892: "not very useful for us". npm 마지막 1.43.0(2026-05-20) | 이론상 가능(TS) | 유지보수 축소 신호 [L1][L2][L3] |
| **Nimbus verified proxy** (Nim) | `nimbus-eth1` v0.4.2(2026-09-29) 안, 배지 "Stability: experimental" | 브라우저 아님(로컬 프록시) | 신뢰 안 하는 web3 provider + `--trusted-block-root`(1-2주 이내) [N1] |
| **Kevlar** (TS CLI) | 마지막 push 2024-12-03 | 로컬 프록시 | 사실상 정지. "Optimistic Sync"는 PoPoS 논문 기반 [K1][K2] |
| **smoldot** (Polkadot) | 브라우저 wasm 라이트 노드가 주력 컴포넌트 | 예(WebSocket/WebRTC로 풀노드 P2P) | 비-이더리움에서 가장 성숙한 "브라우저가 P2P로 직접 붙는" 선례 [S1] |

정리: **브라우저에서 돌아가는 이더리움 무신뢰 클라이언트는 실재하지만(Helios wasm, Colibri), 전부
신뢰하지 않는 HTTP 서버(RPC 또는 prover)를 필요로 한다.** Kohaku(EF 프라이버시 지갑 SDK)도
provider 추상화에 Helios·Colibri를 넣었고 README에 "NOT READY FOR PRODUCTION USE"를 명시한다 [KO1].

### 2.2 이더리움 sync committee 방식과 우리 임계 BLS 비교

이더리움 Altair 라이트 클라이언트 스펙(consensus-specs `specs/altair/light-client/sync-protocol.md`,
mainnet preset `altair.yaml`, 확인 2026-10-05) [E1][E2]:

- `SYNC_COMMITTEE_SIZE = 512`, `EPOCHS_PER_SYNC_COMMITTEE_PERIOD = 256`(= 8,192 슬롯 ≈ 27.3시간).
- 시작은 `trusted_block_root`로 `LightClientBootstrap`을 받는 것(weak subjectivity, Nimbus 문서는 1-2주 이내 권장 [N1]).
- 기간마다 `LightClientUpdate`(다음 위원회 512개 공개키 포함)를 받아 사슬을 따라가야 한다.
- 업데이트 수용 하한 `MIN_SYNC_COMMITTEE_PARTICIPANTS = 1`, 확정 적용은 참가자 ×3 ≥ 크기 ×2(초다수).
- 블록마다 참여 비트에 맞춰 공개키를 합산한 뒤 페어링 검증.
- Altair `BeaconBlockBody`의 슬래싱 연산은 `proposer_slashings`, `attester_slashings`뿐이다.
  sync committee 서명 자체에 대한 슬래싱은 정의되어 있지 않다 [E3].

우리(`crates/light/src/lib.rs`, `docs/research/history-compression-2026.md`):

| 항목 | 이더리움 sync committee | EastSea 임계 BLS |
|---|---|---|
| 서명자 | 512명 무작위 부분집합(합의 투표자 전체가 아님) | 합의 투표 위원회 자체의 임계 서명(쿼럼이 실제 확정한 것) |
| 클라이언트 신뢰 시작점 | 최근(1-2주) 체크포인트 블록 루트 | 번들된 `network.json`의 고정 `identity` |
| 계속 따라가야 하는 것 | 약 27시간마다 위원회 교체 업데이트 | 없음(그룹 키 고정. 리셰어는 같은 그룹 키 유지) |
| 블록당 검증 | 공개키 최대 512개 합산 + 페어링 | 페어링 검증 1회(서명 48-96 B) |
| 오래된 이력 | 별도 증명(`historical_summaries`) 필요, 대부분 클라이언트 미지원 [C1] | MMR `history_root` 경로 약 1-1.7 kB로 임의 과거 블록 증명 |
| 결과 | 클라이언트가 "동기화"를 유지해야 함 | **응답 하나가 자기완결적으로 검증됨**(Colibri가 이더리움에서 ZK로 겨우 얻는 성질) |

시사점: 이더리움 진영이 ZK 라이트 클라이언트를 만드는 이유는 (i) sync committee 교체 사슬 압축,
(ii) 온체인(EVM) 검증 비용 절감이다. 우리에게는 (i)이 없고 (ii)는 브라우저와 무관하다.

### 2.3 ZK 라이트 클라이언트: 브라우저에서 쓰이는가

- **SP1 Helios**(Succinct): SP1 zkVM 안에서 Helios를 돌려 다른 체인 컨트랙트가 증명을 검증하는
  **온체인** 라이트 클라이언트. v1.2.0(2026-06-15). OpenZeppelin 감사(2025-05), Zellic 보고서 있음 [Z1][Z2][Z3].
- SP1 검증기 크레이트는 `no_std`의 `wasm32-unknown-unknown`에서 Groth16/Plonk 검증만 지원한다 [Z4].
  즉 "브라우저에서 ZK 증명 검증"은 기술적으로 가능하지만, 증명은 누군가 GPU로 만들어 서버에서 내려줘야 한다.
- **Lagrange State Committees**: 옵티미스틱 롤업용 ZK 라이트 클라이언트 프로토콜(브리지용) [Z5].
  **Electron Labs**: 체인 간 온체인 검증용 [Z6]. **=nil; zkLLVM**: 저장소 마지막 push 2025-01-23 [Z7].
- **결론**: 2026-10 기준 **브라우저 지갑·익스플로러용으로 배포된 ZK 라이트 클라이언트는 찾지 못했다.**
  ZK 라이트 클라이언트는 브리지(체인→컨트랙트)용이다. 우리에게 ZK가 의미 있는 곳은 합의 검증이
  아니라 **실행 유효성**(Jolt 체인 증명, `history-compression-2026.md` §5)이다.

---

## 3. 브라우저에서 서버 없는 P2P

### 3.1 Ethereum Portal Network

- 스펙 README: "This specification is a work-in-progress and should be considered preliminary."
  전송은 **discv5 위 UDP**(TALKREQ/TALKRESP)다 [P1]. 브라우저는 UDP를 열 수 없다.
- **Trin**(Rust): README 맨 위에 "THIS PROJECT IS NO LONGER ACTIVELY MAINTAINED … things will most likely
  not work". 마지막 커밋 2025-09-23, 마지막 릴리스 v0.3.3(2025-05-28) [P2].
- **Ultralight**(TS): "under active development", 마지막 push 2025-10-30. 브라우저 클라이언트는
  "proof of concept block explorer", 동작하려면 **UDP 프록시 서비스**(또는 안드로이드 앱)가 필요하다 [P3][P4].
- **Fluffy / Nimbus Portal client**: 2025-02-14 공지에서 history·beacon·state 네트워크 참여 [P5]. 지금은
  `nimbus-eth1` 저장소의 `portal/`에 있다.
- EIP-4444 부분 이력 만료 공지(EF, 2025-07-08)는 배포 경로로 "기관 호스팅, 토렌트, P2P"를 들었고
  Portal을 언급하지 않는다. 실제 era1 파일은 ethpandaops·Nimbus HTTP 미러와 토렌트로 배포된다 [P6][P7].
- **평가**: Portal은 브라우저 무서버 읽기의 대표 연구였지만, 2026-10 현재 **브라우저에서 production-ready가
  아니고**(UDP 프록시 필요) 주요 구현 하나는 유지보수가 중단되었다. 이더리움의 실제 이력 배포는
  "정적 파일 + 토렌트"로 수렴했다. 이것이 §4의 근거다.

### 3.2 libp2p 브라우저 전송

- **WebTransport + `serverCertificateHashes`**: W3C 편집자 초안(커밋 2026-09-23)은 인증서 해시를 SHA-256으로
  계산하고, "current time MUST be within the validity period … period MUST NOT exceed two weeks", ECDSA
  secp256r1(P-256) 지원을 요구한다. 해시 옵션은 전용 연결(`allowPooling=false`)에서만 허용된다 [W1][W2].
- libp2p WebTransport 스펙: 14일 인증서 2장을 겹쳐 만들고 둘의 해시를 multiaddr에 넣는 롤링을 권장하며,
  Noise 확장으로 해시 목록을 교차 검증한다 [W3]. js-libp2p `webtransport-v6.0.40`(2026-09-23) [W4].
- **브라우저 지원**: caniuse 기준 Chrome 97+, Edge 98+, Firefox 114+, **Safari/iOS Safari 26.4+**.
  `serverCertificateHashes`도 Safari 26.4에 들어갔다 [W5][W6]. 2026년에 처음으로 3대 엔진이 모두 지원한다.
- **WebRTC-direct**: libp2p 스펙 "Candidate Recommendation, r2 2026-06-20". 신뢰 CA 인증서 없이 브라우저가
  서버에 붙는다. 브라우저는 다이얼만 할 수 있고 리슨은 못 한다 [W7].
- **결정적 제약(NAT)**: WebTransport와 WebRTC-direct 모두 **서버가 공인 IP:UDP 포트로 도달 가능해야 한다.**
  브라우저는 UDP 홀펀칭에 참여할 수 없다(iroh 문서: "browsers don't support sending UDP packets to IP
  addresses from inside the browser sandbox") [I1]. 가정용 NAT 뒤 Mac 팔로워는 UPnP나 포트포워딩 없이는
  WebTransport 서버가 될 수 없다. 현재 `docs/design/08-network.md`의 WebTransport 계획은 이 점을 다루지 않는다.

### 3.3 iroh 브라우저(wasm)

- iroh 0.32에서 브라우저 알파, 0.33부터 wasm 빌드, iroh-gossip도 0.33부터 브라우저 지원 [I2][I3].
- 현재 문서: `iroh = { version = "1", default-features = false }`. 브라우저에서는 **모든 연결이 릴레이를
  거친다**(WebSocket으로 릴레이 연결). 종단 간 암호화는 유지되어 릴레이가 내용을 복호화할 수 없다.
  npm 패키지는 없다. 향후 직접 연결 수단으로 WebTransport·WebRTC를 "탐색 중"이라고 한다 [I1][I3].
- 최신 릴리스 iroh v1.3.0(2026-09-28). 우리는 `=1.2.0`(`crates/net/Cargo.toml`) [I4].
- n0 공개 릴레이: 무료지만 레이트 리밋이 있고, 수치는 공개하지 않으며, "development and hobby use"용이다.
  프로덕션은 자체 릴레이를 권장한다 [I5].
- 탐색: pkarr는 브라우저용 **HTTP 릴레이**(HTTP→Mainline DHT 게이트웨이)와 JS/WASM 바인딩을 제공한다 [I6].
- **우리에게 중요한 점**: 우리 팔로워는 이미 iroh 엔드포인트에 `RelayMode::Default`로 `aether/rpc/1`을
  제공한다. 그래서 **브라우저용 iroh-wasm 클라이언트만 만들면 노드 쪽 변경 없이 NAT 뒤 Mac 팔로워에게
  릴레이 경유로 닿을 수 있다.** WebTransport 계획보다 NAT 문제에 강하다. 대가는 (i) 릴레이가 IP와 트래픽
  양을 본다는 것, (ii) 공개 릴레이 한도 때문에 결국 자체 릴레이(내용을 못 보는 멍청한 서버)를 운영해야
  한다는 것이다.

### 3.4 현재 익스플로러(Cloudflare Pages → `127.0.0.1`)에 생긴 브라우저 변화

- **Chrome 142+ Local Network Access(LNA)**: 공개 HTTPS 사이트가 loopback·사설망으로 `fetch`하려면 사용자
  권한 프롬프트를 거쳐야 한다. 거부하면 CORS 오류처럼 실패한다. WebSocket·WebTransport·WebRTC는 "아직"
  게이팅되지 않았다(Chrome 블로그, 2025-06-09 작성 / 2025-09 갱신) [B1][B2].
- **Safari**: 2025-03 수정(WebKit 279249, RESOLVED FIXED)으로 HTTPS 문서가 localhost/loopback에 접근하는 것을
  혼합 콘텐츠 예외로 허용했다. 다만 보다 일반적인 버그 171934는 여전히 NEW다 [B3][B4]. **실기기 확인이 필요하다.**
- 함의: Pages에 올린 익스플로러는 앱 사용자에게도 Chrome에서 권한 프롬프트를 띄운다. 베타 전에 실패 시
  안내 UX(앱 설치 / 권한 허용 / 게이트웨이로 전환)를 넣어야 한다.

---

## 4. 정적이고 스스로 검증되는 퍼블리싱

### 4.1 선례

- **Ethereum era/e2store**: `.era`는 `SLOTS_PER_HISTORICAL_ROOT = 8192` 슬롯 단위의 블록+상태 묶음이다.
  파일명에 historical root 앞 4바이트가 들어가고, 스펙에 "Verifying era files" 절이 있다 [R1].
  era1(PoW)은 HTTP 미러, 토렌트(트래커 80개 이상), `checksums.txt`로 배포한다 [P7]. 우리 era 파일(8,192블록,
  MMR 서브루트)은 같은 모양이고 검증은 더 강하다. **파일 바깥에서 받은 인증서 하나와 MMR 경로로 era 전체가
  인증된다**(`verify_era_root`).
- **IPFS verified fetch**: `@helia/verified-fetch` 8.1.2(2026-09-22), "verified & trustless IPFS content on the
  web" [R2]. 단, Cloudflare 공개 IPFS 게이트웨이는 2024-08-14에 종료되었다 [R3]. IPFS는 게이트웨이 운영자에게
  의존한다.
- **BitTorrent webseed(BEP 19)**: HTTP 서버를 상시 시드로 쓰고 P2P 조각과 합친다 [R4]. "R2를 webseed로 둔
  월간 era 토렌트"가 `history-compression-2026.md` §Design 3의 "월간 토렌트 미러"와 정확히 맞는다.
- **Colibri / Helios verifiable-api**: "응답 + 증명"을 한 덩어리로 내려주는 서버. 서버는 신뢰 대상이 아니다 [C1][H4].

### 4.2 Cloudflare 한도와 비용 (확인 2026-10-05)

| 항목 | 값 | 출처 |
|---|---|---|
| Pages 파일 수 | Free 20,000 / 유료 100,000, 파일당 25 MiB, Free 빌드 500회/월 | [CF1] (2026-09-05 갱신) |
| R2 저장 | $0.015/GB-월, 무료 10 GB-월 | [CF2] |
| R2 쓰기(Class A) | $4.50/백만, 무료 100만/월 | [CF2] |
| R2 읽기(Class B) | $0.36/백만, 무료 1,000만/월, **송신 무료** | [CF2] |
| R2 공개 버킷 | `r2.dev`는 레이트 리밋 걸린 개발용. 프로덕션은 커스텀 도메인 + "Cache Everything" 규칙 | [CF3] |
| Workers | Free 10만 요청/일, 10 ms CPU. 유료 $5/월(1,000만 요청 포함, 초과 $0.30/백만) | [CF4] |
| WAF 레이트 리밋 | Free: 규칙 1개, IP 기준, 10초 주기 | [CF5] (2026-08-25 갱신) |
| Tunnel | `cloudflared`가 바깥으로만 연결(인바운드 포트 불필요) | [CF6] |

**1초 체인 정적 피드 비용 추정**(우리 계산): 블록당 객체 = 블록 바이트 + 확정 인증서. 7780 실측으로
블록 아카이브 약 530 B/블록, 확정 아카이브 약 260 B/블록이니 객체 하나는 약 0.8 kB다(`history-compression-2026.md` §Status).

- 블록마다 객체 1개: 86,400/일 × 30 ≈ 259만 쓰기/월 → (259만 − 100만) × $4.50/백만 ≈ **$7.2/월**.
  저장 증가 약 2.1 GB/월이라 무시할 수준.
- 읽기는 불변 객체(`Cache-Control: immutable`)라 CDN 캐시 적중이 대부분이다. 캐시 적중이 R2 Class B로
  잡히지 않는다는 것은 일반 CDN 동작에서 한 추론이고 문서로는 확인하지 못했다.
- 블록 수가 Pages 20,000 파일 한도를 하루 만에 넘으므로 **블록 데이터는 Pages가 아니라 R2에 둔다.**
  익스플로러 코드만 Pages에 둔다.
- 신선도: 1초 블록 + 업로드 + CDN 전파. 브라우저 wasm은 인증서가 10분보다 오래되면 거부한다
  (`verified_account`의 stale 검사). 그래서 "tip" 객체는 짧은 TTL로 매 블록 또는 수 초마다 갱신하면 충분하다.
  Cloudflare가 `max-age=1`을 그대로 존중하는지는 문서에서 확인하지 못했으니 배포 시 측정한다 [CF7].

### 4.3 브라우저가 스스로 검증하는 이력 피드 설계(우리 체인용)

```
/v1/tip.json                 최신 확정 높이 h, 짧은 TTL
/v1/fin/{h}.bin              인증 블록: block bytes + finalization (+links) — 불변
/v1/era/{n}.aera             era 파일 — 불변
/v1/era/{n}.proof            era 루트의 MMR 경로(앵커 높이 명시) — 불변
/v1/torrents/{month}.torrent webseed = 위 era 경로 (BEP 19)
```

브라우저 검증: `fin/{h}`는 고정 identity로 `verify_finalized_chain`, 과거 블록과 era는 `verify_old_block`·
`verify_era_root`로 검증한다. **era 집계 BLS 서명(아직 미구현)이 없어도** 이 피드는 완전히 검증된다
(era는 이후 인증 블록의 MMR로 증명). 집계 서명은 "이후 블록 없이 era 단독 검증"을 위한 최적화일 뿐이다.

### 4.4 상태 읽기: 미리 계산할 수 없는 이유

- 상태 증명은 그 블록의 `parent_state_root`에 묶인다. 루트가 블록마다 바뀌므로 "모든 계정의 증명을 매
  블록 미리 계산해 정적으로 올리기"는 계정 수 × 블록 수라서 불가능하다.
- 대안 비교:
  1. **요청 시 증명(현재 `aether_getAccount`)**: 신뢰하지 않는 아무 노드가 응답한다. Helios·Colibri와 같은 모델.
  2. **체크포인트 스냅샷 통째로**: BLAKE3 blob + bao 스트리밍 검증(`history-compression-2026.md` §4).
     네이티브 체크포인트 동기화에는 맞지만, 7780은 상태에 증명시장 레코드만 약 180 MB라 브라우저 탭에는 무겁다.
  3. **변경분 피드**: BAL은 "어디를 건드렸나"만 담고 쓴 값은 담지 않는다(`crates/types/src/bal.rs`
     `AccountAccess`). 브라우저가 상태를 재구성할 수 없다.
- **결론**: 상태는 살아 있는 증명 서버가 필요하다. 그 서버는 신뢰 대상이 아니고 교체 가능하다.
  후보는 팔로워 Mac(iroh-wasm 릴레이 경유, 또는 공인 도달 가능한 경우 WebTransport), 그리고 과도기의
  게이트웨이다.

### 4.5 익스플로러가 실제로 필요로 하는 것과 검증 가능성

| RPC(익스플로러 README 목록) | 무엇 | 지금 브라우저 검증 가능? | 필요한 것 |
|---|---|---|---|
| `aether_status`, `eth_blockNumber` | 최신 높이 | 가능(최신 인증서) | `verifyBlock` JS 노출 |
| `aether_recentBlocks`, `aether_getBlock` | 헤더·거래 목록 | 가능(block bytes + 인증서/links, 과거는 MMR·era) | 정적 피드 또는 `aether_getFinalized` |
| 거래 포함 여부 | tx가 블록에 있나 | 가능(블록 바이트에 `txs`) | 위와 같음 |
| `aether_getReceipt` | 성공 여부·가스·로그 | **불가**(블록에 receipts 커밋 없음) | receipts root를 헤더에 추가(합의 변경) 또는 브라우저 무상태 재실행 |
| `eth_getLogs`(ERC-20 전송 목록) | 이벤트 | **불가**(커밋도, 완전성 증명도 없음) | receipts/logs 커밋 + 범위 완전성(Colibri식) |
| `eth_call`(name/symbol/잔액) | 컨트랙트 읽기 | **불가**(실행 결과). 스토리지 슬롯을 알면 `aether_getStorage` + `verify_storage`로 일부 가능 | revm-wasm 무상태 실행(Helios 방식) 또는 슬롯 직접 증명 |
| `aether_getAccount` | 잔액·nonce·코드 | **가능**(이미 `verifyAccount`) | 익스플로러에서 wasm 재사용 |
| `aether_candidates`, `aether_rewards` | 레지스트리·보상 | 상태에 있으면 스토리지 증명으로 원칙상 가능 | 메서드별 증명 경로 구현 |
| `aether_proverStatus`, `aether_history` | 노드 로컬 정보 | 본질적으로 불가(체인 데이터 아님) | "노드 의견" 라벨 유지 |

---

## 5. 공용 RPC의 프라이버시와 남용

- **IP와 주소 연결**: 공용 RPC 운영자(그리고 경로상 관찰자)는 IP와 조회 주소를 함께 본다.
  - Wang 외, "Deanonymizing Ethereum Users behind Third-Party RPC Services", **IEEE INFOCOM 2024**,
    pp. 1701-1710: 암호화된 TCP 패킷 크기와 비컨 거래 타이밍으로 주소↔IP 연결 정확도 98.70%(테스트넷),
    96.60%(메인넷) [PR1][PR2].
  - Wang 외, "Time Tells All: Deanonymization of Blockchain RPC Users with Zero Transaction Fee", arXiv
    2508.21440(2025-08-29): 원장 확정 시각과 TCP 패킷 시각의 상관으로 수수료 없이 Ethereum·Bitcoin·Solana
    RPC 사용자에 대해 95% 이상 성공. 공격자는 경로상 수동 관찰자 [PR3].
  - 함의: 무신뢰 검증이 해결하는 것은 **무결성**이지 **프라이버시**가 아니다. 무결성이 확보돼도
    "누가 어느 주소를 조회했나"는 남는다.
- **완화 수단**:
  - 정적 이력 피드는 "누군가 블록 h를 봤다"만 남겨 주소 조회보다 누출이 훨씬 적다.
  - 여러 팔로워로 질의 분산(단일 운영자 집계 방지). iroh 릴레이는 IP를 보지만 내용은 못 본다 [I1].
  - **Oblivious HTTP(RFC 9458, 2024-01)**: 릴레이는 IP만, 게이트웨이는 내용만 본다 [PR4]. 우리 게이트웨이를
    OHTTP 게이트웨이로 만들면 같은 운영자가 둘 다 갖지 않도록 분리할 수 있다.
  - **PIR(개인 정보 검색)**: EF 쪽 "Sharded PIR Design for the Ethereum State"(ethresear.ch, 2026-03-30)는
    상태를 조각내고 조각마다 맞춤 PIR을 두는 설계이고, 타이밍 사이드채널과 **머클 증명 오버헤드**(PIR 응답의
    검증)를 미해결 한계로 적는다 [PR5]. EF Kohaku 로드맵은 단기 TEE/ORAM → 장기 PIR을 상정한다 [PR6].
    **2026-10 기준 지갑에 PIR이 production으로 배포된 사례는 찾지 못했다.**
- **남용(DoS)**: 우리 HTTP RPC에는 메서드 허용 목록이 없다(`handle_value`가 `aether_faucet`,
  `aether_sendTransaction`, `aether_snapshot*`까지 처리). 그래서 Tunnel로 그대로 노출하면 안 된다.
  iroh 경로에는 `RpcGate`(동시성·토큰 버킷)가 있지만 HTTP 경로에는 없다. Cloudflare Free WAF는 IP 기준
  규칙 1개뿐이다 [CF5]. 무거운 메서드(`eth_getLogs` 범위, `eth_call` 가스, `aether_eraChunk`, `aether_snapshotChunk`)는
  노드에서 상한을 걸어야 한다.
- **규제·운영 표면**: 런치 계획은 "직접 호스팅하는 화면은 제재 명단 확인·지역 차단"을 조건으로 둔다
  (`docs/design/12-launch-plan.md` 원칙 문단). 읽기 전용 게이트웨이가 쓰기(`sendTransaction`)를 중계하지 않게 해서
  운영 표면을 최소화한다.

---

## 6. 비교표

| 접근 | 신뢰 | 운영 비용 | 지연 | 브라우저 지원(2026-10) | 우리 작업량 | 프라이버시 |
|---|---|---|---|---|---|---|
| A. 공용 RPC(검증 없음) | 운영자를 전적으로 신뢰 | 서버 1대 + Tunnel(무료). 트래픽에 비례 | 낮음(1 RTT) | 모든 브라우저(HTTPS) | 하루 이하(허용 목록 필요) | 나쁨: IP+주소 집중 [PR1][PR3] |
| B. 공용 RPC + 브라우저 wasm 검증 | 무결성은 무신뢰(인증서·증명). receipts/logs/eth_call은 여전히 신뢰 | A와 같음 | 낮음 + 검증 수 ms(미측정) | 모든 브라우저 | 2-4일(`verifyBlock` 노출, 익스플로러 wasm 연동) | A와 같음 |
| C. 정적 인증 이력 피드(R2/CDN + 토렌트) | 무신뢰(이력) | 약 $7/월 + 내보내기 프로세스 | 1초 블록 + 업로드 + CDN(초 단위) | 모든 브라우저(HTTP GET) | 1-2주(익스포터, 경로 규약, JS 검증) | 좋음: 블록 단위 조회만 노출 |
| D. 팔로워 Mac에 WebTransport | 무신뢰(증명) | 우리 서버 0, 팔로워가 부담 | 직접 QUIC, 낮음 | Chrome/Edge/Firefox/Safari 26.4+ [W5]. **NAT 뒤 Mac은 도달 불가** | 2-3주(노드 HTTP/3 서버, 인증서 롤링, 광고 해시, 확장 offscreen) | 중간: 분산되나 팔로워가 IP+주소를 봄 |
| E. iroh-wasm(릴레이 경유) → 팔로워 Mac | 무신뢰(증명) | 자체 릴레이 1-2대(내용 못 봄). 공개 n0 릴레이는 개발용 [I5] | 릴레이 1홉 추가 | iroh 문서상 브라우저 지원(npm 없음) [I3] | 1-2주(wasm 클라이언트, pkarr HTTP 탐색, 릴레이 운영). **노드 변경 없음** | 중간: 릴레이는 IP, 팔로워는 주소 |
| F. Portal식 DHT | 무신뢰 | 0(이론) | 높음(다중 홉) | **브라우저 불가**(UDP 프록시 필요) [P4] | 수개월 | 이론상 좋음 |
| G. ZK 라이트 클라이언트 | 무신뢰 + 증명자 신뢰 불필요 | GPU 증명자 | 증명 생성 지연 | 검증은 wasm 가능 [Z4], 배포 사례 없음 | 수개월, **우리에겐 불필요**(§2.2) | 증명 서버가 조회를 봄 |
| H. PIR / OHTTP | PIR: 조회 은닉. OHTTP: IP 분리 | PIR 서버 연산 큼 | PIR 높음 | OHTTP는 JS로 가능, PIR은 연구 단계 [PR5] | PIR 수개월, OHTTP 1주 | 최상(PIR) / 좋음(OHTTP) |

---

## 7. 브라우저에서 아직 production-ready가 아닌 것 (명시)

1. **Portal Network 브라우저 클라이언트**: Ultralight 브라우저는 PoC이고 UDP 프록시가 필요하다. Trin은 유지보수 중단 [P2][P4].
2. **iroh 브라우저**: 릴레이 전용, npm 패키지 없음, n0 공개 릴레이는 레이트 리밋이 있는 개발용 [I3][I5].
3. **Helios**: 감사받지 않음("It has not been audited"), 최신이 RC [H1][H2]. 실행 RPC와 consensus RPC가 여전히 필요하다.
4. **Lodestar 라이트 클라이언트/prover**: 메인 저장소에서 분리, 팀 스스로 "not very useful for us" [L2][L3].
5. **Nimbus verified proxy**: "Stability: experimental", 브라우저용 아님 [N1].
6. **Kohaku**: "NOT READY FOR PRODUCTION USE … UNAUDITED CODE" [KO1].
7. **PIR 기반 개인 읽기**: 설계·데모 단계 [PR5].
8. **브라우저 측 ZK 라이트 클라이언트**: 검증기는 wasm으로 컴파일되지만 지갑·익스플로러 배포 사례 없음 [Z4].
9. **WebTransport 인증서 해시 경로**: API는 3대 엔진에 있으나(2026 Safari 26.4부터) 서버가 공인 UDP로 도달 가능해야 하고
   2주 인증서 롤링을 해야 한다 [W1][W3][W5]. 가정용 NAT 뒤 Mac에는 그대로 적용할 수 없다.
10. **우리 쪽**: 브라우저 블록 검증 JS API 없음(`verifyAccount`만 있음), receipts/logs 커밋 없음, era 집계 서명 없음,
    확장 WebTransport 없음, 노드 HTTP RPC에 공개용 허용 목록 없음.

---

## 8. 권고

### (a) 다음 주 베타: "얇은 공개 읽기 게이트웨이"를 익스플로러 전용으로

**판단**: 베타에서 앱 없는 방문자가 익스플로러를 보게 하려면 지금 실제로 동작하는 길은 HTTPS 엔드포인트뿐이다(§3, §7).
다만 이것은 "신뢰하는 공용 RPC"가 아니라 **허용 목록·상한·캐시를 둔 교체 가능한 게이트웨이**이고,
화면에는 지금처럼 "노드 제공, 검증 안 됨"을 유지한다. 2026-09-29 팀장 결정에 따라 **확장 기본값에는 넣지 않는다.**
확장은 지금처럼 로컬 노드 또는 사용자가 지정한 HTTPS RPC를 쓰고, 잔액은 이미 wasm으로 검증된다.

최소 계획:
1. `crates/node/src/rpc.rs`: `RpcState`에 `public_read_only: bool` 추가. 켜지면 `handle_value` 앞에서 허용 목록만 통과시킨다.
   허용: 익스플로러 README의 12개 + `aether_getFinalized`, `aether_historyProof`, `aether_eraInfo`, `aether_eraProof`.
   거부: `aether_faucet`, `aether_send*`, `aether_register*`, `aether_snapshot*`, `aether_signHandoff`, `aether_reattest`, `aether_submitProof`, `aether_handoff`.
   상한: `eth_getLogs` 블록 범위, `eth_call` 가스, 요청 크기. 같은 파일의 기존 테스트 모듈에 거부/상한 테스트를 추가한다.
2. `crates/node/src/main.rs`: `--public-read-only` 플래그(기본 끔). 바인딩은 계속 loopback이고 외부 노출은 `cloudflared`만 한다.
3. 운영: 검증자가 아닌 **팔로워 1대**(별도 머신)에 `cloudflared` 터널을 둔다(인바운드 포트 없음 [CF6]). Cloudflare 레이트 리밋 규칙 1개(IP, 10 s) [CF5].
4. `apps/explorer/js/rpc.js`: 엔드포인트를 순서대로 시도한다. 먼저 `127.0.0.1:18545`(앱 사용자, Chrome LNA 프롬프트를 거침), 실패하면 게이트웨이.
   현재 소스를 상단 배지로 표시한다("내 Mac 노드" / "공개 게이트웨이 · 검증 안 됨").
5. `apps/explorer/js/app.js`: LNA 거부나 Safari 혼합 콘텐츠 차단으로 loopback이 실패했을 때 안내 문구를 보여 준다(§3.4). Safari 실기기 확인.
6. 선택(시간이 남으면): `apps/explorer`가 `apps/extension/wasm/aether_wasm.js`를 불러 계정 페이지에서 `verifyAccount`를 호출하고,
   통과하면 잔액 옆에 "인증서 검증됨"을 표시한다. 코드는 이미 있으니 연결만 하면 된다.

하지 말 것: 게이트웨이를 확장 `DEFAULT_RPCS`에 넣는 것, 쓰기 메서드 중계, 검증되지 않은 데이터를 "확인됨"으로 표기하는 것.

### (b) 차세대 목표: "정적 인증 이력 + 교체 가능한 증명 서버 + 브라우저 검증"

목표 상태: 우리 운영 서버가 꺼져도 익스플로러와 확장이 동작한다. 우리 인프라는 **내용을 신뢰받지 않는 정적 버킷과 릴레이**뿐이다.

1. **브라우저 블록 검증 API**(선행, 소): `crates/wasm/src/lib.rs`에 `verifyBlock(network, finalized_json)`,
   `verifyOldBlock(...)`, `verifyEraRoot(...)`를 노출한다(내부는 이미 `crates/light`에 있음). 같은 wasm을 익스플로러와 확장이 공유한다.
2. **정적 이력 피드**(중): 새 모듈 `crates/node/src/feed.rs` 하나. 팔로워가 확정마다 `/v1/fin/{h}.bin`, `/v1/tip.json`을 쓰고,
   봉인된 era마다 `/v1/era/{n}.aera`와 `.proof`를 R2(S3 API)에 올린다. 비용은 약 $7/월(§4.2).
   `apps/explorer/js/rpc.js`에 "feed" 소스를 추가해 블록 페이지가 RPC 없이 동작하게 한다.
   월간 era 토렌트는 R2를 webseed로 둔다(BEP 19). `history-compression-2026.md` §Design 3의 토렌트 미러와 같은 것이다.
3. **상태 증명 전송: iroh-wasm 릴레이 우선, WebTransport는 보조**(중):
   - 새 크레이트 `crates/net-wasm`(iroh `default-features = false`, 릴레이 전용 + pkarr HTTP 릴레이 탐색)이 `aether/rpc/1`로
     `aether_walletServers` 목록의 팔로워에게 직접 묻는다. **노드 변경 없음.**
   - 자체 iroh 릴레이 1-2대를 운영한다(내용을 못 보고 교체 가능한 서버). n0 공개 릴레이는 개발용이다 [I5].
   - `docs/design/08-network.md`의 WebTransport 계획은 **공인 도달 가능한 팔로워에 한한 최적화**로 범위를 좁힌다(NAT 제약, §3.2).
4. **receipts 커밋 — 메인넷 제네시스 전 결정 필요**(합의 변경): `crates/light/src/block.rs` `Payload`(그리고 노드 블록 타입)에
   `receipts_root`(BLAKE3, 기존 트리 해시와 같은 계열)를 추가하고 `verify_receipt`를 `crates/light`에 만든다.
   이것 없이는 익스플로러의 거래 상태·이벤트·토큰 전송 목록이 영원히 "노드 의견"으로 남는다(§4.5).
   Colibri식 `eth_getLogs` 범위 완전성은 그다음 단계다.
5. **프라이버시**(후순위): 게이트웨이를 OHTTP 게이트웨이로 바꾸고 릴레이를 분리한다(RFC 9458). 질의를 여러 팔로워에 분산한다.
   PIR은 EF 설계가 성숙하면 그때 재평가한다.
6. **ZK**: 합의 검증용으로는 하지 않는다(§2.2). 실행 유효성(Jolt 체인 증명)은 기존 계획대로 간다.

순서: 1 → (a)의 6 → 2 → 4(메인넷 제네시스 동결 전) → 3 → 5. 2와 3이 끝나면 (a)의 게이트웨이는 "이력은 피드, 상태는 팔로워"로
대체되어 끌 수 있다. 다만 마지막 수단(fallback)으로 남겨 두어도 신뢰 모델은 변하지 않는다.

---

## 9. 출처 (모두 2026-10-05 확인)

**라이트 클라이언트**
- [H1] Helios README — https://github.com/a16z/helios (GitHub API: push 2026-10-01)
- [H2] Helios 릴리스 0.12.0-rc.1 (2026-10-01), 0.11.1 (2026-02-27) — https://github.com/a16z/helios/releases
- [H3] helios-ts README / npm `@a16z/helios` 0.11.1 — https://github.com/a16z/helios/tree/master/helios-ts
- [H4] Helios verifiable-api — https://github.com/a16z/helios/tree/master/verifiable-api
- [C1] Colibri Stateless README, npm 3.0.0 (2026-09-17) — https://github.com/corpus-core/colibri-stateless
- [L1] Lodestar PR #9346 "move lightclient and prover to external repo" (2026-05-14) — https://github.com/ChainSafe/lodestar/pull/9346
- [L2] Lodestar issue #8892 — https://github.com/ChainSafe/lodestar/issues/8892
- [L3] npm `@lodestar/light-client`, `@lodestar/prover` 1.43.0 (2026-05-20) — https://www.npmjs.com/package/@lodestar/prover
- [N1] Nimbus verified proxy README (nimbus-eth1 v0.4.2, 2026-09-29) — https://github.com/status-im/nimbus-eth1/tree/master/nimbus_verified_proxy
- [K1] Kevlar — https://github.com/lightclients/kevlar (마지막 push 2024-12-03)
- [K2] Proofs of Proof of Stake in Sublinear Complexity — https://arxiv.org/abs/2209.08673
- [S1] smoldot README — https://github.com/smol-dot/smoldot
- [KO1] Kohaku README — https://github.com/ethereum/kohaku
- [E1] Altair light client sync protocol — https://github.com/ethereum/consensus-specs/blob/dev/specs/altair/light-client/sync-protocol.md
- [E2] mainnet preset altair.yaml — https://github.com/ethereum/consensus-specs/blob/dev/presets/mainnet/altair.yaml
- [E3] Altair beacon-chain (BeaconBlockBody slashings) — https://github.com/ethereum/consensus-specs/blob/dev/specs/altair/beacon-chain.md

**ZK**
- [Z1] SP1 Helios — https://github.com/succinctlabs/sp1-helios (v1.2.0, 2026-06-15)
- [Z2] OpenZeppelin SP1 Helios audit (2025-05-12) — https://www.openzeppelin.com/news/sp1-helios-audit
- [Z3] Zellic SP1 Helios — https://reports.zellic.io/publications/sp1-helios
- [Z4] SP1 verifier README (wasm32 Groth16/Plonk) — https://github.com/succinctlabs/sp1/tree/main/crates/verifier
- [Z5] Lagrange State Committees — https://docs.lagrange.dev/state-committees/architecture/architecture-overview
- [Z6] Router Protocol × Electron Labs — https://routerprotocol.medium.com/router-protocol-x-electron-labs-416e2ec48d35
- [Z7] =nil; zkLLVM — https://github.com/NilFoundation/zkllvm (마지막 push 2025-01-23)

**P2P / 브라우저 전송**
- [P1] Portal Network specs README — https://github.com/ethereum/portal-network-specs
- [P2] Trin README "NO LONGER ACTIVELY MAINTAINED" — https://github.com/ethereum/trin
- [P3] Ultralight — https://github.com/ethereumjs/ultralight
- [P4] Ultralight browser-client README — https://github.com/ethereumjs/ultralight/tree/master/packages/browser-client
- [P5] Nimbus Portal client (2025-02-14) — https://blog.nimbus.team/nimbus-portal-client-entering-a-portable-and-decentralised-ethereum/
- [P6] EF partial history expiry (2025-07-08) — https://blog.ethereum.org/2025/07/08/partial-history-exp
- [P7] Ethereum history endpoints — https://eth-clients.github.io/history-endpoints/
- [W1] W3C WebTransport editor's draft (index.bs, 2026-09-23) — https://w3c.github.io/webtransport/
- [W2] W3C WebTransport CR snapshot (2026-07-30) — https://www.w3.org/TR/webtransport/
- [W3] libp2p WebTransport spec — https://github.com/libp2p/specs/blob/master/webtransport/README.md
- [W4] js-libp2p releases — https://github.com/libp2p/js-libp2p/releases
- [W5] caniuse WebTransport — https://caniuse.com/webtransport
- [W6] caniuse serverCertificateHashes — https://caniuse.com/mdn-api_webtransport_webtransport_options_servercertificatehashes_parameter
- [W7] libp2p WebRTC-direct spec (r2 2026-06-20) — https://github.com/libp2p/specs/blob/master/webrtc/webrtc-direct.md
- [I1] iroh wasm browser support — https://docs.iroh.computer/deployment/wasm-browser-support
- [I2] iroh 0.32 browser alpha — https://iroh.computer/blog/iroh-0-32-0-browser-alpha-qad-and-n0-future
- [I3] iroh wasm/browser language page — https://docs.iroh.computer/languages/wasm-browser
- [I4] iroh releases (v1.3.0, 2026-09-28) — https://github.com/n0-computer/iroh/releases
- [I5] iroh relays rate limiting — https://docs.iroh.computer/relays/rate-limiting
- [I6] pkarr (HTTP relays for browsers) — https://github.com/pubky/pkarr
- [B1] Chrome Local Network Access — https://developer.chrome.com/blog/local-network-access
- [B2] Chromium 142 LNA 영향 사례 — https://www.dynamsoft.com/web-twain/docs/faq/chromium-142-local-network-access-issue.html
- [B3] WebKit bug 279249 (RESOLVED FIXED) — https://bugs.webkit.org/show_bug.cgi?id=279249
- [B4] WebKit bug 171934 (NEW) — https://bugs.webkit.org/show_bug.cgi?id=171934

**정적 배포 / Cloudflare**
- [R1] e2store / era 형식 — https://github.com/status-im/nimbus-eth2/blob/stable/docs/e2store.md
- [R2] @helia/verified-fetch (8.1.2, 2026-09-22) — https://github.com/ipfs/helia-verified-fetch
- [R3] Cloudflare 공개 IPFS 게이트웨이 이전 — https://blog.cloudflare.com/cloudflares-public-ipfs-gateways-and-supporting-interplanetary-shipyard
- [R4] BEP 19 WebSeed — https://www.bittorrent.org/beps/bep_0019.html
- [CF1] Pages limits (2026-09-05) — https://developers.cloudflare.com/pages/platform/limits/
- [CF2] R2 pricing — https://developers.cloudflare.com/r2/pricing/
- [CF3] R2 public buckets — https://developers.cloudflare.com/r2/buckets/public-buckets/
- [CF4] Workers pricing — https://developers.cloudflare.com/workers/platform/pricing/
- [CF5] WAF rate limiting rules (2026-08-25) — https://developers.cloudflare.com/waf/rate-limiting-rules/
- [CF6] Cloudflare Tunnel (2026-08-04) — https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/
- [CF7] Cache rules settings (2026-09-16) — https://developers.cloudflare.com/cache/how-to/cache-rules/settings/

**프라이버시**
- [PR1] Wang 외, INFOCOM 2024 — https://ieeexplore.ieee.org/document/10621236/
- [PR2] 같은 논문 NSF PAR 사본 — https://par.nsf.gov/biblio/10568639-deanonymizing-ethereum-users-behind-third-party-rpc-services
- [PR3] Wang 외, "Time Tells All", arXiv 2508.21440 (2025-08-29) — https://arxiv.org/abs/2508.21440
- [PR4] RFC 9458 Oblivious HTTP (2024-01) — https://www.rfc-editor.org/rfc/rfc9458.html
- [PR5] Sharded PIR Design for the Ethereum State (2026-03-30) — https://ethresear.ch/t/sharded-pir-design-for-the-ethereum-state/24552
- [PR6] EF 프라이버시 로드맵 보도(The Block) — https://www.theblock.co/post/370532/ethereum-foundation-sets-end-to-end-privacy-roadmap-with-private-writes-reads-and-proving

**저장소(내부)**: `crates/light/src/lib.rs`, `crates/light/src/block.rs`, `crates/wasm/src/lib.rs`, `crates/node/src/rpc.rs`,
`crates/node/src/main.rs`, `crates/node/src/era.rs`, `crates/net/src/lib.rs`, `crates/types/src/bal.rs`,
`apps/extension/src/lib/rpc.js`, `apps/extension/README.md`, `apps/explorer/README.md`, `docs/design/08-network.md`,
`docs/design/12-launch-plan.md`, `docs/research/history-compression-2026.md`, `docs/research/ext-remote-2026.md`.
