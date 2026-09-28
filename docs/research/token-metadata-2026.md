# 차세대 체인의 토큰 메타데이터 비교와 Aether 권고안 (2026-09)

- 조사 범위: Solana, Sui, Aptos, TON, NEAR, Ethereum/L2(Base, Zora, Clanker), Hyperliquid, Monad, Berachain, Sei, Cosmos SDK, Starknet
- 목적: Aether 토큰 팩토리(ERC-20, name / symbol / logoURI ≤256, 고정 공급)가 **체인에 완전히 저장되고, 발행 후 바꿀 수 없으며, Solana 계열 도구와 EVM 도구 모두 읽을 수 있는** 메타데이터를 갖도록 필드 구성을 정한다.
- 표기: [확인] 1차 문서에서 직접 확인, [추정] 문서로 확인하지 못한 해석

---

## 1. 체인별 정리

### 1.1 Solana — Metaplex Token Metadata (기존 SPL Token)

- **표준**: SPL Token mint와 별도의 Metaplex Metadata PDA 계정. https://metaplex.com/docs/token-metadata
- **온체인 필드**: update_authority, mint, data(name, symbol, uri, seller_fee_basis_points, creators), primary_sale_happened, is_mutable, token_standard, collection, uses 등. https://raw.githubusercontent.com/metaplex-foundation/mpl-token-metadata/main/programs/token-metadata/program/src/state/metadata.rs
- **크기 제한** [확인]: `MAX_NAME_LENGTH = 32`, `MAX_SYMBOL_LENGTH = 10`, `MAX_URI_LENGTH = 200` (같은 파일)
- **오프체인 JSON (Fungible 표준)**: `name`, `symbol`, `description`, `image`만 규정한다. `external_url`, `attributes`, `properties.files`는 FungibleAsset/NFT 표준에만 있다. https://metaplex.com/docs/smart-contracts/token-metadata/token-standard
  ```json
  { "name": "...", "symbol": "...", "description": "...", "image": "https://.../logo.png" }
  ```
- **갱신 권한 / 잠금**: update authority가 갱신한다. "`Is Mutable` 속성으로 불변으로 만들면 `URI`, `Name`, `Creators`를 다시는 바꿀 수 없다." https://metaplex.com/docs/token-metadata
- **이미지**: `image`는 URI다. 대개 Arweave, IPFS 또는 https를 쓴다(같은 문서의 token-standard 페이지).
- **설명·소셜**: 공식 Fungible 스키마에는 소셜 필드가 없다. 런치패드는 `twitter`, `telegram`, `website` 같은 비표준 키를 JSON에 덧붙인다(1.1b 참고).
- **사기 방지 신호**: mint authority가 있으면 공급을 늘릴 수 있고, freeze authority가 있으면 보유자 계정을 동결할 수 있어 둘 다 위험 신호로 본다. https://www.helius.dev/docs/orb/explore-authorities , https://solana.com/docs/tokens/basics/freeze-account . Jupiter의 VRFD 인증 배지는 "정식 토큰"임을 뜻할 뿐 보증은 아니다. 평가 항목은 소셜 지지, 시가총액, organic score, 보유자 분포, 티커 고유성, 유동성이다. https://docs.jup.ag/user-docs/launch/vrfd/token-verification

### 1.1a Solana — Token-2022 `MetadataPointer` + `TokenMetadata` 확장

- **표준**: mint 계정 안에 가변 길이 TLV로 메타데이터를 넣는다. `MetadataPointer`는 메타데이터가 있는 계정 주소를 가리키며 보통 mint 자신을 가리킨다. https://solana.com/docs/tokens/extensions/metadata
- **구조체** [확인]: https://raw.githubusercontent.com/solana-program/token-metadata/main/interface/src/state.rs
  ```rust
  pub struct TokenMetadata {
      pub update_authority: MaybeNull<Address>, // "The authority that can sign to update the metadata"
      pub mint: Address,                        // "used to counter spoofing"
      pub name: String,
      pub symbol: String,
      pub uri: String,                          // "pointing to richer metadata"
      pub additional_metadata: Vec<(String, String)>, // 임의의 key-value
  }
  ```
- **크기**: 인터페이스에 고정 상한이 없다. 필드가 늘면 계정이 커지고, 늘기 전에 rent-exempt lamports를 미리 넣어 두어야 한다. https://solana.com/docs/tokens/extensions/metadata
- **잠금**: `update_authority`를 null로 바꾸면 이후 메타데이터를 변경할 수 없다(같은 문서).
- **이미지**: `uri`가 가리키는 Metaplex 형식 JSON 안에 둔다(같은 문서). additional_metadata에 넣을 수도 있지만 표준 해석은 없다 [추정].
- **Aether 관련성**: `additional_metadata: Vec<(String,String)>`는 Aether가 그대로 흉내 낼 수 있는 가장 단순한 확장 모델이다.

### 1.1b pump.fun (Solana 런치패드)

- **입력 폼**: name, 티커("short, ALL CAPS, typically 3-6 characters"), description, 이미지(정사각형, 512x512 PNG/JPG 권장), 선택 소셜(X, Telegram, 웹사이트). https://pump.fun/docs/create-coin
- **불변성**: "Name, symbol, and image are immutable". 생성 시 컨트랙트가 renounce되어 메타데이터가 불변이며 "the info cannot be changed, added, or removed". 소셜은 DexScreener, CoinGecko 같은 제3자 사이트에서 따로 붙인다. https://intercom.help/pumpfun-web/en/articles/11002198-how-to-edit-coin-image-description-and-socials
- **온체인 명령**: `create`는 name, symbol, uri와 creator pubkey를 받고, creator는 Metaplex metadata의 creators 배열에 들어간다. 총공급은 Global 계정의 `token_total_supply`(1,000,000,000,000,000 base units)로 고정된다. https://github.com/pump-fun/pump-public-docs/blob/main/docs/PUMP_PROGRAM_README.md
- **Authority**: mint authority와 update authority는 생성 시 폐기된다(위 help center 문서, https://www.helius.dev/docs/orb/explore-authorities).
- **시사점**: 성공한 런치패드의 기본값은 "고정 공급, mint·freeze 없음, 메타데이터 불변"이다.

### 1.2 Sui — Coin / CoinMetadata → Currency 표준(Coin Registry)

- **표준**: 예전에는 `coin::create_currency`로 `CoinMetadata<T>`를 만들었다. 새 방식은 `coin_registry::new_currency(_with_otw)`로 `0xc`의 공유 레지스트리에 `Currency<T>`를 등록한다. https://docs.sui.io/standards/currency
- **온체인 필드** [확인]: `decimals`, `name`, `symbol`, `description`, `icon_url`, `supply`(Fixed / BurnOnly / Unknown), `regulated`(Regulated / Unregulated / Unknown), `treasury_cap_id`, `metadata_cap_id`(Claimed / Unclaimed / Deleted), `extra_fields: VecMap`. https://docs.sui.io/references/framework/sui_sui/coin_registry
- **갱신 / 잠금**: `MetadataCap<T>`가 `set_name`, `set_description`, `set_icon_url` 등을 허가한다. `delete_metadata_cap`은 "making further updates of Currency metadata impossible. This action is IRREVERSIBLE"(같은 문서).
- **공급 공개**: `make_supply_fixed`는 "Freeze the supply by destroying the TreasuryCap"이고, `make_supply_burn_only`도 있다. 공급 상태 자체가 온체인 필드로 공개된다(같은 문서).
- **규제**: `make_regulated()`는 `DenyCapV2`(주소 차단)와 전역 정지 옵션을 만든다. https://docs.sui.io/standards/currency
- **이미지**: `icon_url`은 URL 문자열이다.
- **Display 표준**: 객체 타입별 `{field}` 템플릿(name, description, image_url, link 등)이다. 현재 V2이며 주로 NFT와 게임 아이템에 쓴다. 코인 적용은 문서에 명시가 없다. https://docs.sui.io/standards/display
- **시사점**: Sui는 **"메타데이터 잠김", "공급 고정", "규제 여부"를 enum 필드로 온체인에 노출**한다. Aether가 따라 할 공개 방식으로 가장 좋다.

### 1.3 Aptos — Fungible Asset(FA) Metadata

- **표준**: `fungible_asset` 모듈의 `Metadata` 객체. 생성 인자는 `name, symbol, decimals, icon_uri, project_uri`. https://aptos.dev/build/smart-contracts/fungible-asset
- **크기 제한** [확인]: `MAX_NAME_LENGTH = 32`, `MAX_SYMBOL_LENGTH = 32`, `MAX_URI_LENGTH = 512`. https://raw.githubusercontent.com/aptos-labs/aptos-core/main/aptos-move/framework/aptos-framework/sources/fungible_asset.move
- **갱신**: `MutateMetadataRef`를 가진 쪽이 `mutate_metadata()`로 필드를 골라서 바꿀 수 있다(같은 파일). 이 ref는 생성 시 `ConstructorRef`로만 만들 수 있으므로, 만들지 않으면 사실상 불변이다 [추정, 같은 파일의 `generate_mutate_metadata_ref(constructor_ref)` 시그니처 근거].
- **이미지**: `icon_uri`(URI). 웹사이트는 `project_uri`. description 필드는 없다.

### 1.4 TON — Jetton, TEP-64 Token Data Standard

- **레이아웃** [확인]: https://github.com/ton-blockchain/TEPs/blob/master/text/0064-token-data-standard.md
  - Off-chain: 접두 `0x01` 다음에 JSON URI(ASCII)
  - On-chain: 접두 `0x00` 다음에 dictionary. "Key is sha256 hash of string"
  - Semi-chain: on-chain dict에 `uri`를 넣어 JSON과 병합하고, 충돌하면 on-chain 값이 우선한다
- **Jetton 속성**: `uri`, `name`, `description`, `image`, `image_data`, `symbol`, `decimals`(기본 9), `amount_style`("n", "n-of-total", "%"), `render_type`("currency", "game", "hidden")
- **이미지를 체인에 저장**: `image_data`는 "Either binary representation of the image for onchain layout or base64 for offchain layout". 즉 **이미지 바이너리를 온체인에 넣는 방식이 표준에 있다**.
- **직렬화**: 큰 값은 Snake format(`0x00`, 자식 셀로 재귀 연결)이나 Chunked format(`0x01`)으로 저장한다. 명시적 크기 상한은 없다.
- **갱신**: jetton minter의 admin이 content를 바꿀 수 있다. admin을 폐기하면 고정된다 [추정, TEP-74 minter 관례].

### 1.5 NEAR — NEP-148 Fungible Token Metadata

- **구조** [확인]: `{ spec: "ft-1.0.0", name, symbol, icon: string|null, reference: string|null, reference_hash: string|null, decimals }`. https://github.com/near/NEPs/blob/master/neps/nep-0148.md
- **아이콘**: "Must be a data URL, to help consumers display it quickly while protecting user data". 최적화된 SVG를 권장하고, 라이트·다크 모드 모두에서 보이게 디자인하라고 한다.
- **reference / reference_hash**: 추가 JSON의 URI와 그 SHA-256(base64)이다. 충돌하면 reference가 우선한다.
- **보안 주의**: 배포 시점에는 아이콘 안전성을 강제할 수 없으므로 표시하는 쪽이 검증해야 한다(같은 문서).
- **시사점**: **"data URI 아이콘을 체인에 넣는다"는 Aether 목표와 가장 가까운 선례**다.

### 1.6 Ethereum과 L2

- **ERC-20**: `name`, `symbol`, `decimals`는 모두 OPTIONAL이다. 로고와 설명 필드는 없다. https://eips.ethereum.org/EIPS/eip-20
- **ERC-1046 (Final)**: ERC-20에 `tokenURI() returns (string)`를 추가한다. JSON 필드는 `interop`, `name`, `symbol`, `decimals`, `description`, `image`(폭 320-1080px 권장), `images`, `icons`(1:1). https://eips.ethereum.org/EIPS/eip-1046
- **ERC-7572 (Draft)**: `contractURI() returns (string)`와 `event ContractURIUpdated()`. 반환값은 "MAY be an offchain resource or onchain JSON data string (`data:application/json;utf8,{}`)". https://eips.ethereum.org/EIPS/eip-7572
  ```json
  { "name": "필수", "symbol": "", "description": "", "image": "image/* URI",
    "banner_image": "", "featured_image": "", "external_link": "",
    "collaborators": ["0x..."] }
  ```
  collaborators는 dapp에서 관리자급 권한을 받을 수 있다는 보안 경고가 붙어 있다(같은 문서).
- **Uniswap Token Lists 스키마** [확인]: https://raw.githubusercontent.com/Uniswap/token-lists/main/src/tokenlist.schema.json
  - 필수: `chainId`, `address`, `decimals`, `name`, `symbol`
  - `name` ≤ 60자, `symbol` ≤ 20자이며 공백 불가(`^\S+$`), `decimals` 0-255
  - `logoURI`: format uri
  - `tags` ≤ 10개
  - `extensions` ≤ 10 키, 중첩 3단계, 키 1-40자, **문자열 값 ≤ 42자**. 긴 URL은 extensions에 넣지 못한다.
- **Base / Coinbase Wallet**: 로고는 컨트랙트가 아니라 탐색기(BaseScan "Update Token Info")나 지갑 카탈로그 동기화로 붙는다. https://tokpie.io/blog/how-to-add-token-to-coinbase-wallet-base-app-guide/ (2차 자료). Coinbase Wallet은 EAS 어테스테이션으로 로고와 설명을 온체인 등록하는 MVP를 발표했다. https://x.com/CoinbaseWallet/status/1891950793864986648 . EAS는 OP Stack predeploy다. https://base.easscan.org/schemas/explore
- **Zora Coins (Base)**: 모든 코인이 고정 공급 10억 개다. 메타데이터는 EIP-7572 형식이며 `name`, `description`, `image`, `properties.category`와 선택 `animation_url`, `content`를 쓴다. 예시 이미지는 `ipfs://`이다. URI는 "can be updated by coin owners after deployment". SDK 검증기는 IPFS, HTTPS, data URI를 지원한다. https://docs.zora.co/coins , https://docs.zora.co/coins/contracts/metadata
- **Clanker (Farcaster/Base)**: 배포 입력은 name, symbol, `image`(ipfs://), `tokenAdmin`, `metadata.description`, `metadata.socialMediaUrls[]`, `context{interface, platform:"farcaster", messageId, id}`이다. https://clanker.gitbook.io/documentation/general/token-deployments/deploying-a-token . 토큰 컨트랙트의 `updateImage`, `updateMetadata`, `verify`는 "only callable by token's admin"이라 **이미지와 메타데이터가 가변**이다. https://clanker.gitbook.io/documentation/references/core-contracts/clankertoken-v3.1.0-and-v4.0.0

### 1.7 Hyperliquid — HIP-1

- **필드**: 이름·티커 "maximum 6 characters, no uniqueness constraints", `weiDecimals`, `szDecimals`(`szDecimals + 5 <= weiDecimals`), `maxSupply`, genesis 잔액. 배포 가스는 31시간짜리 더치 옥션이다. https://hyperliquid.gitbook.io/hyperliquid-docs/hyperliquid-improvement-proposals-hips/hip-1-native-token-standard
- **API**: `registerToken2`(name, szDecimals, weiDecimals, 선택 `fullName`), `userGenesis`, `genesis`, `registerSpot`, `registerHyperliquidity`. https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/deploying-hip-1-and-hip-2-assets
- **이미지와 소셜**: HIP-1 명세에는 로고 필드가 없다(위 문서). 첫 단계가 끝나면 "the token is locked in"이다.
- **시사점**: 티커가 유일하지 않으므로 **주소가 유일한 식별자**라는 전제를 UI가 지켜야 한다.

### 1.8 Monad

- 네이티브 확장 없이 표준 ERC-20을 쓴다. 로고와 메타데이터는 GitHub `monad-crypto/token-list`로 제출한다. 필수는 chainId(143), address, name, symbol, decimals(온체인 값과 일치해야 함)이고, 로고는 `logo.svg`/`logo.png`, 1:1, 최소 200x200(256 권장)이다. extensions는 coinGeckoId, bridgeInfo, crossChainAddresses. https://github.com/monad-crypto/token-list/blob/main/CONTRIBUTING.md
- "Inclusion ... does not imply endorsement, verification, or approval". https://github.com/monad-crypto/token-list

### 1.9 Berachain

- ERC-20에 `berachain/metadata` 레포(`src/tokens/mainnet.json`, chainId 80094)를 쓴다. 필수는 chainId, address, symbol, name, logoURI, decimals, tags. 로고는 PNG/JPG, 불투명 배경, 1024x1024, 파일명은 소문자 주소이며 Cloudflare에 호스팅한다. PR을 올려도 추가가 보장되지 않는다. https://github.com/berachain/metadata/blob/main/CONTRIBUTING.md

### 1.10 Sei

- 새 토큰은 ERC-20/721/1155로 만들라고 한다. "Do not use tokenfactory for new tokens". CW20/CW721은 pointer contract로 EVM에 노출한다. https://docs.sei.io/learn/dev-token-standards
- MetaMask 표시는 다른 EVM과 같고, 자동 목록에 없으면 사용자가 직접 import한다. https://docs.sei.io/evm/tokens

### 1.11 Cosmos SDK — x/bank `Metadata`

- **필드** [확인]: `description`, `denom_units`, `base`, `display`, `name`, `symbol`, `uri`("URI to a document (on or off-chain)"), `uri_hash`("sha256 hash of a document pointed by URI ... to verify that the document didn't change"). https://raw.githubusercontent.com/cosmos/cosmos-sdk/main/proto/cosmos/bank/v1beta1/bank.proto
- **권한**: genesis나 거버넌스로 설정하고, tokenfactory 모듈에서는 denom admin이 `SetDenomMetadata`를 호출한다. https://docs.junonetwork.io/developer-guides/juno-modules/tokenfactory
- **시사점**: `uri_hash`는 외부 문서의 변조를 감지하는 좋은 패턴이다(NEAR의 `reference_hash`와 같다).

### 1.12 Starknet

- OpenZeppelin Cairo ERC20의 `name() -> ByteArray`, `symbol() -> ByteArray`, `decimals() -> u8`이 전부이고, 로고 필드는 없다. https://docs.openzeppelin.com/contracts-cairo/2.x/erc20
- 로고와 인증은 avnu 토큰 API(community.avnu.fi 제출, 커뮤니티 검토, 초록 체크)로 한다. 인증된 토큰은 이 API를 쓰는 지갑과 dApp에 자동으로 나타난다. https://www.starknet-ecosystem.com/en/tokens

---

## 2. 비교표

| 체인 | 온체인 필드 | 오프체인 | 이미지 저장 | 변경 가능? | 사기 방지 신호 |
|---|---|---|---|---|---|
| Solana Metaplex | name(32) symbol(10) uri(200) creators is_mutable | JSON: name symbol description image | URI(Arweave/IPFS/https) | update authority, `is_mutable=false`로 영구 잠금 | mint/freeze authority, Jupiter VRFD |
| Solana Token-2022 | name symbol uri additional_metadata(KV) update_authority | uri JSON(Metaplex 형식) | URI | authority=null이면 잠금 | 같음 + 확장 목록 |
| pump.fun | Metaplex name/symbol/uri, creator | JSON + 소셜 | IPFS 계열 | 생성 즉시 불변 | mint/update authority 폐기, 고정 공급 |
| Sui Currency | name symbol decimals description icon_url supply regulated metadata_cap extra_fields | 없음 | icon_url(URL) | MetadataCap, delete로 영구 잠금 | **SupplyState / RegulatedState / MetadataCapState 온체인 enum** |
| Aptos FA | name(32) symbol(32) decimals icon_uri(512) project_uri(512) | 없음 | URI | MutateMetadataRef 보유 시 | ref 유무 |
| TON TEP-64 | on-chain dict(sha256 키) 또는 URI, semi-chain | JSON | `image` URI 또는 **`image_data` 바이너리 온체인** | minter admin | 제3자 목록 |
| NEAR NEP-148 | spec name symbol **icon(data URL)** reference reference_hash decimals | reference JSON | **data URL 온체인** | 컨트랙트 구현에 따름 | reference_hash |
| ERC-20 | name symbol decimals(선택) | 없음 | 없음 | 구현에 따름 | 토큰 리스트, 탐색기 |
| ERC-1046 / 7572 | tokenURI / contractURI | JSON 또는 `data:application/json` | URI | 7572는 업데이트 이벤트 | collaborators 경고 |
| Zora | ERC-20 + contractURI | EIP-7572 JSON | ipfs:// | owners가 URI 갱신 | 고정 10억 공급 |
| Clanker | ERC-20 + image, metadata, context 온체인 문자열 | 소셜 포함 JSON 문자열 | ipfs:// | admin이 image/metadata 갱신 | context(FID/cast) |
| Hyperliquid HIP-1 | name(≤6) decimals maxSupply genesis fullName | 없음 | 명세상 없음 | 배포 후 잠금 | 티커 비고유 |
| Monad / Berachain / Starknet | ERC-20 기본 | GitHub/API 토큰 리스트 | 레포/CDN 호스팅 | 리스트 PR | 리스트 검토(보증 아님) |
| Cosmos bank | name symbol description display denom_units uri uri_hash | uri 문서 | URI | genesis, gov, tokenfactory admin | uri_hash |

---

## 3. Aether 권고안

### 3.1 필드 세트 (Metaplex / Token-2022 이름 재사용)

코어 필드는 컨트랙트 스토리지에 두고 생성자에서 한 번만 기록한다.

| 필드 | 제한 | 근거 |
|---|---|---|
| `name` | UTF-8 ≤ 32바이트 | Metaplex 32, Aptos 32, 현재 런치패드 32 |
| `symbol` | `^[A-Z0-9]{1,10}$` | Metaplex 10, Uniswap `^\S+$` ≤20, pump.fun "ALL CAPS" 관례. 현재 ≤12에서 10으로 줄이면 Metaplex 도구와 완전히 호환된다 |
| `decimals` | 18 고정 | ERC-20 관례 |
| `uri` | ≤ 200바이트, 선택 | Metaplex `MAX_URI_LENGTH`. 외부 JSON을 쓸 때만 사용 |
| `image` | data URI(아래 3.2) 또는 비움 | Metaplex JSON `image`, ERC-7572 `image` |
| `description` | ≤ 280바이트, 평문 | Metaplex, Sui, Cosmos 공통 |
| `external_url` | https URL ≤ 200 | Metaplex `external_url`(7572에서는 `external_link`로 매핑) |
| `extensions.website / twitter / telegram / discord` | 각 https URL ≤ 200 | 런치패드 소셜 관례(pump.fun, Clanker `socialMediaUrls`) |
| `additionalMetadata` | `(string,string)[]` ≤ 8쌍, key ≤ 32, value ≤ 128 | Token-2022 `additional_metadata: Vec<(String,String)>` |

- `name`, `symbol`, `decimals`는 ERC-20 view 함수로 그대로 노출한다(모든 EVM 도구의 최소 공통분모).
- 공개용 view(읽기 전용 상수)도 둔다: `creator()`, `createdAt()`, `mintAuthority() == address(0)`, `freezeAuthority() == address(0)`, `metadataUpdateAuthority() == address(0)`, `supplyState() == "fixed"`. Sui의 `SupplyState`, `MetadataCapState` enum과 Solana authority 필드 이름을 합친 것이다.
- 무결성용 `metadataHash()`: 정규화한 JSON의 keccak256(또는 sha256)이다. Cosmos `uri_hash`, NEAR `reference_hash`와 같은 역할을 한다.

### 3.2 저장 방식: 토큰 안의 온체인 JSON vs URI

- **권고**: 필드는 개별 스토리지에 두고, 이미지 바이트는 SSTORE2 방식(데이터를 별도 컨트랙트 바이트코드로 배포)으로 저장한다. JSON은 `contractURI()` / `tokenURI()` 호출 시 view 함수에서 조립한다. 가스는 쓰기에만 들고 읽기는 무료다.
- **가스 비교** (Ethereum 기준 가격. Aether 가스표가 다르면 비율만 참고):
  - SSTORE로 새 슬롯에 쓰면 32바이트당 약 20,000 가스, 1 KB ≈ 640k 가스. https://www.evm.codes/#55
  - 컨트랙트 코드로 저장하면 code deposit이 바이트당 200 가스, 1 KB ≈ 200k 가스 + CREATE 32,000. Ethereum Yellow Paper의 G_codedeposit, https://ethereum.github.io/yellowpaper/paper.pdf
  - 코드 크기 상한은 24,576바이트(EIP-170)이므로 청크 하나가 약 24 KB. https://eips.ethereum.org/EIPS/eip-170
  - calldata는 non-zero 바이트당 16 가스. https://eips.ethereum.org/EIPS/eip-2028
- **이미지 상한**: data URI 원본 바이너리 ≤ 16 KB. 128x128 또는 256x256 PNG/WebP 1장이면 충분하고, 한 청크 안에 들어가며 2중 인코딩 오버헤드도 감당할 수 있다. 지원 MIME은 `image/png`, `image/webp`, `image/svg+xml`(아래 조건) 세 가지로 제한한다.
  - SVG는 NEAR가 권장하는 형식이다. `<img>`로 렌더하면 스크립트가 실행되지 않는다(MDN "SVG as an image": 스크립트와 외부 리소스 차단). https://developer.mozilla.org/en-US/docs/Web/SVG/Guides/SVG_as_an_image . 그래도 팩토리는 `<script`, `on*=`, `href="http` 가 들어간 SVG를 거부해 이중으로 막는다.
- **총량 상한**: 텍스트 필드 합계 ≤ 2 KB, 이미지 ≤ 16 KB, 전체 ≤ 18 KB로 둔다. 상한이 있어야 스토리지 남용(체인 비대)과 RPC 응답 폭주를 막고, 작은 메타데이터가 수수료에서 불리해지지 않는다.
- 기존 `logoURI ≤ 256`은 유지해도 되지만 그 안에 data URI를 넣기에는 너무 짧다. `image`(바이트)와 `uri`(외부, 선택)로 나누는 편이 명확하다.

### 3.3 불변성과 공개 항목

- setter를 아예 두지 않는다. owner, admin, `updateImage`류 함수도 없다(Clanker와 Zora는 가변이므로 반대로 간다). pump.fun, Sui `delete_metadata_cap`, Metaplex `is_mutable=false`와 같은 결과를 "처음부터 불가능"하게 코드로 보장한다.
- ERC-7572 `ContractURIUpdated()`는 원래 "업데이트 시"에 내는 이벤트다. 배포 트랜잭션에서 한 번만 내서 인덱서가 캐시를 채우게 하고, 이후 다시 발생하지 않는다는 점을 문서화한다.
- 공개(view로 체인에 노출):
  - 총공급 고정과 mint 함수 부재(`supplyState = fixed`)
  - freeze, blacklist, pause 부재(Sui `RegulatedState::Unregulated`에 해당)
  - 메타데이터 불변(`metadataUpdateAuthority = 0x0`)
  - `creator` 주소, 생성 블록/시각, 팩토리 주소
  - 초기 분배(creator 보유량, 풀 투입량). Zora의 creator allocation 공개 방식 참고
- 인증은 "팩토리 출신" 여부로만 판단한다. 지갑은 `factory.isFactoryToken(addr)`로 확인한다. 이는 Jupiter 배지처럼 **보증이 아니라 구조적 사실**이라고 UI에 명시한다.

### 3.4 UI 안전 규칙

- **텍스트**: name, symbol, description은 textContent로만 렌더한다. HTML과 Markdown을 해석하지 않는다.
  - 팩토리 단계에서 제어문자, 양방향(bidi) 제어문자(U+202A-202E, U+2066-2069), 제로폭 문자를 거부한다.
  - symbol은 ASCII 대문자와 숫자만 허용해 동형문자(homoglyph) 사칭을 막는다.
- **링크**: `https://`만 허용한다. `javascript:`, `data:`, `http:`는 거부한다.
  - 소셜 키별로 호스트 allowlist를 둔다: twitter는 `x.com`/`twitter.com`, telegram은 `t.me`, discord는 `discord.gg`/`discord.com`.
  - website는 임의 호스트를 허용하되 클릭 전에 도메인을 보여주고 경고한다.
  - 팩토리 컨트랙트에서 접두사를 검사하고, UI에서 URL을 다시 파싱해 한 번 더 검증한다.
- **이미지**: data URI의 MIME이 PNG/WebP/SVG인지, 실제 매직 바이트가 선언과 일치하는지, 디코딩 크기 ≤ 16 KB이고 픽셀 ≤ 512x512인지 확인한다. `<img>`로만 표시하고 iframe, object, CSS background는 쓰지 않는다.
- **동명 토큰 경고**: 티커는 유일하지 않다(HIP-1 명시). 같은 symbol이 여럿이면 주소 앞뒤 4자와 생성 시각을 함께 보여준다.

### 3.5 상호운용성

- **EVM 지갑과 마켓**:
  - `contractURI()`(ERC-7572)는 `data:application/json;base64,...`를 반환한다. 표준은 `;utf8,` 예시를 들지만, base64를 쓰면 JSON 안의 `#`, `%` 같은 인코딩 문제를 피할 수 있다 [추정].
  - 같은 JSON을 `tokenURI()`(ERC-1046, Final)로도 반환한다.
  - JSON 키는 Metaplex와 7572의 합집합으로 쓴다: `name`, `symbol`, `decimals`, `description`, `image`, `external_url`, `external_link`(동일 값), `extensions{website,twitter,telegram,discord}`, `additional_metadata`, `aether{creator, supply_fixed:true, mint_authority:null, freeze_authority:null, metadata_mutable:false, factory}`.
- **Solana 계열 도구**: 같은 JSON이 Metaplex Fungible 스키마(name/symbol/description/image)를 포함하므로 파서를 그대로 재사용할 수 있다. `additional_metadata`는 Token-2022와 같은 `[[key,value],...]` 배열 형태로 둔다.
- **토큰 리스트**(Uniswap 스키마 기반의 Monad, Berachain 방식):
  - 체인 데이터로 누구나 `aether.tokenlist.json`을 재생성할 수 있게 한다(서버 불필요, 정적 파일).
  - 매핑: `logoURI`에는 data URI를 넣는다(스키마는 format uri라 형식상 유효). `tags`는 `["fixed_supply","immutable"]`.
  - `extensions`는 문자열이 42자로 제한되므로 URL을 넣지 말고 `creator`(42자 주소), `supplyFixed:true` 같은 짧은 값만 넣는다.
  - 목록 크기가 커지는 점은 로고 16 KB 상한으로 억제한다.
  - symbol 10자와 name 32자는 스키마 제한(20/60) 안에 든다.
- **인덱서**: `ContractURIUpdated`(배포 시 1회)와 팩토리의 `TokenCreated(token, creator, metadataHash)` 이벤트를 보고 캐시한다. 불변이므로 캐시가 무효화될 일이 없다.

---

## 요약 (5줄)

1. 필드는 Metaplex와 Token-2022 이름을 그대로 쓴다: `name ≤32 / symbol [A-Z0-9]≤10 / description ≤280 / image / external_url / extensions(website·twitter·telegram·discord) / additionalMetadata(KV ≤8쌍)`.
2. 저장은 모두 온체인이다. 텍스트는 스토리지에, 이미지는 SSTORE2 코드 청크(≤16 KB, PNG/WebP/정제 SVG)에 두고, JSON은 view에서 조립해 `contractURI()`와 `tokenURI()`가 `data:application/json;base64`로 반환한다.
3. setter가 없어 배포 즉시 불변이다. `supplyState=fixed`, mint·freeze·metadata authority=0x0, `creator`, `createdAt`, 초기 분배, `metadataHash`를 view로 공개한다(Sui enum 방식과 Solana authority 방식을 합침).
4. UI는 텍스트만 렌더하고 bidi와 제로폭 문자를 거부한다. 링크는 https와 소셜 호스트 allowlist로 제한하고, 이미지는 MIME·매직바이트·크기·픽셀을 검증해 `<img>`로만 표시한다. 같은 티커가 있으면 주소를 함께 보여준다.
5. 상호운용: EVM 지갑과 마켓은 ERC-7572/1046으로, Solana 도구는 Metaplex 호환 JSON으로 읽는다. 토큰 리스트는 체인에서 정적으로 재생성하되, extensions에는 42자 이하 값만 넣는다.
