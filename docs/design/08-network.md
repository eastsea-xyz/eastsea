# 08. 네트워크·DA·배포

## 계층

| 계층 | 구현 | 대상 |
|---|---|---|
| 검증자 간 합의 메시지 | Commonware `p2p` (인증된 검증자 목록) | 위원회 |
| 블록·인증서·증명 gossip | Commonware `broadcast` → 공개 노드에는 iroh gossip 재전송 | 전체 |
| 공개 노드 연결 | iroh 1.2 (QUIC, 홀펀칭, 릴레이) | 지갑·검증 노드 |
| 피어 발견 | Pkarr(서명된 부트노드 목록) → DNS TXT → GitHub raw → Nostr(선택) | 전체 |
| 공인 주소 | Google·Cloudflare STUN | 전체 |
| 릴레이 | 개발: n0 공개 릴레이, 운영: 자체 `iroh-relay`(Oracle A1 또는 poc 서버) | 필요 시 |

## Pkarr 부트노드 목록

- 키: 프로젝트 ed25519 (오프라인 보관). 레코드: `_aether.<pubkey>` TXT = 부트노드 endpoint 목록 + 서명.
- 갱신 주기 12시간(Mainline 만료 대비). 지갑은 Pkarr → DNS → GitHub 순으로 시도.
- 기존 `peers.json`·하드코딩 IP는 제거.

## DA 어댑터

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
