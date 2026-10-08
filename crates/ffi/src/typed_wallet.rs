//! Native dApp signing and account migration adapters. Keys stay in Swift.
use crate::{
    call, expected_chain, p256_key, prepare, quote_for_call, typed_data, validated_state_price,
    PreparedTx, TransferQuote, WalletError, NETWORK_GENERATION, R,
};
use aether_crypto::address_of;
use aether_execution::{EvmCall, AETHER_ACCOUNT};
use aether_types::{Address, Bytes, U256};
use serde_json::json;
use serde_json::Value;
use std::sync::atomic::Ordering;

#[derive(uniffi::Record)]
pub struct PreparedTypedMessage {
    pub chain_id: u64,
    pub account: String,
    /// The 66-byte account/chain Contents message; CryptoKit hashes it with SHA-256.
    pub signing_message: Vec<u8>,
    /// The original dApp's EIP-712 digest for ERC-1271 verification.
    pub digest_hex: String,
    /// Canonical validated JSON: this is the field view the owner confirms.
    pub typed_data_json: String,
}

#[derive(uniffi::Record)]
pub struct DappTransactionSimulation {
    /// The exact gas limit used by every read and later transaction preparation.
    pub gas_limit: u64,
    /// The bounded node result, consumed by the native readable-field parser.
    pub result_json: String,
}

fn code_at(address: Address) -> R<String> {
    let code = call("eth_getCode", json!([address.to_checksum(None), "latest"]))?;
    code.as_str()
        .map(str::to_owned)
        .ok_or_else(|| WalletError::Verification("account bytecode reply is missing".into()))
}

fn supports_at(address: Address, chain: u64) -> R<bool> {
    if chain == 7_780 {
        return Ok(false);
    }
    let account = code_at(address)?;
    let implementation = code_at(aether_execution::AETHER_ACCOUNT)?;
    Ok(typed_data::supports(&account, &implementation))
}

fn unchanged_network(generation: u64) -> R<()> {
    if generation != NETWORK_GENERATION.load(Ordering::SeqCst) {
        return Err(WalletError::Verification(
            "network changed during message preparation".into(),
        ));
    }
    Ok(())
}

/// True only for an account actually delegated to the canonical, deployed
/// EastSeaAccount v2 runtime. Merely running a new-genesis node is insufficient.
#[uniffi::export]
pub fn account_signing_support(address: String) -> R<bool> {
    let generation = NETWORK_GENERATION.load(Ordering::SeqCst);
    let address = address
        .parse()
        .map_err(|_| WalletError::Invalid("account address".into()))?;
    let chain = expected_chain(&call("aether_status", json!([]))?)?;
    let supported = supports_at(address, chain)?;
    unchanged_network(generation)?;
    Ok(supported)
}

#[uniffi::export]
pub fn prepare_typed_message(
    p256_public_key: Vec<u8>,
    typed_data_json: String,
) -> R<PreparedTypedMessage> {
    let generation = NETWORK_GENERATION.load(Ordering::SeqCst);
    let account = address_of(&p256_key(&p256_public_key)?)
        .map_err(|e| WalletError::Invalid(e.to_string()))?;
    let chain = expected_chain(&call("aether_status", json!([]))?)?;
    let prepared =
        typed_data::prepare(&typed_data_json, chain, account).map_err(WalletError::Invalid)?;
    if !supports_at(account, chain)? {
        return Err(WalletError::Invalid("this account needs delegation to the deployed v2 account runtime before signing typed messages".into()));
    }
    unchanged_network(generation)?;
    Ok(PreparedTypedMessage {
        chain_id: prepared.chain_id,
        account: prepared.account.to_checksum(None),
        signing_message: prepared.signing_message,
        digest_hex: prepared.digest.to_string(),
        typed_data_json: prepared.typed_data.to_string(),
    })
}

/// Verify the exact approved contents again and return r || low-s || x || y
/// for EastSeaAccount.isValidSignature. No transaction or network write occurs.
#[uniffi::export]
pub fn attach_typed_signature(
    typed_data_json: String,
    expected_chain: u64,
    account: String,
    signature: Vec<u8>,
    p256_public_key: Vec<u8>,
) -> R<String> {
    let generation = NETWORK_GENERATION.load(Ordering::SeqCst);
    let chain = crate::expected_chain(&call("aether_status", json!([]))?)?;
    if chain != expected_chain {
        return Err(WalletError::Verification(
            "the approved signing chain changed".into(),
        ));
    }
    let account = account
        .parse()
        .map_err(|_| WalletError::Invalid("account address".into()))?;
    if !supports_at(account, chain)? {
        return Err(WalletError::Invalid(
            "the account no longer uses the deployed v2 signing runtime".into(),
        ));
    }
    let signature = typed_data::attach(
        &typed_data_json,
        chain,
        account,
        &signature,
        &p256_public_key,
    )
    .map_err(WalletError::Invalid)?;
    unchanged_network(generation)?;
    Ok(signature)
}

/// Re-delegate this same account to the fixed canonical v2 implementation.
/// The recipient, implementation, and calldata cannot be supplied by a dApp.
#[uniffi::export]
pub fn prepare_account_redelegation(p256_public_key: Vec<u8>) -> R<PreparedTx> {
    let generation = NETWORK_GENERATION.load(Ordering::SeqCst);
    let account = address_of(&p256_key(&p256_public_key)?)
        .map_err(|e| WalletError::Invalid(e.to_string()))?;
    let chain = expected_chain(&call("aether_status", json!([]))?)?;
    let account_code = code_at(account)?;
    let implementation = code_at(aether_execution::AETHER_ACCOUNT)?;
    let call = redelegation_call(account, chain, &account_code, &implementation)
        .map_err(WalletError::Invalid)?;
    let prepared = prepare(&p256_public_key, None, |_| Ok(call))?;
    unchanged_network(generation)?;
    Ok(prepared)
}

pub(crate) fn redelegation_call(
    account: Address,
    chain: u64,
    account_code: &str,
    implementation_code: &str,
) -> Result<EvmCall, String> {
    if chain == 7_780 || !typed_data::runtime_is_v2(implementation_code) {
        return Err("this network does not deploy the pinned v2 account runtime".into());
    }
    if account_code != "0x" && !typed_data::supports(account_code, implementation_code) {
        return Err(
            "account migration cannot preserve settings from a different implementation".into(),
        );
    }
    Ok(EvmCall {
        to: Some(account),
        value: U256::ZERO,
        input: Bytes::new(),
        gas_limit: 100_000,
        delegate: Some(AETHER_ACCOUNT),
    })
}

fn dapp_call(to: &str, value_wei: &str, data_hex: &str, gas_limit: u64) -> R<EvmCall> {
    if gas_limit > 10_000_000 {
        return Err(WalletError::Invalid(
            "requested gas exceeds the wallet limit".into(),
        ));
    }
    if data_hex.len() > 2 + 2 * 65_536 {
        return Err(WalletError::Invalid(
            "call data exceeds the wallet limit".into(),
        ));
    }
    let to = if to.is_empty() {
        None
    } else {
        Some(
            to.parse()
                .map_err(|_| WalletError::Invalid("contract address".into()))?,
        )
    };
    let value = if value_wei.is_empty() {
        U256::ZERO
    } else {
        value_wei
            .parse()
            .map_err(|_| WalletError::Invalid("value".into()))?
    };
    let input = alloy_primitives::hex::decode(data_hex.trim_start_matches("0x"))
        .map_err(|_| WalletError::Invalid("call data is not hex".into()))?;
    let gas_limit = match gas_limit {
        0 if input.is_empty() && to.is_some() => aether_execution::tx::PLAIN_TRANSFER_GAS,
        0 => 3_000_000,
        gas => gas,
    };
    Ok(EvmCall {
        to,
        value,
        input: Bytes::from(input),
        gas_limit,
        delegate: None,
    })
}

/// The fee cap for precisely the transaction the dApp simulation describes.
#[uniffi::export]
pub fn dapp_transaction_quote(
    to: String,
    value_wei: String,
    data_hex: String,
    gas_limit: u64,
) -> R<TransferQuote> {
    let body = dapp_call(&to, &value_wei, &data_hex, gas_limit)?;
    let status = call("aether_status", json!([]))?;
    let price = validated_state_price(&status)?;
    Ok(quote_for_call(&status, None, price, &body))
}

/// Sign the same gas limit that was simulated, after rechecking any displayed
/// maximum fee. Excessive requested gas is refused rather than clamped.
#[uniffi::export]
pub fn prepare_dapp_transaction(
    p256_public_key: Vec<u8>,
    to: String,
    value_wei: String,
    data_hex: String,
    gas_limit: u64,
    shown_fee_wei: Option<String>,
) -> R<PreparedTx> {
    let body = dapp_call(&to, &value_wei, &data_hex, gas_limit)?;
    prepare(&p256_public_key, shown_fee_wei.as_deref(), |_| Ok(body))
}

fn simulation_error() -> WalletError {
    WalletError::Verification("the transaction simulation could not be read consistently".into())
}

fn simulation_hex(value: &str) -> R<Vec<u8>> {
    if value.len() > 2 * 1024 * 1024 {
        return Err(simulation_error());
    }
    let raw = value.strip_prefix("0x").ok_or_else(simulation_error)?;
    alloy_primitives::hex::decode(raw).map_err(|_| simulation_error())
}

fn simulation_quantity(value: &Value) -> R<u64> {
    let raw = value
        .as_str()
        .and_then(|raw| raw.strip_prefix("0x"))
        .filter(|raw| !raw.is_empty() && raw.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(simulation_error)?;
    u64::from_str_radix(raw, 16).map_err(|_| simulation_error())
}

/// Read-only dApp simulation through the wallet's existing local/peer transport.
/// Network or invalid-response errors can never become an overridable revert.
#[uniffi::export]
pub fn simulate_dapp_transaction(
    p256_public_key: Vec<u8>,
    to: String,
    value_wei: String,
    data_hex: String,
    gas_limit: u64,
) -> R<DappTransactionSimulation> {
    let generation = NETWORK_GENERATION.load(Ordering::SeqCst);
    let from = address_of(&p256_key(&p256_public_key)?)
        .map_err(|error| WalletError::Invalid(error.to_string()))?;
    let mut body = dapp_call(&to, &value_wei, &data_hex, gas_limit)?;
    let read = |method: &str, params| {
        unchanged_network(generation)?;
        call(method, params)
    };
    let chain = expected_chain(&read("aether_status", json!([]))?)?;
    if gas_limit == 0 && body.input.is_empty() {
        if let Some(to) = body.to {
            let code = code_at(to)?;
            body.gas_limit = if simulation_hex(&code)?.is_empty() {
                21_000
            } else {
                100_000
            };
        }
    }
    let mut request = json!({"from":from.to_checksum(None),"value":format!("{:#x}",body.value),
        "data":alloy_primitives::hex::encode_prefixed(&body.input),"gas":format!("0x{:x}",body.gas_limit)});
    if let Some(to) = body.to {
        request["to"] = json!(to.to_checksum(None));
    }
    let params = json!([request, "latest"]);
    let (call_success, call_output) = match read("eth_call", params.clone()) {
        Ok(Value::String(output)) => (true, simulation_hex(&output)?),
        Ok(_) => return Err(simulation_error()),
        Err(WalletError::Rejected(reason)) if reason.starts_with("execution reverted: ") => (
            false,
            simulation_hex(&reason["execution reverted: ".len()..])?,
        ),
        Err(error) => return Err(error),
    };
    let result = read("aether_simulateTransaction", params.clone())?;
    let result_json = result.to_string();
    if result_json.len() > 2 * 1024 * 1024 {
        return Err(simulation_error());
    }
    let success = result["success"].as_bool().ok_or_else(simulation_error)?;
    let used = simulation_quantity(&result["gasUsed"])?;
    let output = result["output"].as_str().ok_or_else(simulation_error)?;
    let failure = match result.get("failureReason") {
        Some(Value::Null) => None,
        Some(Value::String(reason)) if !reason.is_empty() && reason.len() <= 8_192 => {
            Some(reason.as_str())
        }
        _ => return Err(simulation_error()),
    };
    if success != call_success
        || used > body.gas_limit
        || simulation_hex(output)? != call_output
        || (success && failure.is_some())
    {
        return Err(simulation_error());
    }
    for (name, limit) in [
        ("nativeChanges", 2048),
        ("logs", 2048),
        ("tokenChanges", 128),
        ("measuredTokens", 129),
    ] {
        if result[name]
            .as_array()
            .is_none_or(|values| values.len() > limit)
        {
            return Err(simulation_error());
        }
    }
    if result["tokenCoverageComplete"].as_bool().is_none() {
        return Err(simulation_error());
    }
    match read("eth_estimateGas", params) {
        Ok(estimate) if success => {
            let estimate = simulation_quantity(&estimate)?;
            if estimate == 0 || estimate > body.gas_limit {
                return Err(simulation_error());
            }
        }
        Err(WalletError::Rejected(reason))
            if !success && reason == failure.unwrap_or("Execution reverted.") => {}
        Ok(_) => return Err(simulation_error()),
        Err(error) => return Err(error),
    }
    if expected_chain(&read("aether_status", json!([]))?)? != chain {
        return Err(simulation_error());
    }
    unchanged_network(generation)?;
    Ok(DappTransactionSimulation {
        gas_limit: body.gas_limit,
        result_json,
    })
}
