//! What became of a submitted transaction, and the submit order that keeps a
//! sender's queue free of gaps (contracts-live bug #5, 2026-10-06).
//!
//! The stress run showed hashes that never ran and never said why. The node
//! now answers `aether_getReceipt` with `pending` + what it waits for, or
//! `dropped` + the reason. This module turns that answer into one plain
//! Korean sentence for the activity row (and an English one for agents), and
//! says whether signing again with the same nonce and a fresh fee can help.
//!
//! It also serialises submits per process: a transaction is only sent when
//! every lower nonce of its sender is on chain or still queued, so a refusal
//! of nonce N never leaves N+1 waiting behind a gap.

use crate::{call, TxReceipt, WalletError, R};
use aether_types::{Address, TxHash, U256};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Mutex;

/// What became of a submitted transaction.
#[derive(uniffi::Record)]
pub struct TxStatus {
    /// "included", "pending", "dropped" or "unknown" (the node has no record).
    pub state: String,
    /// The node's reason (`state_price_above_cap`, `nonce_gap`, `expired`, …):
    /// what a pending one waits for, or why a dropped one left.
    pub reason: Option<String>,
    /// One plain Korean sentence for the person who sent it.
    pub message: String,
    /// The same answer in English, with the numbers, for agents and logs.
    pub detail: String,
    /// Dropped for a reason that signing again with the same nonce and a
    /// fresh fee can fix ("새 가격으로 다시 보내기").
    pub can_resend: bool,
    pub receipt: Option<TxReceipt>,
}

/// Ask the node what became of `tx_hash` (never an error for a hash it
/// does not know: that is `unknown`).
#[uniffi::export]
pub fn tx_status(tx_hash: String) -> R<TxStatus> {
    let h: TxHash = tx_hash.parse().map_err(|_| WalletError::Invalid("tx hash".into()))?;
    Ok(status_of(&call("aether_getReceipt", json!([h]))?))
}

/// Seconds a block takes on a new genesis (1 s): enough for "about N minutes".
const BLOCK_SECONDS: u64 = 1;

fn wait_text(blocks: u64) -> String {
    let secs = blocks.saturating_mul(BLOCK_SECONDS);
    if secs < 60 {
        format!("{secs}초")
    } else {
        format!("{}분", secs.div_ceil(60))
    }
}

/// The pure half of `tx_status`: the node's answer, in words.
pub(crate) fn status_of(v: &Value) -> TxStatus {
    if v.get("receipt").is_some() {
        let r = &v["receipt"];
        let success = r["success"].as_bool().unwrap_or(false);
        return TxStatus {
            state: "included".into(),
            reason: None,
            message: if success { "완료됐어요.".into() } else { "블록에 들어갔지만 실행이 실패했어요.".into() },
            detail: format!("included in block {} ({})", v["height"], if success { "success" } else { "failed" }),
            can_resend: false,
            receipt: Some(TxReceipt {
                height: v["height"].as_u64().unwrap_or_default(),
                success,
                gas_used: r["gas_used"].as_u64().unwrap_or_default(),
                state_fee_wei: crate::u256_of_json(&r["state_fee"]).unwrap_or(U256::ZERO).to_string(),
            }),
        };
    }
    let pending = v["status"] == "pending" || v["pending"] == true;
    if pending {
        let w = &v["waiting"];
        let kind = w["kind"].as_str().map(str::to_owned);
        let (message, detail) = match kind.as_deref() {
            Some("state_price_above_cap") => {
                let when = w["blocks"].as_u64().map(wait_text);
                (
                    match &when {
                        Some(t) => format!("네트워크가 붐벼 지금 수수료가 이 거래에 허용한 최대치보다 높아요. 약 {t} 뒤 내려가면 처리돼요. 그 전에 10분 대기가 끝나면 취소되고, 돈은 빠져나가지 않아요."),
                        None => "네트워크가 붐벼 지금 수수료가 이 거래에 허용한 최대치보다 높아요. 내려가면 처리돼요.".into(),
                    },
                    format!(
                        "pending: the state price {} is above this transaction's cap {}{}",
                        w["price"].as_str().unwrap_or("?"),
                        w["cap"].as_str().unwrap_or("?"),
                        w["blocks"].as_u64().map(|b| format!("; about {b} blocks until it falls to the cap")).unwrap_or_default()
                    ),
                )
            }
            Some("nonce_gap") => (
                "앞서 보낸 거래가 먼저 처리되기를 기다리고 있어요.".into(),
                format!("pending: waiting for nonce {} of the same sender", w["expected"]),
            ),
            Some("fee_cap_below_base") => (
                "네트워크 수수료가 잠시 올라 내려가기를 기다리고 있어요.".into(),
                "pending: the base fee is above this transaction's fee cap".into(),
            ),
            _ => ("처리 중이에요.".into(), "pending: waiting for a block".into()),
        };
        return TxStatus { state: "pending".into(), reason: kind, message, detail, can_resend: false, receipt: None };
    }
    if v["status"] == "dropped" {
        let r = &v["reason"];
        let kind = r["kind"].as_str().unwrap_or("unknown").to_owned();
        let kept = "돈은 빠져나가지 않았어요.";
        let (message, detail) = match kind.as_str() {
            "state_price_above_cap" => (
                format!("네트워크가 붐벼 수수료가 이 거래에 허용한 최대치보다 올라 처리되지 않았어요. {kept} 새 가격으로 다시 보낼 수 있어요."),
                format!(
                    "dropped: the state price {} stayed above this transaction's cap {} until the mempool expired it",
                    r["price"].as_str().unwrap_or("?"),
                    r["cap"].as_str().unwrap_or("?")
                ),
            ),
            "fee_cap_below_base" => (
                format!("네트워크 수수료가 올라 처리되지 않았어요. {kept} 새 가격으로 다시 보낼 수 있어요."),
                "dropped: the base fee stayed above this transaction's fee cap".into(),
            ),
            "nonce_gap" => (
                format!("앞서 보낸 거래가 처리되지 않아 이 거래도 처리되지 않았어요. {kept} 다시 보낼 수 있어요."),
                format!("dropped: it waited behind a nonce gap (nonce {} never arrived)", r["expected"]),
            ),
            "expired" => (
                format!("오래 기다려도 처리되지 않아 취소됐어요. {kept} 다시 보낼 수 있어요."),
                "dropped: it waited the whole mempool TTL".into(),
            ),
            "evicted" => (
                format!("네트워크 대기열이 가득 차 처리되지 않았어요. {kept} 다시 보낼 수 있어요."),
                "dropped: a higher-paying transaction took its place in a full mempool".into(),
            ),
            "replaced" => (
                "같은 순서 번호로 보낸 다른 거래가 대신 처리됐어요. 이 거래로는 돈이 빠져나가지 않았어요.".into(),
                "dropped: another transaction with the same nonce was included".into(),
            ),
            "unaffordable" => (
                format!("잔액이 부족해 처리되지 않았어요. {kept}"),
                "dropped: the balance no longer covers it".into(),
            ),
            _ => (format!("처리되지 않았어요. {kept}"), format!("dropped: {r}")),
        };
        let can_resend = v["resendable"].as_bool().unwrap_or(!matches!(kind.as_str(), "replaced" | "unaffordable"));
        return TxStatus { state: "dropped".into(), reason: Some(kind), message, detail, can_resend, receipt: None };
    }
    TxStatus {
        state: "unknown".into(),
        reason: None,
        message: "네트워크에서 이 거래를 찾지 못했어요.".into(),
        detail: "unknown: the node has no receipt, no pending entry and no record of a drop for this hash".into(),
        can_resend: false,
        receipt: None,
    }
}

// ---------------- per-sender submit order (bug #5, decision 5) ----------------

/// The newest transaction this process queued for each sender: (nonce, hash).
static QUEUED: Mutex<BTreeMap<Address, (u64, TxHash)>> = Mutex::new(BTreeMap::new());
/// One submit at a time, so two sends never race for the same queue slot.
static SUBMIT: Mutex<()> = Mutex::new(());

/// The nonce a sender's next transaction takes: right after this process's
/// newest queued one while it is still pending (`queued_pending`), else the
/// chain's next nonce.
pub(crate) fn next_nonce(chain_nonce: u64, queued: Option<u64>, queued_pending: bool) -> u64 {
    match queued {
        Some(n) if n >= chain_nonce && queued_pending => n + 1,
        _ => chain_nonce,
    }
}

/// Refuse a submit that would wait behind a gap: `nonce` above the next one
/// the sender can use means an earlier send was refused or dropped.
pub(crate) fn gap_refusal(nonce: u64, next: u64) -> Option<String> {
    (nonce > next).then(|| {
        format!(
            "nonce {nonce} would wait behind a gap: this account's next transaction is nonce {next} (an earlier send was refused or dropped). Nothing was sent; prepare it again"
        )
    })
}

fn chain_nonce(from: Address) -> R<u64> {
    let hex = call("eth_getTransactionCount", json!([from]))?;
    u64::from_str_radix(hex.as_str().unwrap_or("0x0").trim_start_matches("0x"), 16).map_err(|e| WalletError::Invalid(e.to_string()))
}

fn pending_at_node(h: TxHash) -> bool {
    call("aether_getReceipt", json!([h])).map(|v| status_of(&v).state == "pending").unwrap_or(false)
}

/// The nonce `from`'s next transaction should sign (see `next_nonce`).
pub(crate) fn nonce_for(from: Address) -> R<u64> {
    let chain = chain_nonce(from)?;
    let queued = QUEUED.lock().expect("queued lock").get(&from).copied();
    let pending = queued.is_some_and(|(n, h)| n >= chain && pending_at_node(h));
    Ok(next_nonce(chain, queued.map(|(n, _)| n), pending))
}

/// Submit `send` for `from` at `nonce` under the per-process submit lock:
/// refused (nothing sent) when an earlier nonce is neither on chain nor
/// queued; on success the hash becomes the sender's newest queued tx.
pub(crate) fn submit_in_order(from: Address, nonce: u64, send: impl FnOnce() -> R<TxHash>) -> R<TxHash> {
    let _serial = SUBMIT.lock().unwrap_or_else(|p| p.into_inner());
    let next = nonce_for(from)?;
    if let Some(why) = gap_refusal(nonce, next) {
        return Err(WalletError::Rejected(why));
    }
    let h = send()?;
    let mut q = QUEUED.lock().expect("queued lock");
    if q.get(&from).is_none_or(|(n, _)| nonce >= *n || *n > next) {
        q.insert(from, (nonce, h));
    }
    Ok(h)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Contracts-live bug #5: a pending tx under the risen state price, and
    /// the same tx once the TTL dropped it, each come back with a reason.
    #[test]
    fn a_stuck_transfer_says_why_in_plain_words() {
        let pending = json!({ "pending": true, "status": "pending",
            "waiting": { "kind": "state_price_above_cap", "cap": "2000000000000", "price": "43000000000000", "blocks": 1_200 } });
        let s = status_of(&pending);
        assert_eq!((s.state.as_str(), s.reason.as_deref()), ("pending", Some("state_price_above_cap")));
        assert!(s.message.contains("약 20분"), "{}", s.message);
        assert!(s.detail.contains("43000000000000") && s.detail.contains("1200 blocks"), "{}", s.detail);
        assert!(!s.can_resend, "it may still land: no second signature yet");

        let dropped = json!({ "pending": false, "status": "dropped", "resendable": true,
            "reason": { "kind": "state_price_above_cap", "cap": "2000000000000", "price": "43000000000000" } });
        let s = status_of(&dropped);
        assert_eq!((s.state.as_str(), s.reason.as_deref()), ("dropped", Some("state_price_above_cap")));
        assert!(s.can_resend);
        assert!(s.message.contains("돈은 빠져나가지 않았어요") && s.message.contains("다시 보낼 수"), "{}", s.message);

        let replaced = status_of(&json!({ "status": "dropped", "reason": { "kind": "replaced" } }));
        assert!(!replaced.can_resend, "the nonce is used: re-signing it cannot run");
        let gap = status_of(&json!({ "status": "dropped", "resendable": true, "reason": { "kind": "nonce_gap", "expected": 4 } }));
        assert!(gap.detail.contains("nonce 4") && gap.can_resend);

        // The old pending shape (an older node) still reads as pending.
        assert_eq!(status_of(&json!({ "pending": true })).state, "pending");
        assert_eq!(status_of(&Value::Null).state, "unknown");
        let done = status_of(&json!({ "height": 9, "receipt": { "success": true, "gas_used": 21000, "state_fee": "0" } }));
        assert_eq!(done.state, "included");
        assert_eq!(done.receipt.unwrap().height, 9);
    }

    /// Decision 5: nonces follow this process's queued txs, and a submit that
    /// would sit behind a refused or dropped nonce is refused, not queued.
    #[test]
    fn submits_never_queue_behind_a_gap() {
        assert_eq!(next_nonce(5, None, false), 5);
        assert_eq!(next_nonce(5, Some(5), true), 6, "nonce 5 is queued: the next send takes 6");
        assert_eq!(next_nonce(5, Some(5), false), 5, "nonce 5 was dropped: take it again");
        assert_eq!(next_nonce(7, Some(5), true), 7, "the chain moved past it");
        assert_eq!(gap_refusal(6, 6), None);
        assert_eq!(gap_refusal(5, 6), None, "re-signing a lower nonce replaces, it never waits");
        let why = gap_refusal(7, 6).expect("nonce 6 is missing: 7 would wait forever");
        assert!(why.contains("nonce 6") && why.contains("Nothing was sent"), "{why}");
    }
}
