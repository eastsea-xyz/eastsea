# 03. 교체 가능 경계 (trait)

원칙 2의 구현. 각 trait은 자기 크레이트에 살고, `node/`가 조립한다. 시그니처는 1단계 착수 시 확정하되 아래를 기준으로 한다.

## Hasher (`hash/`)

```rust
pub trait Hasher: Send + Sync + 'static {
    const ID: HashId;                       // Poseidon2KoalaBear16 | Blake3
    type Digest: AsRef<[u8]> + Copy + Eq;   // 32바이트
    fn hash_leaf(&self, key: &[u8; 32], value: &[u8]) -> Self::Digest;   // 리프는 항상 선해시
    fn compress(&self, l: &Self::Digest, r: &Self::Digest) -> Self::Digest;
    fn hash_bytes(&self, data: &[u8]) -> Self::Digest;
}
```

- 체인 config가 `HashId`를 정한다. 제네시스 후 변경은 하드포크.
- Poseidon2 상수·MDS 행렬은 공개 NUMS 절차로 생성하고 `hash/src/poseidon2/params.rs`에 생성 스크립트와 함께 커밋.

## PolynomialCommitment (`proving/`)

```rust
pub trait Pcs {
    type Field;                  // KoalaBear | M31 | (격자)
    type Commitment; type Proof; type ProverData;
    fn commit(&self, polys: &[Poly<Self::Field>]) -> (Self::Commitment, Self::ProverData);
    fn open(&self, data: &Self::ProverData, points: &[Point]) -> Self::Proof;
    fn verify(&self, c: &Self::Commitment, points: &[Point], evals: &[Eval], p: &Self::Proof) -> bool;
    fn proof_size_bound(&self, n: usize) -> usize;   // ≤300KiB 예산 검사
}
```

- 실제로는 zkVM이 PCS를 내부에 품으므로, 이 trait은 **우리가 직접 만드는 재귀 집계 회로**에서 쓰인다. zkVM 자체 교체는 `Prover` trait.

## Prover / Verifier (`proving/`)

```rust
pub trait Prover: Send + Sync {
    fn system(&self) -> ProofSystemId;
    fn prove_chunk(&self, input: ChunkInput) -> Result<ChunkProof>;
    fn aggregate(&self, proofs: &[ChunkProof]) -> Result<BlockProof>;
    fn estimate_cycles(&self, input: &ChunkInput) -> u64;   // 스케줄러용
}
pub trait Verifier: Send + Sync {
    fn verify_block(&self, proof: &BlockProof, expected: &ProofIo) -> Result<()>;
}
```

- `ChunkInput = { pre_root, txs, bal_slice, witness: StateWitness }`.
- 백엔드: `JoltProver`, `StwoProver`, (`Risc0Prover` 조건부). feature flag.
- `Verifier`는 지갑에 포팅되므로 no_std + wasm 호환이어야 한다.

## Consensus (`consensus/`)

Commonware `Automaton`/`Relay`/`Committer`를 감싸는 우리 쪽 경계:

```rust
pub trait BlockProducer { fn propose(&mut self, parent: &BlockHeader, mempool: &dyn Mempool) -> Block; }
pub trait BlockValidator { fn validate(&self, block: &Block, parent: &BlockHeader) -> Result<()>; }  // BAL 일치 포함
pub trait CommitteeSelector {
    fn committee(&self, epoch: u64, seed: &Hash, candidates: &[ValidatorInfo]) -> Vec<ValidatorId>;
}
pub trait ForkChoice { fn head(&self) -> Hash; fn on_certificate(&mut self, c: &Certificate); }
```

- 합의 엔진 교체(Simplex → Minimmit/Alpenglow식)는 `consensus/engine_*.rs`만 바뀐다.

## DataAvailability (`da/`)

```rust
pub trait DaLayer: Send + Sync {
    fn post(&self, blob: &[u8]) -> Result<DaRef>;
    fn get(&self, r: &DaRef) -> Result<Bytes>;
    fn verify_inclusion(&self, r: &DaRef, proof: &DaProof) -> Result<()>;   // 라이트 노드용
    fn max_blob(&self) -> usize;
}
```

- 구현(설계): `CelestiaDa` (Lumina), `EthBlobDa` (EIP-4844), `NullDa` (로컬 테스트). 지금 코드에는 `LocalDa`만 있고 노드가 쓰지 않는다(D11 보류).

## Signer / SignatureVerifier (`crypto/`)

```rust
pub trait Signer { fn scheme(&self) -> SignerScheme; fn public_key(&self) -> PublicKey; fn sign(&self, msg: &[u8]) -> Result<Signature>; }
pub trait SigVerifier { fn verify(&self, scheme: SignerScheme, pk: &PublicKey, msg: &[u8], sig: &Signature) -> bool; }
```

- 구현: `SecureEnclaveP256` (cryptokit-rs, macOS/iOS), `SoftwareP256`, `Secp256k1`, `Ed25519`. PQ slot은 enum에 예약.

## StateRepository (`state/`)

```rust
pub trait StateRepository {
    fn get(&self, key: &UnifiedKey) -> Result<Option<Bytes>>;
    fn batch_write(&mut self, writes: &[(UnifiedKey, Option<Bytes>)]) -> Result<()>;
    fn root(&self) -> Hash;
    fn prove(&self, keys: &[UnifiedKey]) -> Result<StateWitness>;   // multiproof
    fn snapshot(&self, height: u64) -> Result<SnapshotHandle>;
}
```

- 구현: `NomtRepo`, `MemRepo` (테스트). QMDB는 필요 시.

## Mempool / OrderingPolicy (`node/`)

```rust
pub trait OrderingPolicy { fn order(&self, txs: Vec<TxEnvelope>, inclusion: &[TxHash], seed: &Hash) -> Vec<TxEnvelope>; }
```

- 구현: `FifoWithInclusion`(1단계), `SeededShuffle`, `DecryptAtCommit`(2단계). MEV 실험실은 여기서 정책을 바꾼다.

## Advisor (`advisor/`)

```rust
pub trait Advisor { fn score(&self, input: &AdvisorInput) -> AdvisorHint; }   // 절대 Result<bool> 아님: 힌트만
```
