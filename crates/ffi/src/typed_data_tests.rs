//! Regression vectors shared by the native wallet and browser signing core.
use crate::typed_data;
use alloy_primitives::{keccak256, Address, B256, U256};
use p256::ecdsa::{signature::Signer as _, SigningKey};
use serde_json::{json, Value};

fn account() -> Address {
    Address::repeat_byte(0x42)
}

fn owner_address(public: &[u8]) -> Address {
    aether_crypto::address_of(&aether_crypto::PublicKey {
        scheme: aether_types::SignerScheme::P256,
        bytes: public.to_vec(),
    })
    .unwrap()
}

pub(crate) fn mail() -> Value {
    json!({
        "types": {
            "EIP712Domain": [
                {"name":"name","type":"string"},
                {"name":"version","type":"string"},
                {"name":"chainId","type":"uint256"},
                {"name":"verifyingContract","type":"address"}
            ],
            "Person": [{"name":"name","type":"string"},{"name":"wallet","type":"address"}],
            "Mail": [{"name":"from","type":"Person"},{"name":"to","type":"Person"},{"name":"contents","type":"string"}]
        },
        "primaryType":"Mail",
        "domain":{"name":"Ether Mail","version":"1","chainId":1,"verifyingContract":"0xCcCCccccCCCCcCCCCCCcCcCccCcCcCCCcCcccccccC"},
        "message":{
            "from":{"name":"Cow","wallet":"0xCD2a3d9F938E13CD947Ec05AbC7FE734Df8DD826"},
            "to":{"name":"Bob","wallet":"0xbBbBBBBbbBBBbbbBbbBbbbbBBbBbbbbBbBbbBBbB"},
            "contents":"Hello, Bob!"
        }
    })
}

fn prepared(value: &Value) -> typed_data::Prepared {
    typed_data::prepare(&value.to_string(), 1, account()).unwrap()
}

fn words(parts: &[[u8; 32]]) -> B256 {
    keccak256(parts.iter().flatten().copied().collect::<Vec<_>>())
}

fn address_word(address: Address) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[12..].copy_from_slice(address.as_slice());
    word
}

fn wrapper(chain: u64, account: Address, contents: B256) -> Vec<u8> {
    let domain = words(&[
        keccak256(
            "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
        )
        .0,
        keccak256("EastSeaAccount").0,
        keccak256("2").0,
        U256::from(chain).to_be_bytes(),
        address_word(account),
    ]);
    let message = words(&[keccak256("Contents(bytes32 contents)").0, contents.0]);
    [&[0x19, 0x01][..], domain.as_slice(), message.as_slice()].concat()
}

#[test]
fn eip712_reference_mail_hash_matches_the_published_vector() {
    // https://eips.ethereum.org/EIPS/eip-712, Example.js / Example.sol.
    let prepared = prepared(&mail());
    assert_eq!(
        prepared.digest.to_string(),
        "0xbe609aee343fb3c4b28e1df9e632fca64fcfaede20f02e86244efddf30957bd2"
    );
    assert_eq!(prepared.typed_data, mail());
}

#[test]
fn v4_arrays_signed_integers_and_fixed_bytes_use_solidity_words() {
    let data = json!({
        "types": {
            "EIP712Domain":[{"name":"chainId","type":"uint256"}],
            "Item":[{"name":"number","type":"int8"},{"name":"tag","type":"bytes2"}],
            "Basket":[{"name":"items","type":"Item[2]"},{"name":"counts","type":"uint256[]"},{"name":"enabled","type":"bool"},{"name":"matrix","type":"uint8[2][]"},{"name":"blob","type":"bytes"},{"name":"debt","type":"int256"}]
        },
        "primaryType":"Basket", "domain":{"chainId":"0x1"},
        "message":{"items":[{"number":-128,"tag":"0x1234"},{"number":127,"tag":"0xabcd"}],"counts":["0", "340282366920938463463374607431768211456"],"enabled":true,"matrix":[[1,2],[3,4]],"blob":"0x010203","debt":"-57896044618658097711785492504343953926634992332820282019728792003956564819968"}
    });
    let mut negative = [0xffu8; 32];
    negative[31] = 0x80;
    let mut first_tag = [0u8; 32];
    first_tag[..2].copy_from_slice(&[0x12, 0x34]);
    let mut second_tag = [0u8; 32];
    second_tag[..2].copy_from_slice(&[0xab, 0xcd]);
    let item_type = keccak256("Item(int8 number,bytes2 tag)");
    let first = words(&[item_type.0, negative, first_tag]);
    let second = words(&[item_type.0, U256::from(127).to_be_bytes(), second_tag]);
    let items = words(&[first.0, second.0]);
    let counts = words(&[
        U256::ZERO.to_be_bytes(),
        (U256::from(1) << 128usize).to_be_bytes(),
    ]);
    let matrix = words(&[
        words(&[U256::from(1).to_be_bytes(), U256::from(2).to_be_bytes()]).0,
        words(&[U256::from(3).to_be_bytes(), U256::from(4).to_be_bytes()]).0,
    ]);
    let message = words(&[
        keccak256(
            "Basket(Item[2] items,uint256[] counts,bool enabled,uint8[2][] matrix,bytes blob,int256 debt)Item(int8 number,bytes2 tag)",
        )
        .0,
        items.0,
        counts.0,
        U256::from(1).to_be_bytes(),
        matrix.0,
        keccak256([1u8, 2, 3]).0,
        (U256::from(1) << 255usize).to_be_bytes(),
    ]);
    let domain = words(&[
        keccak256("EIP712Domain(uint256 chainId)").0,
        U256::from(1).to_be_bytes(),
    ]);
    let expected = keccak256([&[0x19, 0x01][..], domain.as_slice(), message.as_slice()].concat());
    assert_eq!(prepared(&data).digest, expected);
    let mut wrong_length = data;
    wrong_length["message"]["items"]
        .as_array_mut()
        .unwrap()
        .pop();
    assert!(typed_data::prepare(&wrong_length.to_string(), 1, account()).is_err());
}

#[test]
fn domain_chain_must_be_declared_present_and_match_the_wallet() {
    prepared(&mail());
    let mut data = mail();
    data["domain"]["chainId"] = json!(2);
    assert!(typed_data::prepare(&data.to_string(), 1, account())
        .unwrap_err()
        .contains("chain"));
    data["domain"].as_object_mut().unwrap().remove("chainId");
    assert!(typed_data::prepare(&data.to_string(), 1, account()).is_err());
    data = mail();
    data["types"]["EIP712Domain"]
        .as_array_mut()
        .unwrap()
        .remove(2);
    assert!(typed_data::prepare(&data.to_string(), 1, account()).is_err());
    data = mail();
    data["domain"]["verifyingContract"] = json!("0x1234");
    assert!(typed_data::prepare(&data.to_string(), 1, account()).is_err());
    data = mail();
    data["domain"]
        .as_object_mut()
        .unwrap()
        .remove("verifyingContract");
    data["types"]["EIP712Domain"].as_array_mut().unwrap().pop();
    assert!(typed_data::prepare(&data.to_string(), 1, account()).is_ok());
}

#[test]
fn schema_rejects_hidden_fields_unknown_types_and_duplicate_names() {
    prepared(&mail());
    let mut data = mail();
    data["message"]["amount"] = json!("1000000");
    assert!(typed_data::prepare(&data.to_string(), 1, account()).is_err());
    data = mail();
    data["message"]["from"]
        .as_object_mut()
        .unwrap()
        .remove("wallet");
    assert!(typed_data::prepare(&data.to_string(), 1, account()).is_err());
    data = mail();
    data["types"]["Mail"][0]["type"] = json!("Unknown");
    assert!(typed_data::prepare(&data.to_string(), 1, account()).is_err());
    data = mail();
    data["types"]["Person"][1]["name"] = json!("name");
    assert!(typed_data::prepare(&data.to_string(), 1, account()).is_err());
    let duplicate = mail()
        .to_string()
        .replacen("\"chainId\":1", "\"chainId\":1,\"chainId\":2", 1);
    assert!(typed_data::prepare(&duplicate, 1, account()).is_err());
}

#[test]
fn integer_and_bytes_values_are_exact_and_never_coerced() {
    let data = json!({
        "types":{"EIP712Domain":[{"name":"chainId","type":"uint256"}],"Numbers":[{"name":"amount","type":"uint8"},{"name":"delta","type":"int8"},{"name":"tag","type":"bytes2"}]},
        "primaryType":"Numbers","domain":{"chainId":1},
        "message":{"amount":"255","delta":"-128","tag":"0x1234"}
    });
    prepared(&data);
    for value in [json!(256), json!(-1), json!(1.5), json!(true), json!("+1")] {
        let mut bad = data.clone();
        bad["message"]["amount"] = value;
        assert!(typed_data::prepare(&bad.to_string(), 1, account()).is_err());
    }
    for value in [json!(-129), json!(128)] {
        let mut bad = data.clone();
        bad["message"]["delta"] = value;
        assert!(typed_data::prepare(&bad.to_string(), 1, account()).is_err());
    }
    let mut bad = data.clone();
    bad["message"]["tag"] = json!("0x123456");
    assert!(typed_data::prepare(&bad.to_string(), 1, account()).is_err());
    bad = data;
    bad["types"]["Numbers"][0]["type"] = json!("uint256");
    bad["message"]["amount"] = json!(9007199254740992u64);
    assert!(typed_data::prepare(&bad.to_string(), 1, account()).is_err());
    bad["message"]["amount"] = json!("9007199254740992");
    assert!(typed_data::prepare(&bad.to_string(), 1, account()).is_ok());
}

#[test]
fn typed_input_size_array_count_and_recursive_values_are_bounded() {
    prepared(&mail());
    let mut data = mail();
    data["message"]["contents"] = json!("x".repeat(65536));
    assert!(typed_data::prepare(&data.to_string(), 1, account()).is_err());
    let mut nested = json!({"children":[]});
    let mut recursive = json!({
        "types":{"EIP712Domain":[{"name":"chainId","type":"uint256"}],"Nest":[{"name":"children","type":"Nest[]"}]},
        "primaryType":"Nest","domain":{"chainId":1},"message":nested
    });
    assert!(typed_data::prepare(&recursive.to_string(), 1, account()).is_ok());
    for _ in 0..20 {
        nested = json!({"children":[nested]});
    }
    recursive["message"] = nested;
    assert!(typed_data::prepare(&recursive.to_string(), 1, account()).is_err());
    recursive["message"] = json!({"children":vec![json!({"children":[]});257]});
    assert!(typed_data::prepare(&recursive.to_string(), 1, account()).is_err());
}

#[test]
fn contract_wrapper_binds_every_message_to_account_and_chain() {
    let prepared = prepared(&mail());
    // Independently encoded with Foundry cast abi-encode/keccak, not this parser.
    assert_eq!(
        alloy_primitives::hex::encode_prefixed(&prepared.signing_message),
        "0x1901fdc9b46b9628db823ef3c50e383ecf65cd929497e4ad1c3c60ccc5921fd81a8a53182595609e2fe5319d931462d5e7dc98e6d7ada21b5a43027d7fdaaaa139f2"
    );
    assert_eq!(
        prepared.signing_message,
        wrapper(1, account(), prepared.digest)
    );
    assert_eq!(prepared.signing_message.len(), 66);
    let other_account =
        typed_data::prepare(&mail().to_string(), 1, Address::repeat_byte(0x43)).unwrap();
    assert_ne!(other_account.signing_message, prepared.signing_message);
    let mut other_chain = mail();
    other_chain["domain"]["chainId"] = json!(2);
    assert_ne!(
        typed_data::prepare(&other_chain.to_string(), 2, account())
            .unwrap()
            .signing_message,
        prepared.signing_message
    );
}

#[test]
fn owner_signature_is_normalized_and_returns_the_erc1271_key_format() {
    let key = SigningKey::from_slice(&[7u8; 32]).unwrap();
    let public = key.verifying_key().to_sec1_point(true);
    let owner = owner_address(public.as_bytes());
    let data = mail().to_string();
    let prepared = typed_data::prepare(&data, 1, owner).unwrap();
    let signature: p256::ecdsa::Signature = key.sign(&prepared.signing_message);
    let (r, s) = signature.split_scalars();
    let other_s = p256::ecdsa::Signature::from_scalars(r, -*s).unwrap();
    let low =
        typed_data::attach(&data, 1, owner, &signature.to_bytes(), public.as_bytes()).unwrap();
    assert_eq!(
        low,
        typed_data::attach(&data, 1, owner, &other_s.to_bytes(), public.as_bytes()).unwrap()
    );
    let packed = alloy_primitives::hex::decode(&low).unwrap();
    assert_eq!(packed.len(), 128);
    assert_eq!(&packed[..64], signature.normalize_s().to_bytes().as_slice());
    let uncompressed = key.verifying_key().to_sec1_point(false);
    assert_eq!(&packed[64..], &uncompressed.as_bytes()[1..]);
}

#[test]
fn typed_signatures_refuse_other_keys_accounts_payloads_and_chains() {
    let key = SigningKey::from_slice(&[7u8; 32]).unwrap();
    let public = key.verifying_key().to_sec1_point(true);
    let owner = owner_address(public.as_bytes());
    let mut data = mail();
    let prepared = typed_data::prepare(&data.to_string(), 1, owner).unwrap();
    let signature: p256::ecdsa::Signature = key.sign(&prepared.signing_message);
    typed_data::attach(
        &data.to_string(),
        1,
        owner,
        &signature.to_bytes(),
        public.as_bytes(),
    )
    .unwrap();
    assert!(typed_data::attach(
        &data.to_string(),
        1,
        account(),
        &signature.to_bytes(),
        public.as_bytes()
    )
    .is_err());
    let other = SigningKey::from_slice(&[8u8; 32]).unwrap();
    assert!(typed_data::attach(
        &data.to_string(),
        1,
        owner,
        &signature.to_bytes(),
        other.verifying_key().to_sec1_point(true).as_bytes()
    )
    .is_err());
    data["message"]["contents"] = json!("Changed after approval");
    assert!(typed_data::attach(
        &data.to_string(),
        1,
        owner,
        &signature.to_bytes(),
        public.as_bytes()
    )
    .is_err());
    data = mail();
    data["domain"]["chainId"] = json!(2);
    assert!(typed_data::attach(
        &data.to_string(),
        2,
        owner,
        &signature.to_bytes(),
        public.as_bytes()
    )
    .is_err());
    assert!(
        typed_data::attach(&mail().to_string(), 1, owner, &[0; 63], public.as_bytes()).is_err()
    );
}

#[test]
fn typed_signing_requires_actual_canonical_v2_account_delegation() {
    let target = aether_execution::AETHER_ACCOUNT;
    let delegation = alloy_primitives::hex::encode_prefixed(
        [&[0xef, 0x01, 0x00][..], target.as_slice()].concat(),
    );
    let implementation =
        alloy_primitives::hex::encode_prefixed(aether_execution::aether_account_code_v2());
    assert!(typed_data::supports(&delegation, &implementation));
    assert!(!typed_data::supports("0x", &implementation));
    assert!(!typed_data::supports(
        &format!("{delegation}00"),
        &implementation
    ));
    assert!(!typed_data::supports(
        &delegation,
        &alloy_primitives::hex::encode_prefixed(aether_execution::aether_account_code())
    ));
    assert!(!typed_data::supports(&delegation, "0x6000"));
    assert!(!typed_data::supports(
        "0xef01000000000000000000000000000000000000000001",
        &implementation
    ));
}
