# 02. 핵심 데이터 형식

모든 직렬화는 SSZ 또는 고정 길이 바이너리(증명 게스트 안에서 파싱 비용 최소화). JSON은 RPC·매니페스트에만.

## 트랜잭션 봉투

```rust
pub struct TxEnvelope {
    pub header: TxHeader,          // 평문. 멤풀 검증·수수료·스팸 방지용
    pub payload: TxPayload,        // Plain 또는 Encrypted
    pub signature: Signature,      // header ‖ payload_commitment 에 대한 서명
}

pub struct TxHeader {
    pub chain_id: u64,
    pub sender: Address,
    pub nonce: u64,
    pub gas: GasVector,            // {exec, state, prove}  — 3차원
    pub max_fee: FeeVector,
    pub payload_commitment: Hash,  // Encrypted일 때 복호화 후 검증
    pub scheme: SignerScheme,      // P256 | Secp256k1 | Ed25519 | (PQ 예약)
}

pub enum TxPayload {
    Plain(EvmTx),                  // alloy TxEnvelope (Legacy/2930/1559/7702)
    Encrypted { epoch: u64, ciphertext: Bytes },   // 2단계 tle
}
```

- `GasVector.prove`는 prove-gas 상한. 실행 시 Inspector가 계량하고 초과하면 revert.
- 서명자 scheme은 봉투에 명시. 계정 계약(7702 위임)이 허용한 scheme만 유효.

## 블록

```rust
pub struct Block {
    pub header: BlockHeader,
    pub body: BlockBody,
}

pub struct BlockHeader {
    pub height: u64,
    pub parent: Hash,
    pub timestamp: u64,
    pub proposer: ValidatorId,
    pub tx_root: Hash,             // body.txs 머클
    pub bal_root: Hash,            // body.bal 머클
    pub inclusion_list_root: Hash, // FOCIL 위원회 포함 목록
    pub exec_target: u64,          // 이 블록이 실행·증명해야 하는 과거 블록 높이 (= height - lag)
    pub state_root_at: (u64, Hash),// (exec_target, 그 높이의 state root)
    pub gas_used: GasVector,
    pub da_ref: Option<DaRef>,     // Celestia 높이·커밋먼트
}

pub struct BlockBody {
    pub txs: Vec<TxEnvelope>,
    pub bal: BlockAccessList,
    pub inclusion_list: Vec<TxHash>,
}
```

- **순서 먼저:** 합의는 `txs`, `bal`, `inclusion_list`만 확정한다. `state_root_at`은 lag 블록 전의 결과다.
- `exec_target = height - LAG`, LAG는 1 또는 2 (S4에서 확정).

## BAL (Block-level Access List, EIP-7928 의미론)

```rust
pub struct BlockAccessList {
    pub accounts: Vec<AccountAccess>,   // 주소 정렬
}
pub struct AccountAccess {
    pub address: Address,
    pub reads: Vec<StorageKey>,         // 정렬
    pub writes: Vec<(StorageKey, TxIndex)>,   // 마지막 쓰기 tx
    pub balance_touched: bool,
    pub nonce_touched: bool,
    pub code_touched: bool,
}
```

- 생산자가 실행하며 수집. 검증자는 재실행하며 **정확히 일치**해야 블록 유효.
- Block-STM은 `writes`로 정적 DAG를 만든다. 증명기는 BAL의 키 집합만 witness로 읽는다.

## 인증서

```rust
pub struct Certificate {
    pub height: u64,
    pub block_hash: Hash,
    pub kind: CertKind,             // Notarized | Finalized
    pub signature: BlsThresholdSig, // 고정 48바이트
    pub committee_epoch: u64,
}
```

## 증명

```rust
pub struct BlockProof {
    pub height: u64,
    pub pre_state_root: Hash,
    pub post_state_root: Hash,
    pub bal_root: Hash,
    pub proof_bytes: Bytes,         // ≤ 300 KiB
    pub system: ProofSystemId,      // Jolt/Stwo/... + 버전
}

pub struct ChunkProof { pub height: u64, pub chunk: u16, pub of: u16, pub proof_bytes: Bytes, pub io: ChunkIo }
pub struct ChunkIo { pub pre_root: Hash, pub post_root: Hash, pub tx_range: (u32, u32), pub gas: GasVector }
```

## 매니페스트 (이력 배포)

```json
{
  "version": 1,
  "chain_id": 8453001,
  "kind": "history_chunk",
  "range": { "from": 100000, "to": 100999 },
  "hash": { "algo": "blake3", "value": "…" },
  "size": 1073741824,
  "mirrors": [
    { "type": "https", "url": "https://github.com/…/releases/download/…" },
    { "type": "https", "url": "https://r2…" },
    { "type": "iroh", "ticket": "blob…" },
    { "type": "torrent", "magnet": "…", "webseeds": ["…"] }
  ],
  "signature": { "scheme": "ed25519", "key": "…", "sig": "…" }
}
```

지갑은 미러를 동시에 시도하고 첫 완료본을 해시 검증한다.

## 식별자·주소

- 주소: 이더리움 20바이트. P-256 계정도 7702 위임 EOA라 같은 형식.
- 상태 키: EIP-7864 32바이트 통합 키 (`05-state.md`).
- ValidatorId: BLS 공개키 해시.
- 체인 ID: 테스트넷 임시값, 메인넷 없음.
