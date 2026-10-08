//! Chain-only adapters for the public AppRegistry and EastSeaNames event ABIs.
//! IDs include the emitting contract: an unrelated log cannot mutate another
//! registry's record. Text is a publisher claim, never fetched from a hint URL.

use crate::search::{SearchEvent, SearchMetadata};
use aether_execution::{CallTargets, Event, Receipt, WorldState};
use aether_types::{Address, TxEnvelope, B256};
use alloy_primitives::keccak256;
use std::collections::BTreeSet;

const GRACE: u64 = 30 * 24 * 60 * 60;

fn id(kind: &str, source: Address, key: B256) -> String {
    format!("{kind}:{source:#x}:{key:#x}")
}

fn word(data: &[u8], index: usize) -> Option<B256> {
    let start = index.checked_mul(32)?;
    Some(B256::from_slice(data.get(start..start.checked_add(32)?)?))
}

fn number(data: &[u8], index: usize) -> Option<u64> {
    let bytes = word(data, index)?;
    if bytes[..24].iter().any(|b| *b != 0) {
        return None;
    }
    Some(u64::from_be_bytes(bytes[24..].try_into().ok()?))
}

fn address(topic: B256) -> Option<Address> {
    if topic[..12].iter().any(|b| *b != 0) {
        return None;
    }
    Some(Address::from_slice(&topic[12..]))
}

fn string(data: &[u8], index: usize, head_words: usize, max: usize) -> Option<String> {
    let start = usize::try_from(number(data, index)?).ok()?;
    if start % 32 != 0 || start < head_words.checked_mul(32)? {
        return None;
    }
    let tail = data.get(start..)?;
    let len = usize::try_from(number(tail, 0)?).ok()?;
    if len > max {
        return None;
    }
    let value = std::str::from_utf8(tail.get(32..32usize.checked_add(len)?)?).ok()?;
    Some(value.to_owned())
}

fn topic(event: &Event, signature: &str, count: usize) -> bool {
    event.topics.len() == count && event.topics.first() == Some(&keccak256(signature))
}

fn nonzero(hash: B256) -> Option<B256> {
    (hash != B256::ZERO).then_some(hash)
}

fn valid_name(name: &str) -> bool {
    let bare = name.strip_suffix(".sea").unwrap_or(name);
    !bare.is_empty()
        && bare.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 32
                && !label.starts_with('-')
                && !label.ends_with('-')
                && !label.as_bytes().get(2..4).is_some_and(|p| p == b"--")
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

pub fn app_url(app_id: B256) -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut host = String::with_capacity(52);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for byte in app_id.as_slice().iter().copied() {
        buffer = (buffer << 8) | byte as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            host.push(ALPHABET[((buffer >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        host.push(ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    format!("sea://{host}/")
}

pub fn app_id(publisher: Address, slug: &str) -> B256 {
    let mut encoded = vec![0; 96 + slug.len().div_ceil(32) * 32];
    encoded[12..32].copy_from_slice(publisher.as_slice());
    encoded[63] = 64;
    encoded[64..96].copy_from_slice(&aether_types::U256::from(slug.len()).to_be_bytes::<32>());
    encoded[96..96 + slug.len()].copy_from_slice(slug.as_bytes());
    keccak256(encoded)
}

/// A bounded log produces at most two events (the shared transfer ABI).
pub fn decode(event: &Event, at: u64) -> Vec<SearchEvent> {
    let mut result = Vec::new();
    let data = event.data.as_ref();
    let parse = || -> Option<Vec<SearchEvent>> {
        if topic(
            event,
            "Published(bytes32,address,string,bytes32,bytes32,address,string)",
            3,
        ) {
            let name = string(data, 0, 5, 32)?;
            if !valid_name(&name) || name.contains('.') || name.len() < 3 {
                return None;
            }
            let publisher = address(event.topics[2])?;
            if publisher == Address::ZERO || app_id(publisher, &name) != event.topics[1] {
                return None;
            }
            let hint = string(data, 4, 5, 256).unwrap_or_default();
            return Some(vec![SearchEvent::AppPublished {
                id: id("app", event.address, event.topics[1]),
                publisher,
                name,
                content_hash: nonzero(word(data, 2)?).or_else(|| nonzero(word(data, 1)?)),
                metadata: SearchMetadata::from_hint(&hint),
                url: app_url(event.topics[1]),
                created_at: at,
            }]);
        }
        if topic(
            event,
            "ReleaseQueued(bytes32,uint32,bytes32,bytes32,bool,uint64,string)",
            2,
        ) {
            let hint = string(data, 5, 6, 256).unwrap_or_default();
            return Some(vec![SearchEvent::AppReleaseQueued {
                id: id("app", event.address, event.topics[1]),
                content_hash: nonzero(word(data, 2)?),
                metadata: SearchMetadata::from_hint(&hint),
                activates_at: number(data, 4)?,
            }]);
        }
        if topic(event, "UnlistQueued(bytes32,uint64)", 2) {
            return Some(vec![SearchEvent::AppUnlistQueued {
                id: id("app", event.address, event.topics[1]),
                activates_at: number(data, 0)?,
            }]);
        }
        if topic(event, "PendingCancelled(bytes32,uint8,address)", 3) {
            return Some(vec![SearchEvent::PendingCancelled {
                id: id("app", event.address, event.topics[1]),
            }]);
        }
        if topic(
            event,
            "Registered(string,bytes32,address,uint64,uint256)",
            3,
        ) {
            let name = string(data, 0, 3, 127)?;
            if !valid_name(&name) || keccak256(name.as_bytes()) != event.topics[1] {
                return None;
            }
            return Some(vec![SearchEvent::NameRegistered {
                id: id("name", event.address, event.topics[1]),
                name,
                owner: address(event.topics[2])?,
                expires_at: number(data, 1)?.saturating_add(GRACE),
                created_at: at,
            }]);
        }
        if topic(event, "Renewed(bytes32,uint64,uint256)", 2) {
            return Some(vec![SearchEvent::NameRenewed {
                id: id("name", event.address, event.topics[1]),
                expires_at: number(data, 0)?.saturating_add(GRACE),
            }]);
        }
        if topic(event, "AddrSet(bytes32,address)", 3) {
            return Some(vec![SearchEvent::NameAddress {
                id: id("name", event.address, event.topics[1]),
                address: address(event.topics[2])?,
            }]);
        }
        if topic(event, "TextSet(bytes32,string,string)", 2) {
            return Some(vec![SearchEvent::NameText {
                id: id("name", event.address, event.topics[1]),
                key: string(data, 0, 2, 32)?,
                value: string(data, 1, 2, 128).unwrap_or_default(),
            }]);
        }
        if topic(event, "TransferAccepted(bytes32,address,address)", 4) {
            let owner = address(event.topics[3])?;
            return Some(vec![
                SearchEvent::AppTransferred {
                    id: id("app", event.address, event.topics[1]),
                    publisher: owner,
                },
                SearchEvent::NameTransferred {
                    id: id("name", event.address, event.topics[1]),
                    owner,
                },
            ]);
        }
        None
    };
    if let Some(events) = parse() {
        result.extend(events);
    }
    result
}

pub fn decode_from_source(
    event: &Event,
    state: &WorldState,
    sources: &crate::search_sources::SearchSources,
    at: u64,
) -> Vec<SearchEvent> {
    let app = sources.allows_app(event.address, state);
    let name = sources.allows_name(event.address, state);
    if !app && !name {
        return Vec::new();
    }
    decode(event, at)
        .into_iter()
        .filter(|event| match event {
            SearchEvent::AppPublished { .. }
            | SearchEvent::AppUpdated { .. }
            | SearchEvent::AppReleaseQueued { .. }
            | SearchEvent::AppUnlistQueued { .. }
            | SearchEvent::AppTransferred { .. }
            | SearchEvent::AppRemoved { .. }
            | SearchEvent::PendingCancelled { .. } => app,
            SearchEvent::NameRegistered { .. }
            | SearchEvent::NameAddress { .. }
            | SearchEvent::NameText { .. }
            | SearchEvent::NameTransferred { .. }
            | SearchEvent::NameRenewed { .. } => name,
            _ => false,
        })
        .collect()
}

/// Successful finalized execution only. The inspector supplies actual direct
/// and internal calls in transaction order, including silent account batches.
pub fn block_events(
    txs: &[TxEnvelope],
    receipts: &[Receipt],
    traces: &[CallTargets],
    state: &WorldState,
    sources: &crate::search_sources::SearchSources,
    at: u64,
) -> Vec<SearchEvent> {
    let mut events = vec![SearchEvent::Tick { at }];
    for (i, (tx, receipt)) in txs.iter().zip(receipts).enumerate() {
        if !receipt.success {
            continue;
        }
        for log in &receipt.events {
            events.extend(decode_from_source(log, state, sources, at));
        }
        let mut called = BTreeSet::new();
        if let Some(trace) = traces.get(i) {
            called.extend(trace.addresses.iter().copied());
            if !trace.complete {
                events.push(SearchEvent::UsageIncomplete { at });
            }
        } else {
            events.push(SearchEvent::UsageIncomplete { at });
        }
        events.extend(
            called
                .into_iter()
                .map(|contract| SearchEvent::ContractCalled {
                    contract,
                    caller: tx.header.sender,
                    at,
                }),
        );
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_abi_is_ignored_without_allocating_unbounded_text() {
        let event = Event {
            address: Address::repeat_byte(1),
            topics: vec![
                keccak256("TextSet(bytes32,string,string)"),
                B256::repeat_byte(2),
            ],
            data: vec![0xff; 256].into(),
        };
        assert!(decode(&event, 123).is_empty());
        assert_eq!(app_url(B256::ZERO), format!("sea://{}/", "a".repeat(52)));
        assert!(valid_name("docs.alice.sea"));
        assert!(!valid_name("xn--alice"));
        assert!(!valid_name("alice..sea"));
    }
}
