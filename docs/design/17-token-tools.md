# 17. 토큰 배포 도구: 머클 청구 캠페인과 일괄 전송 (2026-09-29)

에어드랍과 다중 송금을 위한 도구 모음. 컨트랙트는 [contracts/src/MerkleDistributor.sol](../../contracts/src/MerkleDistributor.sol) 한 파일에 세 개가 들어 있고, 트리와 증명은 [scripts/merkle-build.mjs](../../scripts/merkle-build.mjs)가 만든다.

## 한 문장

> 받을 사람 목록을 CSV로 주면 스크립트가 머클 루트와 증명 JSON을 만들고, 팩토리가 그 루트로 완전히 입금된 청구 컨트랙트를 하나 뽑는다. 청구는 누구나 대신 보낼 수 있고 수수료는 없다.

## 구성

| 이름 | 하는 일 |
|---|---|
| `MerkleDistributorFactory` | 캠페인마다 `MerkleDistributor`를 하나 배포하며 같은 트랜잭션에서 전액 입금한다. CREATE2라 주소가 결정적이다(생성자별 연번 솔트) |
| `MerkleDistributor` | 한 캠페인. 생성 후에는 아무도(생성자도) 토큰을 잡을 수 없고, 트리에 적힌 사람만 받는다 |
| `TokenBatch` | Disperse 방식 ERC-20 일괄 전송. 한 번 승인(approve)하면 각 수령인에게 곧장 `transferFrom`으로 보낸다(예치 없음) |
| `scripts/merkle-build.mjs` | `address,amount` CSV → 정렬된 트리 → 루트·총액·증명 JSON. 의존성 없음(keccak 직접 구현) |

네이티브 AETH 일괄 송금은 이미 계정 자체가 한다(`AetherAccount.execute`의 여러 호출). 이 도구는 ERC-20용이다.

## 머클 규격

컨트랙트와 CLI가 같은 규칙을 쓴다. 어느 쪽을 믿어도 같은 루트가 나온다.

| 항목 | 규칙 |
|---|---|
| 리프 | `keccak256(index, account, amount)` (`abi.encodePacked`, 인덱스 포함) |
| 내부 노드 | 두 해시를 **작은 해시가 앞으로** 정렬해 붙여 해시 |
| 홀수 레벨 | 마지막 노드를 자기 짝으로 복제해 쓴다 |
| 인덱스 배정 | CSV 행을 (주소, 금액) 오름차순 정렬한 뒤 0부터 매긴다. 행 순서가 달라도 같은 루트 |
| 증명 | 리프에서 루트까지의 형제 해시 나열. 컨트랙트는 좌우를 다시 정렬해 계산하므로 방향을 몰라도 된다 |

- 리프에 인덱스가 들어가므로 같은 주소가 두 번 listed 되어도(보조금 두 건 등) 각각 따로 청구된다.
- 짝 정렬(small-first) 방식이라 트리 빌더와 검증자의 좌우 해석이 어긋날 여지가 없다.

## 캠페인 생애주기

| 단계 | 누가 | 무슨 일 |
|---|---|---|
| 생성 | 생성자(자금 낸 사람) | 토큰으로 팩토리에 승인해 두고 `create(token, root, ends, total)`. `total`이 통째로 새 캠페인으로 옮겨진다. `ends`는 종료 시각(0 = 종료 없음), 과거여서는 안 된다 |
| 청구 | **누구나** | `claim(index, account, amount, proof)`. 증명이 맞으면 토큰은 `account`에게, 가스를 낸 사람에게가 아니라. 인덱스당 한 번 |
| 종료 | — | `ends`가 지나면 청구는 멈춘다. 마지막 순간(ends 1초 전)까지는 청구된다 |
| 회수 | 생성자만 | 종료 후 `sweep()`으로 미청구 잔액을 생성자 주소로. `ends`가 0인 캠페인은 영원히 청구 가능하고 회수도 없다 |

- 설계: **immutable, 무관리자, 무수수료.** 생성자가 남겨 둔 권리는 "종료 후 회수" 하나뿐이고, 그마저 종료 시각은 캠페인 시작 때 자기가 정했다.
- 청구를 중개인(relayer)이 대신 보낼 수 있는 것은 의도된 기능이다. 받는 사람은 서명조차 필요 없다 — 누군가 (index, account, amount, proof)를 제출하면 된다. 가스 스폰서싱·지갑 없는 수령인 모두 지원.
- ERC-20은 표준 동작을 가정한다(전송 중 수수료를 떼는 토큰이면 입금액이 모자랄 수 있다). 전송 실패 시 토큰의 원래 revert 사유가 그대로 올라온다.

## TokenBatch (일괄 전송)

```
token.approve(TokenBatch, 총액)
TokenBatch.send(token, [주소들], [금액들])
```

- 각 수령인에게 `transferFrom(송금자 → 수령인)`이 곧장 실행된다. 컨트랙트가 토큰을 한 순간도 들고 있지 않다(예치형이 아니다).
- 전부 아니면 전무: 도중 하나라도 실패하면(승인이 중간에 바닥나는 경우 등) 전체가 되돌아간다.
- 배열 길이가 다르면 `LengthMismatch`.

## CLI

```
# recipients.csv: address,amount (양은 최소 단위 정수, # 주석과 헤더 줄 허용)
node scripts/merkle-build.mjs recipients.csv --token 0xTOKEN --out merkle.json
```

출력(stdout; `--out`이면 파일)은 지갑·중개인이 읽는 JSON이다. 모든 청구를 한 줄에 하나씩 쓴다:

```json
{
  "root": "0x7408…",
  "total": "44251000000000000000000",
  "count": 5,
  "claims": [
    {"index":0,"address":"0x…01","amount":"42000000000000000000000","proof":["0x…","0x…","0x…"]}
  ]
}
```

- 스크립트는 출력 전에 모든 증명을 루트에 대해 다시 검증한다(자체 점검).
- 금액은 문자열로 쓴다(JSON 수치는 정밀도 손실 위험).
- 트리·증명 생성기에 keccak256을 직접 구현했다(의존성 없음). 벡터는 `cast keccak`과 대조했고, 아래 연동 테스트가 체인 검증과 맞춰 본다.

## 테스트

```
cd contracts && forge test                      # 16개: 유효/무효 증명, 이중 청구, 종료 전후 회수,
                                                #        승인 부족 일괄 전송의 원자성, 팩토리 입금 등
node --test scripts/merkle-build.test.mjs       # 6개: keccak 벡터, CSV 검증, 정렬·결정성
```

- **연동(fixture) 테스트**: `contracts/test/fixtures/recipients.csv`(순서 섞임, 같은 주소 두 번 포함)를 CLI로 굽고, forge 테스트가 그 JSON(`merkle.json`)을 읽어 컨트랙트를 배포한 뒤 모든 청구를 온체인에서 검증한다. 솔리디티 쪽에서 트리를 다시 만들지 않는다 — CLI의 출력만으로 루트가 맞고 캠페인이 정확히 바닥나는지 본다.
- 이 저장소의 forge(0.2.0, 2023)는 JSON 치트코드가 없어서, 테스트가 JSON을 파일로 읽어 직접 파싱한다.
- fixture 갱신: CSV를 고치고 재생성 — `node scripts/merkle-build.mjs contracts/test/fixtures/recipients.csv --out contracts/test/fixtures/merkle.json` (CSV 머리에도 적어 두었다).
