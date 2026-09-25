# 09. 지갑 앱

## 구조

```
Aether.app (SwiftUI)
  ├─ Keys: CryptoKit SecureEnclave.P256.Signing (생체 정책), Keychain 메타
  ├─ UI: 잔액·송금·계약·상태 배지 / 설정(참여·증명 토글) / 엔지니어 모드(웹뷰 dashboard)
  ├─ Power: 전원·열·유휴 감시 → 역할 스위치
  └─ AetherCore.xcframework (UniFFI)
        └─ crates/ffi → node(검증 노드 모드) + proving::verifier + history downloader + rpc client
```

## UniFFI 인터페이스 (`crates/ffi/src/aether.udl` 요지)

```
namespace aether {
  Node start(NodeConfig cfg);
};
interface Node {
  Status status();                       // height, finalized, proven, da_sampled, peers
  Balance balance(string address);       // (value, proof_state: Proven|Certified|Proposed)
  TxHash submit(bytes signed_envelope);
  void set_role(Role r);                 // Verify | Validate | Prove
  Stream<Event> events();
};
interface Signer { bytes sign_digest(bytes digest); bytes public_key(); }   // Swift가 구현, SE 호출
```

- 서명은 Swift 쪽에서 한다(SE 키는 Rust로 나오지 않음). Rust는 다이제스트를 만들고 서명을 받아 봉투를 조립.
- `proof_state` 세 단계가 UI 배지의 근거.

## 계정 모델

- 첫 실행: SE P-256 키 생성 → 주소 = 7702 위임 EOA(EOA 키는 SE 키에서 파생된 secp256k1 임시 키로 7702 서명 1회, 이후 P-256 소유자만 사용). 대안: 체인 자체 규칙으로 P-256 EOA 허용(EIP-8030 방식). **1단계에서 둘 중 택일**(EVM 도구 호환은 7702 방식이 유리).
- 위임 계약: P-256 소유자 검증(P256VERIFY), 세션 키(ERC-7715), 복구 서명자(선택), scheme 확장 슬롯.
- paymaster: 테스트넷 온보딩 가스 후원.

## 역할 정책

| 역할 | 기본 | 조건 | 자원 목표 |
|---|---|---|---|
| Verify | 켜짐 | 항상 | CPU ~0, RSS ≤200MB |
| Validate | 꺼짐 | 전원 연결 권장 | 블록당 순간 1~2코어 |
| Prove | 꺼짐 | 전원 연결 + 유휴 + 온도·배터리 임계 | GPU 100% 허용, 사용자 설정 상한 |

- Power 모듈이 macOS `IOPMCopyPowerSourcesInfo`, thermal state, idle time을 읽어 역할을 낮춘다. Advisor는 여기에 "언제 올릴지" 힌트만 준다.

## 온보딩 흐름

1. 설치(DMG) → 열기 → 생체 등록 → 키 생성 (설정 화면 없음)
2. Pkarr로 부트노드 → iroh 연결 → 최신 인증서·증명 수신 → 내 계정 multiproof → "✓ 직접 검증됨"
3. 첫 화면: 잔액, 상태 배지, "네트워크에 기여하기" 카드(꺼짐)

## 서명 전 시뮬레이션

- 로컬 revm으로 tx 시뮬레이션 → 잔액 변화·로그·revert 표시 (Rabby 방식). 실패 예상 시 경고.

## 배포

- Sparkle(EdDSA 서명) + 노터라이즈 DMG. iOS는 4단계, Organization 계정 필요.
- 기존 개인 Team ID로 macOS 노터라이즈는 가능.

## 접근성·현지화

- 한국어·영어. 시스템 다크 모드. VoiceOver 라벨.
