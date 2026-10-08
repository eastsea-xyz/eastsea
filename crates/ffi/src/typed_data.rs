//! Bounded EIP-712 v4 encoding shared by the native and browser wallets.
//! The returned signing message is the account's ERC-1271 Contents wrapper,
//! never the dApp digest directly (EastSeaAccount.signatureMessage).

use aether_crypto::{address_of, verify, PublicKey};
use aether_execution::AETHER_ACCOUNT;
use aether_types::SignerScheme;
use alloy_primitives::{hex, keccak256, Address, B256, U256};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const MAX_JSON_BYTES: usize = 65_536;
const MAX_JSON_DEPTH: usize = 48;
const MAX_SIGNED_DEPTH: usize = 16;
const MAX_JSON_NODES: usize = 4_096;
const MAX_SIGNED_NODES: usize = 2_048;
const MAX_ARRAY_ITEMS: usize = 256;
const MAX_STRING_BYTES: usize = 8_192;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Debug)]
pub struct Prepared {
    pub chain_id: u64,
    pub account: Address,
    pub digest: B256,
    pub signing_message: Vec<u8>,
    pub typed_data: Value,
}

#[derive(Clone)]
enum Kind {
    Address,
    Bool,
    String,
    Bytes(Option<usize>),
    Uint(usize),
    Int(usize),
    Struct(String),
}

struct FieldType {
    kind: Kind,
    // Solidity writes the outermost dimension last: Item[2][] is a list
    // of two-item arrays.
    arrays: Vec<Option<usize>>,
}

struct Field {
    name: String,
    declaration: String,
    ty: FieldType,
}

type Types = BTreeMap<String, Vec<Field>>;

pub fn prepare(input: &str, expected_chain: u64, account: Address) -> Result<Prepared, String> {
    if expected_chain == 7_780 {
        return Err("typed-message signing needs the new account runtime; legacy chain 7780 cannot provide it".into());
    }
    let typed_data = document(input)?;
    let root = typed_data
        .as_object()
        .ok_or("typed data must be an object")?;
    if root.len() != 4
        || ["types", "primaryType", "domain", "message"]
            .iter()
            .any(|name| !root.contains_key(*name))
    {
        return Err("typed data must contain only types, primaryType, domain, and message".into());
    }
    let types = parse_types(&typed_data["types"])?;
    let primary = typed_data["primaryType"]
        .as_str()
        .ok_or("primaryType must be a type name")?;
    if primary == "EIP712Domain" || !types.contains_key(primary) {
        return Err("primaryType must name a declared message type".into());
    }
    let domain_fields = types
        .get("EIP712Domain")
        .ok_or("EIP712Domain must be declared")?;
    let mut declared_chain = false;
    for field in domain_fields {
        let expected_type = match field.name.as_str() {
            "name" | "version" => "string",
            "chainId" => {
                declared_chain = true;
                "uint256"
            }
            "verifyingContract" => "address",
            "salt" => "bytes32",
            _ => return Err("unsupported EIP712Domain field".into()),
        };
        if field.declaration != expected_type {
            return Err(format!(
                "domain {} must have type {expected_type}",
                field.name
            ));
        }
    }
    if !declared_chain {
        return Err("domain.chainId must be declared as uint256".into());
    }
    let domain = typed_data["domain"]
        .as_object()
        .ok_or("domain must be an object")?;
    let chain_word = integer(
        domain.get("chainId").ok_or("domain.chainId is required")?,
        256,
        false,
    )?;
    if chain_word != U256::from(expected_chain).to_be_bytes::<32>() {
        return Err(format!(
            "domain.chainId does not match wallet chain {expected_chain}"
        ));
    }
    let hashes = types
        .keys()
        .map(|name| Ok((name.clone(), keccak256(encode_type(name, &types)?))))
        .collect::<Result<_, String>>()?;
    let mut encoder = Encoder {
        types: &types,
        hashes,
        remaining: MAX_SIGNED_NODES,
    };
    let separator = encoder.struct_hash("EIP712Domain", &typed_data["domain"], 0)?;
    let message = encoder.struct_hash(primary, &typed_data["message"], 0)?;
    let digest = keccak256([&[0x19, 0x01][..], separator.as_slice(), message.as_slice()].concat());
    Ok(Prepared {
        chain_id: expected_chain,
        account,
        digest,
        signing_message: account_message(expected_chain, account, digest),
        typed_data,
    })
}

pub fn attach(
    input: &str,
    chain: u64,
    account: Address,
    signature: &[u8],
    public_key: &[u8],
) -> Result<String, String> {
    let key = p256::ecdsa::VerifyingKey::from_sec1_bytes(public_key)
        .map_err(|_| "not a P-256 public key")?;
    let pk = PublicKey {
        scheme: SignerScheme::P256,
        bytes: key.to_sec1_point(true).as_bytes().to_vec(),
    };
    if address_of(&pk).map_err(|e| e.to_string())? != account {
        return Err("typed-message signer does not own this wallet account".into());
    }
    let prepared = prepare(input, chain, account)?;
    let signature = p256::ecdsa::Signature::from_slice(signature)
        .map_err(|_| "signature must be 64-byte r‖s")?
        .normalize_s();
    verify(&pk, &prepared.signing_message, &signature.to_bytes())
        .map_err(|_| "signature does not match the approved message, account, and chain")?;
    let point = key.to_sec1_point(false);
    Ok(hex::encode_prefixed(
        [signature.to_bytes().as_slice(), &point.as_bytes()[1..]].concat(),
    ))
}

pub fn supports(account_code: &str, implementation_code: &str) -> bool {
    let expected = [&[0xef, 0x01, 0x00][..], AETHER_ACCOUNT.as_slice()].concat();
    code_equals(account_code, &expected) && runtime_is_v2(implementation_code)
}

fn code_equals(code: &str, expected: &[u8]) -> bool {
    code.strip_prefix("0x")
        .filter(|raw| raw.len() == expected.len() * 2)
        .and_then(|raw| hex::decode(raw).ok())
        .is_some_and(|raw| raw.as_slice() == expected)
}

pub fn runtime_is_v2(code: &str) -> bool {
    code_equals(code, &aether_execution::aether_account_code_v2())
}

fn account_message(chain: u64, account: Address, contents: B256) -> Vec<u8> {
    let mut domain = Vec::with_capacity(160);
    for word in [
        keccak256(
            "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
        )
        .0,
        keccak256("EastSeaAccount").0,
        keccak256("2").0,
        U256::from(chain).to_be_bytes(),
        address_word(account),
    ] {
        domain.extend_from_slice(&word);
    }
    let contents_hash = keccak256(
        [
            keccak256("Contents(bytes32 contents)").as_slice(),
            contents.as_slice(),
        ]
        .concat(),
    );
    [
        &[0x19, 0x01][..],
        keccak256(domain).as_slice(),
        contents_hash.as_slice(),
    ]
    .concat()
}

fn address_word(address: Address) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[12..].copy_from_slice(address.as_slice());
    word
}

fn identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    name.len() <= 64
        && bytes
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        && bytes.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

fn primitive(name: &str) -> Result<Option<Kind>, String> {
    let kind = match name {
        "address" => Kind::Address,
        "bool" => Kind::Bool,
        "string" => Kind::String,
        "bytes" => Kind::Bytes(None),
        "uint" | "int" => return Err("integer types must specify their bit width".into()),
        _ => {
            for prefix in ["bytes", "uint", "int"] {
                if let Some(suffix) = name
                    .strip_prefix(prefix)
                    .filter(|s| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit()))
                {
                    let width = suffix.parse::<usize>().map_err(|_| "invalid type width")?;
                    if suffix != width.to_string() {
                        return Err("noncanonical type width".into());
                    }
                    return match prefix {
                        "bytes" if (1..=32).contains(&width) => Ok(Some(Kind::Bytes(Some(width)))),
                        "uint" | "int" if (8..=256).contains(&width) && width % 8 == 0 => {
                            Ok(Some(if prefix == "uint" {
                                Kind::Uint(width)
                            } else {
                                Kind::Int(width)
                            }))
                        }
                        _ => Err("invalid type width".into()),
                    };
                }
            }
            return Ok(None);
        }
    };
    Ok(Some(kind))
}

fn field_type(declaration: &str) -> Result<FieldType, String> {
    if declaration.len() > 128 {
        return Err("field type is too long".into());
    }
    let start = declaration.find('[').unwrap_or(declaration.len());
    let base = &declaration[..start];
    if !identifier(base) {
        return Err("invalid field type".into());
    }
    let kind = primitive(base)?.unwrap_or_else(|| Kind::Struct(base.to_owned()));
    let mut tail = &declaration[start..];
    let mut arrays = Vec::new();
    while !tail.is_empty() {
        let raw = tail.strip_prefix('[').ok_or("invalid array type")?;
        let close = raw.find(']').ok_or("invalid array type")?;
        let count = &raw[..close];
        let dimension = if count.is_empty() {
            None
        } else {
            let size = count
                .parse::<usize>()
                .map_err(|_| "invalid fixed array length")?;
            if size == 0 || size > MAX_ARRAY_ITEMS || count != size.to_string() {
                return Err("fixed array length is outside wallet limits".into());
            }
            Some(size)
        };
        arrays.push(dimension);
        if arrays.len() > 8 {
            return Err("too many array dimensions".into());
        }
        tail = &raw[close + 1..];
    }
    Ok(FieldType { kind, arrays })
}

fn parse_types(value: &Value) -> Result<Types, String> {
    let map = value.as_object().ok_or("types must be an object")?;
    if map.is_empty() || map.len() > 64 {
        return Err("too many or missing typed-data types".into());
    }
    let mut types = Types::new();
    let mut total = 0;
    for (name, value) in map {
        if !identifier(name) || primitive(name)?.is_some() {
            return Err("invalid struct type name".into());
        }
        let fields = value
            .as_array()
            .ok_or("type declaration must list fields")?;
        total += fields.len();
        if fields.len() > 32 || total > 512 {
            return Err("too many typed-data fields".into());
        }
        let mut seen = BTreeSet::new();
        let fields = fields
            .iter()
            .map(|value| {
                let object = value
                    .as_object()
                    .ok_or("field declaration must be an object")?;
                if object.len() != 2 {
                    return Err("field declarations contain only name and type".into());
                }
                let name = object
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or("field name is required")?;
                let declaration = object
                    .get("type")
                    .and_then(Value::as_str)
                    .ok_or("field type is required")?;
                if !identifier(name) || !seen.insert(name) {
                    return Err("invalid or duplicate field name".into());
                }
                Ok(Field {
                    name: name.to_owned(),
                    declaration: declaration.to_owned(),
                    ty: field_type(declaration)?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        types.insert(name.clone(), fields);
    }
    for fields in types.values() {
        for field in fields {
            if let Kind::Struct(name) = &field.ty.kind {
                if !types.contains_key(name) {
                    return Err(format!("undeclared type {name}"));
                }
            }
        }
    }
    Ok(types)
}

fn encode_type(primary: &str, types: &Types) -> Result<String, String> {
    let mut seen = BTreeSet::new();
    let mut pending = vec![primary.to_owned()];
    while let Some(name) = pending.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        for field in types.get(&name).ok_or("undeclared struct type")? {
            if let Kind::Struct(child) = &field.ty.kind {
                pending.push(child.clone());
            }
        }
    }
    seen.remove(primary);
    let mut encoded = String::new();
    for name in std::iter::once(primary).chain(seen.iter().map(String::as_str)) {
        encoded.push_str(name);
        encoded.push('(');
        for (index, field) in types[name].iter().enumerate() {
            if index != 0 {
                encoded.push(',');
            }
            encoded.push_str(&field.declaration);
            encoded.push(' ');
            encoded.push_str(&field.name);
        }
        encoded.push(')');
    }
    Ok(encoded)
}

struct Encoder<'a> {
    types: &'a Types,
    hashes: BTreeMap<String, B256>,
    remaining: usize,
}

impl Encoder<'_> {
    fn bound(&mut self, depth: usize) -> Result<(), String> {
        if depth > MAX_SIGNED_DEPTH || self.remaining == 0 {
            return Err("typed message is too deeply nested or too large".into());
        }
        self.remaining -= 1;
        Ok(())
    }

    fn struct_hash(&mut self, name: &str, value: &Value, depth: usize) -> Result<B256, String> {
        self.bound(depth)?;
        let fields = self.types.get(name).ok_or("undeclared struct type")?;
        let object = value.as_object().ok_or("struct value must be an object")?;
        if object.len() != fields.len() {
            return Err(format!("{name} contains missing or undeclared fields"));
        }
        let mut data = Vec::with_capacity((fields.len() + 1) * 32);
        data.extend_from_slice(self.hashes[name].as_slice());
        for field in fields {
            let value = object
                .get(&field.name)
                .ok_or_else(|| format!("missing {name}.{}", field.name))?;
            data.extend_from_slice(&self.encode(
                &field.ty,
                value,
                field.ty.arrays.len(),
                depth + 1,
            )?);
        }
        Ok(keccak256(data))
    }

    fn encode(
        &mut self,
        ty: &FieldType,
        value: &Value,
        dimensions: usize,
        depth: usize,
    ) -> Result<[u8; 32], String> {
        self.bound(depth)?;
        if dimensions != 0 {
            let values = value
                .as_array()
                .ok_or("array field must contain an array")?;
            if values.len() > MAX_ARRAY_ITEMS
                || ty.arrays[dimensions - 1].is_some_and(|n| n != values.len())
            {
                return Err("array length does not match its type or exceeds wallet limits".into());
            }
            let mut encoded = Vec::with_capacity(values.len() * 32);
            for value in values {
                encoded.extend_from_slice(&self.encode(ty, value, dimensions - 1, depth + 1)?);
            }
            return Ok(keccak256(encoded).0);
        }
        match &ty.kind {
            Kind::Address => {
                let raw = value.as_str().ok_or("address must be a string")?;
                if raw.len() != 42 || !raw.starts_with("0x") {
                    return Err("address must contain exactly 20 hex bytes".into());
                }
                Ok(address_word(raw.parse().map_err(|_| "invalid address")?))
            }
            Kind::Bool => Ok(U256::from(
                value.as_bool().ok_or("bool must be true or false")? as u64
            )
            .to_be_bytes()),
            Kind::String => Ok(keccak256(
                value
                    .as_str()
                    .ok_or("string field must be a string")?
                    .as_bytes(),
            )
            .0),
            Kind::Bytes(size) => {
                let raw = value
                    .as_str()
                    .and_then(|s| s.strip_prefix("0x"))
                    .ok_or("bytes must be a 0x-prefixed hex string")?;
                let bytes = hex::decode(raw).map_err(|_| "invalid bytes hex")?;
                if let Some(size) = size {
                    if bytes.len() != *size {
                        return Err("bytes value does not match its declared width".into());
                    }
                    let mut word = [0u8; 32];
                    word[..*size].copy_from_slice(&bytes);
                    Ok(word)
                } else {
                    Ok(keccak256(bytes).0)
                }
            }
            Kind::Uint(bits) => integer(value, *bits, false),
            Kind::Int(bits) => integer(value, *bits, true),
            Kind::Struct(name) => Ok(self.struct_hash(name, value, depth + 1)?.0),
        }
    }
}

fn integer(value: &Value, bits: usize, signed: bool) -> Result<[u8; 32], String> {
    let raw = match value {
        Value::String(raw) => raw.clone(),
        Value::Number(number) => {
            let safe = number.as_u64().is_some_and(|n| n <= MAX_SAFE_INTEGER)
                || number
                    .as_i64()
                    .is_some_and(|n| n.unsigned_abs() <= MAX_SAFE_INTEGER);
            if !safe {
                return Err(
                    "integer JSON numbers must be exact; use a decimal string for large amounts"
                        .into(),
                );
            }
            number.to_string()
        }
        _ => return Err("integer must be an exact number or decimal/hex string".into()),
    };
    if raw.len() > 80 {
        return Err("integer is too large".into());
    }
    let (negative, digits) = match raw.strip_prefix('-') {
        Some(digits) => (true, digits),
        None => (false, raw.as_str()),
    };
    if negative && !signed {
        return Err("unsigned integer cannot be negative".into());
    }
    let (digits, radix) = match digits.strip_prefix("0x") {
        Some(digits) => (digits, 16),
        None => (digits, 10),
    };
    if digits.is_empty()
        || !digits.bytes().all(|c| {
            if radix == 16 {
                c.is_ascii_hexdigit()
            } else {
                c.is_ascii_digit()
            }
        })
    {
        return Err("invalid integer value".into());
    }
    let magnitude = U256::from_str_radix(digits, radix).map_err(|_| "integer exceeds 256 bits")?;
    if signed {
        let limit = U256::from(1u64) << (bits - 1);
        if if negative {
            magnitude > limit
        } else {
            magnitude >= limit
        } {
            return Err("signed integer exceeds its declared width".into());
        }
    } else if bits < 256 && magnitude >= (U256::from(1u64) << bits) {
        return Err("unsigned integer exceeds its declared width".into());
    }
    Ok(if negative && magnitude != U256::ZERO {
        (!magnitude).wrapping_add(U256::from(1u64))
    } else {
        magnitude
    }
    .to_be_bytes())
}

fn document(input: &str) -> Result<Value, String> {
    if input.len() > MAX_JSON_BYTES {
        return Err("typed data exceeds 64 KiB".into());
    }
    reject_duplicate_keys(input)?;
    let value = serde_json::from_str(input).map_err(|_| "typed data must be valid JSON")?;
    let mut remaining = MAX_JSON_NODES;
    shape_limits(&value, 0, &mut remaining)?;
    Ok(value)
}

fn shape_limits(value: &Value, depth: usize, remaining: &mut usize) -> Result<(), String> {
    if depth > MAX_JSON_DEPTH || *remaining == 0 {
        return Err("typed JSON is too deeply nested or too large".into());
    }
    *remaining -= 1;
    match value {
        Value::Array(values) => {
            if values.len() > MAX_ARRAY_ITEMS {
                return Err("array exceeds wallet limits".into());
            }
            for value in values {
                shape_limits(value, depth + 1, remaining)?;
            }
        }
        Value::Object(values) => {
            if values.len() > 64 {
                return Err("object exceeds wallet field limits".into());
            }
            for value in values.values() {
                shape_limits(value, depth + 1, remaining)?;
            }
        }
        Value::String(value) if value.len() > MAX_STRING_BYTES => {
            return Err("string exceeds wallet limits".into())
        }
        _ => {}
    }
    Ok(())
}

// serde_json::Value keeps the last duplicate object key. Check the raw JSON
// first so its displayed and signed contents cannot disagree across parsers.
fn reject_duplicate_keys(input: &str) -> Result<(), String> {
    enum Container {
        Object {
            keys: BTreeSet<String>,
            expect_key: bool,
        },
        Array,
    }
    let bytes = input.as_bytes();
    let mut stack = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'{' => stack.push(Container::Object {
                keys: BTreeSet::new(),
                expect_key: true,
            }),
            b'[' => stack.push(Container::Array),
            b'}' | b']' => {
                stack.pop();
            }
            b',' => {
                if let Some(Container::Object { expect_key, .. }) = stack.last_mut() {
                    *expect_key = true;
                }
            }
            b'"' => {
                let start = index;
                index += 1;
                while index < bytes.len() && bytes[index] != b'"' {
                    if bytes[index] == b'\\' {
                        index += 1;
                    }
                    index += 1;
                }
                if index >= bytes.len() {
                    return Err("unterminated JSON string".into());
                }
                if let Some(Container::Object { keys, expect_key }) = stack.last_mut() {
                    if *expect_key {
                        let name: String = serde_json::from_str(&input[start..=index])
                            .map_err(|_| "invalid JSON field name")?;
                        if !keys.insert(name) {
                            return Err("duplicate JSON field name".into());
                        }
                        *expect_key = false;
                    }
                }
            }
            _ => {}
        }
        if stack.len() > MAX_JSON_DEPTH {
            return Err("typed JSON is too deeply nested".into());
        }
        index += 1;
    }
    Ok(())
}
