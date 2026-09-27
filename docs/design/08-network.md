# 08. 네트워크·DA·배포

## 계층

| 계층 | 구현 | 대상 |
|---|---|---|
| 검증자 간 합의 메시지 | Commonware `p2p::authenticated::lookup` (인증된 검증자 목록) → 루프백 링크 포트 → iroh `aether/p2p/1` | 위원회 |
| 블록·인증서·증명 gossip | Commonware `broadcast` → 공개 노드에는 iroh gossip 재전송 | 전체 |
| 공개 노드 연결 | iroh 1.2 (QUIC, 홀펀칭, 릴레이) | 지갑·검증 노드 |
| 피어 발견 | Pkarr(서명된 부트노드 목록) → DNS TXT → GitHub raw → Nostr(선택) | 전체 |
| 공인 주소 | Google·Cloudflare STUN | 전체 |
| 릴레이 | 개발: n0 공개 릴레이, 운영: 자체 `iroh-relay`(Oracle A1 또는 poc 서버) | 필요 시 |

## 검증자 링크 (구현됨)

```text
검증자 A                                                   검증자 B
commonware ─tcp→ 127.0.0.1:<B 링크 포트> ─iroh QUIC(aether/p2p/1)→ Inbound ─tcp→ 127.0.0.1:<B p2p>
```

- Commonware p2p는 루프백에서만 듣는다. 외부는 iroh 엔드포인트 하나(UDP)로만 들어온다.
- TCP 연결 하나 = QUIC 양방향 스트림 하나. 인증은 Commonware ed25519 핸드셰이크가 종단 간에 한다(터널은 신뢰하지 않음).
- 주소 발견: 노드 ID → BitTorrent Mainline DHT(pkarr). n0 DNS 미사용.
- 게시 필터: 루프백·링크로컬·Tailscale/CGNAT(100.64/10, fd7a:115c:a1e0::/48) 제외.
- 경로 선택: `PublicPathSelector`가 같은 대역 경로를 절대 고르지 않는다. 직결(공인/LAN) 우선, 없으면 공개 릴레이.
- `n0-mainline` 0.6.0 `get_mutable_most_recent` 버그(첫 응답을 채택 → 이사한 노드의 옛 레코드가 새 주소를 가림)를 `vendor/`에서 패치.
- 검증(2026-09-26): 이 맥(공인 198.51.100.10)에 검증자 1~3, poc-m3(공인 203.0.113.20, 다른 회선)에 검증자 4. 양쪽 모두 `direct <상대 공인IP>` 경로. 검증자 3 정지 중에는 정족수에 원격 검증자가 필수인데도 15초에 14블록 확정, 송금 확정 후 원격에서 잔액 일치.
- 오프라인 테스트용 `--peers <i@host:port,…> --offline`은 평문 TCP.

## Pkarr 부트노드 목록

- 키: 프로젝트 ed25519 (오프라인 보관). 레코드: `_aether.<pubkey>` TXT = 부트노드 endpoint 목록 + 서명.
- 갱신 주기 12시간(Mainline 만료 대비). 지갑은 Pkarr → DNS → GitHub 순으로 시도.
- 기존 `peers.json`·하드코딩 IP는 제거.

## DA 어댑터 — 보류 (00-overview D11)

지금 코드: `crates/da`에 트레이트와 `LocalDa`(프로세스 안 구현)만 있고 노드가 쓰지 않는다. 아래는 설계.

- `CelestiaDa`: Lumina 라이트 노드 내장. 블록 body(txs+BAL) 직렬화 → blob → `DaRef{height, commitment, namespace}`.
- 지갑 검증 노드: Lumina로 DAS 샘플링 → "데이터 존재 확인" 상태 표시. 이것이 검증자를 믿지 않는 두 번째 근거(첫째는 ZK 증명).
- `EthBlobDa`: 4844 blob. 브리지 필요 시 전환. `NullDa`: 로컬.
- DA 실패 시: 블록은 제안되지만 `da_ref=None`인 블록은 확정 불가(가용 체인에만 존재).

## 이력 배포 (`history/`)

- 청크 = 1,000블록, ≤2GiB. 매니페스트(02 참조) 서명.
- 미러 게시 파이프라인(아카이브 노드 전용): 청크 생성 → GitHub Release 업로드 → R2/B2 업로드 → iroh-blobs 시딩 → Internet Archive(주 1회) → 매니페스트 갱신.
- 다운로더: 미러 동시 시도, 첫 완료본 BLAKE3 검증, 실패 미러 점수 하락.
- 2단계: librqbit 추가(webseed = 같은 HTTP 미러).

## 릴리스 배포

- Sparkle appcast → GitHub Releases. 보조: GHCR OCI, jsDelivr(50MB 이하), Filebase 핀.
- 노터라이즈 필수. `install.sh`는 DMG 다운로드 + 노터라이즈 확인(`spctl -a`)으로 바꾸고 `xattr -cr` 일괄 해제는 제거.

## 로컬 API 보안 (0단계에서 적용, 새 노드에도 유지)

- 기본 바인딩 127.0.0.1. 외부 노출은 `--public` + iroh 경유만.
- 로컬 HTTP API: 시작 시 생성한 토큰(`~/.aether/token`) 필수, `Origin`/`Host` 검사, CORS 없음.
- 원격 입력(gossip/sync)으로 상태를 직접 쓰는 경로는 존재하지 않는다. 상태는 실행 결과로만 바뀐다.
