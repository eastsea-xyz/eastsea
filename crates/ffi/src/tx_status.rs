//! What became of a submitted transaction, and the submit order that keeps a
//! sender's queue free of gaps (contracts-live bug #5, 2026-10-06; B5 review
//! round 2, 2026-10-07).
//!
//! The stress run showed hashes that never ran and never said why. The node
//! now answers `aether_getReceipt` with `pending` + what it waits for, or
//! `dropped` + the reason. This module turns that answer into one plain
//! Korean sentence for the activity row (and an English one for agents), and
//! says whether signing again with the same nonce and a fresh fee can help.
//!
//! A drop is one node's observation, not a chain fact: another node may still
//! hold the transaction and include it. So a drop (or a node that has no
//! record) is never final here. The only final answers are chain facts — a
//! receipt, or the sender's nonce used on chain by another transaction — and
//! this process keeps each send's context (sender, nonce, the validator that
//! admitted it) until one of them is seen (round 2, findings 2 and 5).
//!
//! It also serialises submits per process: a transaction is only sent when
//! every lower nonce of its sender is on chain or still pending in one
//! unbroken run from the chain nonce, so an older send that drops while a
//! younger one still waits never lets a third queue behind the gap (round 2,
//! finding 3).

use crate::{call, TxReceipt, WalletError, R};
use aether_types::{Address, TxEnvelope, TxHash, TxPayload, U256};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Mutex;

/// What became of a submitted transaction.
#[derive(uniffi::Record)]
pub struct TxStatus {
    /// "included", "pending", "dropped" (one node removed it: not on chain
    /// yet), "replaced" (its nonce was used on chain by another transaction)
    /// or "unknown" (the nodes asked have no record).
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
    /// A chain fact settled it: `included`, or `replaced`. Anything else can
    /// still change — a later receipt supersedes a drop.
    pub is_final: bool,
}

/// Ask the network what became of `tx_hash`: the validator that admitted it
/// first (when this process sent it), then the ordinary read path, then a
/// validator when a follower has no record (finding 2). Never an error for a
/// hash nobody knows: that is `unknown`. A send this process made is also
/// reconciled against its nonce (see `tx_status_for`).
#[uniffi::export]
pub fn tx_status(tx_hash: String) -> R<TxStatus> {
    let h: TxHash = tx_hash.parse().map_err(|_| WalletError::Invalid("tx hash".into()))?;
    let context = book().sent.get(&h).map(|s| (s.sender, s.nonce));
    status_with_context(h, context)
}

/// `tx_status` for a send whose sender and nonce the caller kept (a wallet
/// row after a restart, an agent's pending history): when the chain nonce has
/// moved past `nonce` and no node has this hash's receipt, another transaction
/// used the nonce — `replaced`, final. Until then a drop stays not-final.
#[uniffi::export]
pub fn tx_status_for(tx_hash: String, sender: String, nonce: u64) -> R<TxStatus> {
    let h: TxHash = tx_hash.parse().map_err(|_| WalletError::Invalid("tx hash".into()))?;
    let sender: Address = sender.parse().map_err(|_| WalletError::Invalid("sender address".into()))?;
    status_with_context(h, Some((sender, nonce)))
}

fn status_with_context(h: TxHash, context: Option<(Address, u64)>) -> R<TxStatus> {
    // The nonce is read BEFORE the receipt: a nonce that moved past ours
    // while the receipt is still unseen can then only mean another tx used it
    // (read the other way round, our own inclusion in between would look so).
    let chain = match context {
        Some((sender, _)) => chain_nonce(sender).ok(),
        None => None,
    };
    let st = status_of(&crate::receipt_answer(h, false)?);
    if st.is_final {
        book().settled(&h);
        return Ok(st);
    }
    let (Some((_, nonce)), Some(chain)) = (context, chain) else { return Ok(st) };
    if !nonce_used_elsewhere(chain, nonce, false) {
        return Ok(st);
    }
    // Every path once more before calling it replaced: a receipt anywhere wins.
    let again = status_of(&crate::receipt_answer(h, true)?);
    let out = if again.is_final { again } else { replaced(nonce, chain) };
    book().settled(&h);
    Ok(out)
}

/// The chain's nonce moved past ours and no receipt names our hash.
pub(crate) fn nonce_used_elsewhere(chain_nonce: u64, nonce: u64, included: bool) -> bool {
    !included && chain_nonce > nonce
}

fn replaced(nonce: u64, chain: u64) -> TxStatus {
    TxStatus {
        state: "replaced".into(),
        reason: Some("nonce_used".into()),
        message: format!("같은 순서 번호({nonce})로 보낸 다른 거래가 체인에 기록됐어요. 이 거래는 처리되지 않으며, 이 거래로 빠져나간 돈은 없어요."),
        detail: format!("replaced: nonce {nonce} is used on chain (the account's next nonce is {chain}) by another transaction; this one can no longer run"),
        can_resend: false,
        receipt: None,
        is_final: true,
    }
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

/// How a not-included transaction is worded (round 2, finding 5): what one
/// node saw, never a permanent failure.
pub const NOT_RECORDED: &str = "처리되지 않았어요 (아직 체인에 기록되지 않음)";

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
            is_final: true,
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
                        Some(t) => format!("네트워크가 붐벼 지금 수수료가 이 거래에 허용한 최대치보다 높아요. 약 {t} 뒤 내려가면 처리돼요. 10분 안에 내려가지 않으면 처리되지 않을 수 있고, 그때는 새 가격으로 다시 보낼 수 있어요."),
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
        return TxStatus { state: "pending".into(), reason: kind, message, detail, can_resend: false, receipt: None, is_final: false };
    }
    if v["status"] == "dropped" {
        let r = &v["reason"];
        let kind = r["kind"].as_str().unwrap_or("unknown").to_owned();
        // One node's drop: "not recorded yet", never a permanent failure.
        let (message, detail) = match kind.as_str() {
            "state_price_above_cap" => (
                format!("네트워크가 붐벼 수수료가 이 거래에 허용한 최대치보다 올라 {NOT_RECORDED}. 새 가격으로 다시 보낼 수 있어요."),
                format!(
                    "dropped by this node: the state price {} stayed above this transaction's cap {} until its mempool expired it (not on chain yet; another node may still include it)",
                    r["price"].as_str().unwrap_or("?"),
                    r["cap"].as_str().unwrap_or("?")
                ),
            ),
            "fee_cap_below_base" => (
                format!("네트워크 수수료가 올라 {NOT_RECORDED}. 새 가격으로 다시 보낼 수 있어요."),
                "dropped by this node: the base fee stayed above this transaction's fee cap (not on chain yet)".into(),
            ),
            "nonce_gap" => (
                format!("앞서 보낸 거래가 처리되지 않아 이 거래도 {NOT_RECORDED}. 다시 보낼 수 있어요."),
                format!("dropped by this node: it waited behind a nonce gap (nonce {} never arrived; not on chain yet)", r["expected"]),
            ),
            "expired" => (
                format!("오래 기다려도 {NOT_RECORDED}. 다시 보낼 수 있어요."),
                "dropped by this node: it waited the whole mempool TTL (not on chain yet)".into(),
            ),
            "evicted" => (
                format!("네트워크 대기열이 가득 차 {NOT_RECORDED}. 다시 보낼 수 있어요."),
                "dropped by this node: a higher-paying transaction took its place in a full mempool (not on chain yet)".into(),
            ),
            "replaced" => (
                format!("같은 순서 번호로 보낸 다른 거래가 먼저 처리돼 이 거래는 {NOT_RECORDED}."),
                "dropped by this node: another transaction with the same nonce was included there".into(),
            ),
            "unaffordable" => (
                format!("잔액이 부족해 {NOT_RECORDED}."),
                "dropped by this node: the balance no longer covers it (not on chain yet)".into(),
            ),
            _ => (format!("{NOT_RECORDED}."), format!("dropped by this node: {r} (not on chain yet)")),
        };
        let can_resend = v["resendable"].as_bool().unwrap_or(!matches!(kind.as_str(), "replaced" | "unaffordable"));
        return TxStatus { state: "dropped".into(), reason: Some(kind), message, detail, can_resend, receipt: None, is_final: false };
    }
    TxStatus {
        state: "unknown".into(),
        reason: None,
        message: format!("아직 이 거래의 기록을 찾지 못했어요. {NOT_RECORDED}."),
        detail: "unknown: the nodes asked have no receipt, no pending entry and no record of a drop for this hash (not on chain yet)".into(),
        can_resend: false,
        receipt: None,
        is_final: false,
    }
}

// ---------------- what this process sent (round 2, findings 2, 3, 5) ----------------

/// At most this many unsettled sends are remembered (oldest forgotten first).
const MAX_SENT: usize = 1_024;
/// At most this many senders' queues are kept.
const MAX_SENDERS: usize = 16;
/// A sender's queue never holds more than the node's per-sender limit.
const MAX_QUEUED: usize = 64;

/// One send this process submitted and has not seen settled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Sent {
    pub sender: Address,
    pub nonce: u64,
    /// The transaction nonce plus any self-delegation authorization nonce.
    pub nonce_consumption: u64,
    /// The validator that accepted it (None: this Mac's own node, or unknown).
    pub admitter: Option<aether_net::EndpointId>,
}

/// Bounded memory of this process's sends: each hash's context until a chain
/// fact settles it, and each sender's queue — nonce → the hash last sent at
/// it — from which the next nonce is the end of the unbroken pending run.
#[derive(Default)]
pub(crate) struct Book {
    pub sent: HashMap<TxHash, Sent>,
    order: VecDeque<TxHash>,
    queued: BTreeMap<Address, BTreeMap<u64, (TxHash, u64)>>,
}

impl Book {
    /// Remember a submission.
    pub fn record(&mut self, h: TxHash, sent: Sent) {
        if self.sent.insert(h, sent).is_none() {
            self.order.push_back(h);
        }
        while self.order.len() > MAX_SENT {
            if let Some(old) = self.order.pop_front() {
                self.sent.remove(&old);
            }
        }
        if !self.queued.contains_key(&sent.sender) && self.queued.len() >= MAX_SENDERS {
            let victim = self.queued.keys().next().copied();
            if let Some(v) = victim {
                self.queued.remove(&v);
            }
        }
        let q = self.queued.entry(sent.sender).or_default();
        q.insert(sent.nonce, (h, sent.nonce_consumption));
        while q.len() > MAX_QUEUED {
            q.pop_first();
        }
    }

    /// A chain fact settled `h`: its context is no longer needed.
    pub fn settled(&mut self, h: &TxHash) {
        if let Some(s) = self.sent.remove(h) {
            self.order.retain(|o| o != h);
            if let Some(q) = self.queued.get_mut(&s.sender) {
                if q.get(&s.nonce).is_some_and(|(queued, _)| queued == h) {
                    q.remove(&s.nonce);
                }
            }
        }
    }

    /// `sender`'s queue from `chain_nonce` up (lower nonces are on chain).
    pub fn queue_from(&mut self, sender: Address, chain_nonce: u64) -> BTreeMap<u64, (TxHash, u64)> {
        let Some(q) = self.queued.get_mut(&sender) else { return BTreeMap::new() };
        *q = q.split_off(&chain_nonce);
        q.clone()
    }

    pub fn admitter_of(&self, h: &TxHash) -> Option<aether_net::EndpointId> {
        self.sent.get(h).and_then(|s| s.admitter)
    }
}

static BOOK: Mutex<Option<Book>> = Mutex::new(None);

pub(crate) struct BookGuard(std::sync::MutexGuard<'static, Option<Book>>);

impl std::ops::Deref for BookGuard {
    type Target = Book;
    fn deref(&self) -> &Book {
        self.0.as_ref().expect("initialised in book()")
    }
}

impl std::ops::DerefMut for BookGuard {
    fn deref_mut(&mut self) -> &mut Book {
        self.0.as_mut().expect("initialised in book()")
    }
}

/// This process's sends (never held across a network read).
pub(crate) fn book() -> BookGuard {
    let mut g = BOOK.lock().unwrap_or_else(|p| p.into_inner());
    g.get_or_insert_with(Book::default);
    BookGuard(g)
}

// ---------------- per-sender submit order (bug #5 decision 5; round 2 finding 3) ----------------

/// One submit at a time, so two sends never race for the same queue slot.
static SUBMIT: Mutex<()> = Mutex::new(());

/// The nonce a sender's next transaction takes: the end of the unbroken run
/// of this process's queued sends that are still pending, starting at the
/// chain's next nonce. An older send that dropped ends the run there, even
/// while a younger one still waits (it waits behind that very gap).
pub(crate) fn next_in_sequence(chain_nonce: u64, queued: &BTreeMap<u64, (TxHash, u64)>, mut pending: impl FnMut(&TxHash) -> bool) -> u64 {
    let mut next = chain_nonce;
    while let Some((h, consumed)) = queued.get(&next) {
        if !pending(h) {
            break;
        }
        let Some(after) = next.checked_add(*consumed) else { return u64::MAX };
        next = after;
    }
    next
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

/// Pending at the node that can know (the admitting validator first).
fn pending_at_node(h: &TxHash) -> bool {
    crate::receipt_answer(*h, false).map(|v| status_of(&v).state == "pending").unwrap_or(false)
}

/// The nonce `from`'s next transaction should sign (see `next_in_sequence`).
/// At most one receipt read per queued nonce (≤ 64), none under a lock.
pub(crate) fn nonce_for(from: Address) -> R<u64> {
    let chain = chain_nonce(from)?;
    let queue = book().queue_from(from, chain);
    Ok(next_in_sequence(chain, &queue, pending_at_node))
}

/// Submit `send` for `from` at `nonce` under the per-process submit lock:
/// refused (nothing sent) when an earlier nonce is neither on chain nor in
/// the unbroken pending run; on success the hash, its nonce and the
/// validator that admitted it are remembered until a chain fact settles it.
pub(crate) fn submit_in_order(
    env: &TxEnvelope,
    send: impl FnOnce() -> R<(TxHash, Option<aether_net::EndpointId>)>,
) -> R<TxHash> {
    let (from, nonce) = (env.header.sender, env.header.nonce);
    let nonce_consumption = nonce_consumption(&env.payload)?;
    if nonce.checked_add(nonce_consumption).is_none() {
        return Err(WalletError::Invalid("transaction exhausts the account nonce range".into()));
    }
    let _serial = SUBMIT.lock().unwrap_or_else(|p| p.into_inner());
    let next = nonce_for(from)?;
    if let Some(why) = gap_refusal(nonce, next) {
        return Err(WalletError::Rejected(why));
    }
    let (h, admitter) = send()?;
    book().record(h, Sent { sender: from, nonce, nonce_consumption, admitter });
    Ok(h)
}

/// Execution applies a self-authorization at `nonce + 1`, even when the
/// call reuses or clears an existing delegation. Read the signed payload.
fn nonce_consumption(payload: &TxPayload) -> R<u64> {
    let TxPayload::Plain(bytes) = payload else {
        return Err(WalletError::Invalid("encrypted transaction payload not supported".into()));
    };
    let call = aether_execution::EvmCall::decode(bytes).map_err(|e| WalletError::Invalid(format!("transaction payload: {e:?}")))?;
    Ok(1 + u64::from(call.delegate.is_some()))
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
        assert!(!s.is_final);

        let dropped = json!({ "pending": false, "status": "dropped", "resendable": true,
            "reason": { "kind": "state_price_above_cap", "cap": "2000000000000", "price": "43000000000000" } });
        let s = status_of(&dropped);
        assert_eq!((s.state.as_str(), s.reason.as_deref()), ("dropped", Some("state_price_above_cap")));
        assert!(s.can_resend);
        assert!(s.message.contains("다시 보낼 수"), "{}", s.message);

        let replaced = status_of(&json!({ "status": "dropped", "reason": { "kind": "replaced" } }));
        assert!(!replaced.can_resend, "the nonce is used: re-signing it cannot run");
        let gap = status_of(&json!({ "status": "dropped", "resendable": true, "reason": { "kind": "nonce_gap", "expected": 4 } }));
        assert!(gap.detail.contains("nonce 4") && gap.can_resend);

        // The old pending shape (an older node) still reads as pending.
        assert_eq!(status_of(&json!({ "pending": true })).state, "pending");
        assert_eq!(status_of(&Value::Null).state, "unknown");
        let done = status_of(&json!({ "height": 9, "receipt": { "success": true, "gas_used": 21000, "state_fee": "0" } }));
        assert_eq!(done.state, "included");
        assert!(done.is_final);
        assert_eq!(done.receipt.unwrap().height, 9);
    }

    /// Round 2, finding 5: a node-local drop, or a node with no record, is
    /// "not recorded yet" — never final, never worded as a permanent failure
    /// or as money that can never move. Only a receipt, or the nonce used on
    /// chain by another transaction, is final.
    #[test]
    fn a_drop_is_not_final_and_says_not_recorded_yet() {
        for kind in ["state_price_above_cap", "fee_cap_below_base", "nonce_gap", "expired", "evicted", "replaced", "unaffordable", "something_new"] {
            let s = status_of(&json!({ "status": "dropped", "reason": { "kind": kind } }));
            assert!(!s.is_final, "{kind}: one node's drop is not a chain fact");
            assert!(s.message.contains(NOT_RECORDED), "{kind}: {}", s.message);
            for permanent in ["취소됐어요", "빠져나가지 않았어요", "실패"] {
                assert!(!s.message.contains(permanent), "{kind} says {permanent:?}: {}", s.message);
            }
            assert!(s.detail.contains("by this node"), "{kind}: {}", s.detail);
        }
        let unknown = status_of(&Value::Null);
        assert!(!unknown.is_final && unknown.message.contains(NOT_RECORDED), "{}", unknown.message);
        let pending = status_of(&json!({ "pending": true, "status": "pending",
            "waiting": { "kind": "state_price_above_cap", "cap": "2", "price": "4", "blocks": 30 } }));
        assert!(!pending.message.contains("취소"), "{}", pending.message);

        // The nonce moved past ours with no receipt for our hash: final.
        assert!(nonce_used_elsewhere(8, 7, false));
        assert!(!nonce_used_elsewhere(7, 7, false), "the nonce is still free: a drop can still be resent or land");
        assert!(!nonce_used_elsewhere(8, 7, true), "our own receipt is inclusion, not replacement");
        let r = replaced(7, 8);
        assert!(r.is_final && r.state == "replaced" && !r.can_resend);
        assert!(r.message.contains("7") && r.detail.contains("nonce 7"), "{}", r.detail);
    }

    /// Round 2, finding 3: the next nonce follows the unbroken pending run
    /// from the chain nonce. N dropped (its TTL came first) while the younger
    /// N+1 still waits behind it: the next send takes N again — it fills the
    /// gap — and N+2 is refused, never queued behind the gap.
    #[test]
    fn an_older_drop_under_a_pending_successor_holds_the_next_send() {
        let (h5, h6) = (TxHash::repeat_byte(5), TxHash::repeat_byte(6));
        let queue: BTreeMap<u64, (TxHash, u64)> = [(5, (h5, 1)), (6, (h6, 1))].into_iter().collect();
        let next = next_in_sequence(5, &queue, |h| *h == h6);
        assert_eq!(next, 5, "nonce 5 dropped: the run ends there");
        assert!(gap_refusal(7, next).is_some(), "N+2 would wait behind the missing N");
        assert_eq!(gap_refusal(5, next), None, "re-signing N fills the gap");
        // Both pending: the run continues past them.
        assert_eq!(next_in_sequence(5, &queue, |_| true), 7);
        // The chain moved past both: nothing queued counts.
        assert_eq!(next_in_sequence(7, &queue, |_| true), 7);
    }

    /// R15: a self-delegating batch consumes the transaction nonce and the
    /// authorization nonce before its successor can execute.
    #[test]
    fn pending_delegated_batch_reserves_its_authorization_nonce() {
        let sender = Address::repeat_byte(0xa1);
        let batch = crate::batch_call(sender, &[(Address::repeat_byte(0xb2), U256::from(1), Default::default())]);
        assert!(batch.delegate.is_some(), "the fixture must self-authorize delegation");
        let mut b = Book::default();
        let consumed = nonce_consumption(&TxPayload::Plain(batch.encode().into())).unwrap();
        b.record(TxHash::repeat_byte(5), Sent { sender, nonce: 5, nonce_consumption: consumed, admitter: None });
        let queue = b.queue_from(sender, 5);
        assert_eq!(
            next_in_sequence(5, &queue, |_| true),
            7,
            "R15: a pending delegated batch reserves both nonce 5 and its authorization nonce 6"
        );
        assert_eq!(next_in_sequence(5, &queue, |_| false), 5, "a dropped batch reserves neither nonce");
        b.record(TxHash::repeat_byte(7), Sent { sender, nonce: 7, nonce_consumption: 1, admitter: None });
        assert_eq!(next_in_sequence(5, &b.queue_from(sender, 5), |_| true), 8, "ordinary sends continue after both batch nonces");
        // Re-signing at the batch's starting nonce replaces the entire queued
        // envelope; a normal transfer no longer reserves its authorization.
        b.record(TxHash::repeat_byte(6), Sent { sender, nonce: 5, nonce_consumption: 1, admitter: None });
        assert_eq!(next_in_sequence(5, &b.queue_from(sender, 5), |_| true), 6);
    }

    /// Bug #5 decision 5 (kept): nonces follow this process's queued txs, and
    /// a submit that would sit behind a refused or dropped nonce is refused.
    #[test]
    fn submits_never_queue_behind_a_gap() {
        let h = TxHash::repeat_byte(1);
        let one: BTreeMap<u64, (TxHash, u64)> = [(5, (h, 1))].into_iter().collect();
        assert_eq!(next_in_sequence(5, &BTreeMap::new(), |_| true), 5);
        assert_eq!(next_in_sequence(5, &one, |_| true), 6, "nonce 5 is queued: the next send takes 6");
        assert_eq!(next_in_sequence(5, &one, |_| false), 5, "nonce 5 was dropped: take it again");
        assert_eq!(gap_refusal(6, 6), None);
        assert_eq!(gap_refusal(5, 6), None, "re-signing a lower nonce replaces, it never waits");
        let why = gap_refusal(7, 6).expect("nonce 6 is missing: 7 would wait forever");
        assert!(why.contains("nonce 6") && why.contains("Nothing was sent"), "{why}");
    }

    /// The book is bounded and forgets a send once a chain fact settles it;
    /// the queue drops what the chain nonce has passed.
    #[test]
    fn the_book_is_bounded_and_forgets_settled_sends() {
        let mut b = Book::default();
        let a = Address::repeat_byte(0xa1);
        for n in 0..(MAX_QUEUED as u64 + 10) {
            let mut bytes = [0u8; 32];
            bytes[..8].copy_from_slice(&n.to_be_bytes());
            b.record(TxHash::from(bytes), Sent { sender: a, nonce: n, nonce_consumption: 1, admitter: None });
        }
        assert_eq!(b.queue_from(a, 0).len(), MAX_QUEUED, "one sender's queue stays within the node's per-sender limit");
        assert_eq!(b.queue_from(a, 70).len(), 4, "nonces below the chain nonce are on chain");
        for i in 0..(MAX_SENT as u64 + 5) {
            let mut bytes = [0xffu8; 32];
            bytes[..8].copy_from_slice(&i.to_be_bytes());
            let mut sender = [0u8; 20];
            sender[0] = (i % 40) as u8;
            b.record(TxHash::from(bytes), Sent { sender: Address::from(sender), nonce: i, nonce_consumption: 1, admitter: None });
        }
        assert!(b.sent.len() <= MAX_SENT && b.queued.len() <= MAX_SENDERS, "{} sends, {} senders", b.sent.len(), b.queued.len());
        let h = TxHash::repeat_byte(0x77);
        b.record(h, Sent { sender: a, nonce: 200, nonce_consumption: 1, admitter: None });
        assert!(b.sent.contains_key(&h));
        b.settled(&h);
        assert!(!b.sent.contains_key(&h) && b.admitter_of(&h).is_none());
        assert!(!b.queue_from(a, 0).contains_key(&200));
    }
}
