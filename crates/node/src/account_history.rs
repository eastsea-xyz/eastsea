//! Non-consensus, finalized account activity. One row per address and tx;
//! token movements in a swap stay together for the wallet to describe.

use aether_execution::{EvmCall, Receipt, tx_hash};
use aether_types::{Address, TxEnvelope, TxPayload, U256};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenMove {
    pub token: Address,
    pub from: Address,
    pub to: Address,
    pub amount: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairSwap {
    pub pair: Address,
    pub amount0_in: String,
    pub amount1_in: String,
    pub amount0_out: String,
    pub amount1_out: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub address: Address,
    pub height: u64,
    pub tx_index: u32,
    pub tx_hash: String,
    pub timestamp_ms: u64,
    pub direction: String,
    pub kind: String,
    pub from: Option<Address>,
    pub to: Option<Address>,
    pub value_wei: String,
    pub method: Option<String>,
    pub approval_amount: Option<String>,
    pub approval_spender: Option<Address>,
    pub contract_address: Option<Address>,
    /// WAETH Withdrawal log observed in this transaction. The wallet checks
    /// that `native_payout_source` is its known WAETH contract before using it.
    pub native_received_wei: Option<String>,
    pub native_payout_source: Option<Address>,
    pub success: bool,
    pub tokens: Vec<TokenMove>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pair_swaps: Vec<PairSwap>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Page {
    pub entries: Vec<Entry>,
    pub next_cursor: Option<String>,
    pub history_start: u64,
    pub indexed_height: u64,
}

// The indexed Transfer event is independent of token metadata. A malicious
// contract can emit it too, so the wallet still applies its token-label rules.
const TRANSFER: [u8; 32] = [
    0xdd, 0xf2, 0x52, 0xad, 0x1b, 0xe2, 0xc8, 0x9b, 0x69, 0xc2, 0xb0, 0x68, 0xfc, 0x37, 0x8d, 0xaa,
    0x95, 0x2b, 0xa7, 0xf1, 0x63, 0xc4, 0xa1, 0x16, 0x28, 0xf5, 0x5a, 0x4d, 0xf5, 0x23, 0xb3, 0xef,
];
const WITHDRAWAL: [u8; 32] = [
    0x7f, 0xcf, 0x53, 0x2c, 0x15, 0xf0, 0xa6, 0xdb, 0x0b, 0xd6, 0xd0, 0xe0, 0x38, 0xbe, 0xa7, 0x1d,
    0x30, 0xd8, 0x08, 0xc7, 0xd9, 0x8c, 0xb3, 0xbf, 0x72, 0x68, 0xa9, 0x5b, 0xf5, 0x08, 0x1b, 0x65,
];
const SWAP: [u8; 32] = [
    0xd7, 0x8a, 0xd9, 0x5f, 0xa4, 0x6c, 0x99, 0x4b, 0x65, 0x51, 0xd0, 0xda, 0x85, 0xfc, 0x27, 0x5f,
    0xe6, 0x13, 0xce, 0x37, 0x65, 0x7f, 0xb8, 0xd5, 0xe3, 0xd1, 0x30, 0x84, 0x01, 0x59, 0xd8, 0x22,
];

fn transfer(event: &aether_execution::Event) -> Option<TokenMove> {
    if event.topics.len() != 3 || event.topics[0].0 != TRANSFER || event.data.len() != 32 {
        return None;
    }
    let from = Address::from_slice(&event.topics[1].as_slice()[12..]);
    let to = Address::from_slice(&event.topics[2].as_slice()[12..]);
    Some(TokenMove {
        token: event.address,
        from,
        to,
        amount: U256::from_be_slice(&event.data).to_string(),
    })
}

fn pair_swap(event: &aether_execution::Event) -> Option<PairSwap> {
    if event.topics.len() != 3 || event.topics[0].0 != SWAP || event.data.len() != 128 {
        return None;
    }
    let amount = |i: usize| U256::from_be_slice(&event.data[i * 32..(i + 1) * 32]).to_string();
    Some(PairSwap {
        pair: event.address,
        amount0_in: amount(0),
        amount1_in: amount(1),
        amount0_out: amount(2),
        amount1_out: amount(3),
    })
}

/// `compact_swaps` is enabled only for a new genesis. Legacy history keeps
/// Swap details on every address row for byte-for-byte chain-7780 compatibility.
pub fn transaction(
    tx: &TxEnvelope,
    receipt: &Receipt,
    height: u64,
    index: u32,
    timestamp_ms: u64,
    is_aether_account: bool,
    compact_swaps: bool,
) -> Vec<Entry> {
    let sender = tx.header.sender;
    let call = match &tx.payload {
        TxPayload::Plain(bytes) => EvmCall::decode(bytes).ok(),
        TxPayload::Encrypted { .. } => None,
    };
    let destination = call.as_ref().and_then(|c| c.to);
    let value = call.as_ref().map(|c| c.value).unwrap_or(U256::ZERO);
    let mut native_transfers = BTreeMap::<Address, U256>::new();
    if receipt.success && is_aether_account && destination == Some(sender) {
        if let Some(calls) = call.as_ref().and_then(|c| aether_execution::account::decode_execute(&c.input)) {
            for (to, amount, _) in calls {
                if to != sender && amount > U256::ZERO {
                    let total = native_transfers.entry(to).or_default();
                    *total = total.saturating_add(amount);
                }
            }
        }
    }
    let native_sent = native_transfers.values().copied().fold(U256::ZERO, |total, amount| total.saturating_add(amount));
    let method = call
        .as_ref()
        .and_then(|c| (c.input.len() >= 4).then(|| format!("0x{}", hex::encode(&c.input[..4]))));
    let approval = call
        .as_ref()
        .filter(|c| method.as_deref() == Some("0x095ea7b3") && c.input.len() >= 68);
    let moves: Vec<_> = if receipt.success {
        receipt.events.iter().filter_map(transfer).collect()
    } else {
        Vec::new()
    };
    let pair_swaps: Vec<_> = if receipt.success {
        receipt.events.iter().filter_map(pair_swap).collect()
    } else {
        Vec::new()
    };
    let withdrawal = receipt.events.iter().find(|event| {
        receipt.success
            && event.topics.len() == 2
            && event.topics[0].0 == WITHDRAWAL
            && event.data.len() == 32
    });
    let mut accounts: BTreeMap<Address, Vec<TokenMove>> = BTreeMap::new();
    accounts.entry(sender).or_default();
    if receipt.success && value > U256::ZERO {
        if let Some(to) = destination {
            accounts.entry(to).or_default();
        }
    }
    for recipient in native_transfers.keys() {
        accounts.entry(*recipient).or_default();
    }
    for movement in moves {
        if movement.from != Address::ZERO {
            accounts
                .entry(movement.from)
                .or_default()
                .push(movement.clone());
        }
        if movement.to != Address::ZERO && movement.to != movement.from {
            accounts.entry(movement.to).or_default().push(movement);
        }
    }
    accounts
        .into_iter()
        .map(|(address, tokens)| {
            let native_value = native_transfers.get(&address).copied().unwrap_or(U256::ZERO);
            let native_in = receipt.success
                && ((value > U256::ZERO && destination == Some(address) && sender != address)
                    || native_value > U256::ZERO);
            let token_in = tokens.iter().any(|t| t.to == address && t.from != address);
            let kind = if address == sender && destination.is_none() {
                "deploy"
            } else if address == sender
                && !tokens.is_empty()
                && matches!(method.as_deref(), Some("0xa9059cbb" | "0x23b872dd"))
            {
                "erc20_transfer"
            } else if address == sender && method.is_some() {
                "contract_call"
            } else if address == sender || native_in {
                "native_transfer"
            } else {
                "erc20_transfer"
            };
            Entry {
                address,
                height,
                tx_index: index,
                tx_hash: format!("{:#x}", tx_hash(tx)),
                timestamp_ms,
                direction: (if address == sender {
                    "out"
                } else if native_in || token_in {
                    "in"
                } else {
                    "out"
                })
                .into(),
                kind: kind.into(),
                from: Some(sender),
                to: if native_value > U256::ZERO { Some(address) } else { destination },
                value_wei: (if native_value > U256::ZERO { native_value } else if address == sender && native_sent > U256::ZERO {
                    native_sent
                } else { value }).to_string(),
                method: method.clone(),
                approval_amount: approval
                    .map(|c| U256::from_be_slice(&c.input[36..68]).to_string()),
                approval_spender: approval.map(|c| Address::from_slice(&c.input[16..36])),
                contract_address: receipt.contract_address,
                native_received_wei: withdrawal.map(|e| U256::from_be_slice(&e.data).to_string()),
                native_payout_source: withdrawal.map(|e| e.address),
                success: receipt.success,
                tokens,
                // A swap describes the sender's call. Copying every swap into
                // every token recipient's row makes history grow as the product
                // of Swap and Transfer event counts.
                pair_swaps: if !compact_swaps || address == sender { pair_swaps.clone() } else { Vec::new() },
            }
        })
        .collect()
}

pub fn reward(
    address: Address,
    height: u64,
    index: u32,
    timestamp_ms: u64,
    amount: U256,
    node: bool,
) -> Entry {
    Entry {
        address,
        height,
        tx_index: index,
        tx_hash: format!("reward:{height}:{index}:{address}"),
        timestamp_ms,
        direction: "in".into(),
        kind: (if node { "node_reward" } else { "proof_reward" }).into(),
        from: None,
        to: Some(address),
        value_wei: amount.to_string(),
        method: None,
        approval_amount: None,
        approval_spender: None,
        contract_address: None,
        native_received_wei: None,
        native_payout_source: None,
        success: true,
        tokens: Vec::new(),
        pair_swaps: Vec::new(),
    }
}

pub fn registration(
    address: Address,
    height: u64,
    index: u32,
    timestamp_ms: u64,
    id: aether_types::TxHash,
) -> Entry {
    Entry {
        address,
        height,
        tx_index: index,
        tx_hash: format!("{id:#x}"),
        timestamp_ms,
        direction: "out".into(),
        kind: "registration".into(),
        from: Some(address),
        to: None,
        value_wei: "0".into(),
        method: None,
        approval_amount: None,
        approval_spender: None,
        contract_address: None,
        native_received_wei: None,
        native_payout_source: None,
        success: true,
        tokens: Vec::new(),
        pair_swaps: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_crypto::P256Signer;
    use aether_execution::sign_call;
    use aether_types::{B256, Bytes};

    fn indexed_address(address: Address) -> B256 {
        let mut topic = [0u8; 32];
        topic[12..].copy_from_slice(address.as_slice());
        B256::from(topic)
    }

    fn history_with_events(n: usize, compact_swaps: bool) -> (Address, Vec<Entry>) {
        let signer = P256Signer::from_seed(&[19; 32]).unwrap();
        let call = EvmCall {
            to: Some(Address::repeat_byte(0x99)),
            value: U256::ZERO,
            input: Bytes::new(),
            gas_limit: 15_000_000,
            delegate: None,
        };
        let tx = sign_call(&signer, 7, 0, 0, &call).unwrap();
        let sender = tx.header.sender;
        let mut events = Vec::with_capacity(n * 2);
        for i in 0..n {
            let recipient = Address::repeat_byte(i as u8 + 1);
            events.push(aether_execution::Event {
                address: Address::repeat_byte(0x44),
                topics: vec![B256::from(TRANSFER), indexed_address(sender), indexed_address(recipient)],
                data: Bytes::from(vec![0u8; 32]),
            });
            events.push(aether_execution::Event {
                address: Address::repeat_byte(0x55),
                topics: vec![B256::from(SWAP), B256::ZERO, B256::ZERO],
                data: Bytes::from(vec![0u8; 128]),
            });
        }
        let receipt = Receipt {
            tx_hash: tx_hash(&tx),
            success: true,
            gas_used: 0,
            prove_gas: 0,
            state_gas: 0,
            state_fee: U256::ZERO,
            contract_address: None,
            logs: events.len() as u32,
            output: Bytes::new(),
            events,
        };
        (sender, transaction(&tx, &receipt, 1, 0, 1_000, false, compact_swaps))
    }

    #[test]
    fn swap_details_stay_on_sender_row_as_transfer_recipients_grow() {
        let (sender, small) = history_with_events(24, true);
        let (_, large) = history_with_events(48, true);
        assert_eq!(small.len(), 25);
        assert_eq!(large.len(), 49);
        assert_eq!(large.iter().find(|row| row.address == sender).unwrap().pair_swaps.len(), 48);
        assert!(large.iter().filter(|row| row.address != sender).all(|row| row.pair_swaps.is_empty()));

        let small_bytes = serde_json::to_vec(&small).unwrap().len();
        let large_bytes = serde_json::to_vec(&large).unwrap().len();
        assert!(large_bytes < small_bytes * 3, "history grew faster than events: {small_bytes} -> {large_bytes}");
    }

    #[test]
    fn legacy_history_repeats_swaps_on_recipient_rows() {
        let (_, rows) = history_with_events(2, false);
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|row| row.pair_swaps.len() == 2));
    }
}
