//! A bounded, chain-only search projection. Apply events in finalized order.
//!
//! Matching is exact name or app URL host, then name prefix, then all query
//! tokens in the title or description. Within each tier, complete seven-day distinct-caller counts
//! sort descending, then first publication time ascending, then the scoped id.
//! There is no publisher, category, payment, or moderation preference.
//!
//! Admission is first finalized registration while capacity is available. A
//! removed app or expired name frees its slot. Overflow is public in `info()`;
//! callers should display it. Every observed successful contract call contributes
//! to the bounded rolling history, including before a catalog declaration. Query
//! usage counts only a record's declared contracts and resolved name address,
//! deduplicating callers across that set. Retained activity is shared by apps
//! declaring the same contract. An overflow or
//! incomplete replay disables usage ordering for every result until the missing
//! interval has left the seven-day window, rather than ranking partial counts.
//! A zero-time snapshot waits for a reliable finalized timestamp before starting
//! that full window; local wall-clock time never completes the signal.
//! A query also visits at most two million usage rows. If that budget is reached,
//! all matched records use age instead and visibly report a limited usage signal.

use aether_types::{Address, B256};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_RECORDS: usize = 100_000;
pub const MAX_USAGE_PAIRS: usize = 1_000_000;
pub const MAX_RESULTS: usize = 50;
pub const USAGE_WINDOW_SECONDS: u64 = 7 * 24 * 60 * 60;
pub const MAX_ID_BYTES: usize = 192;
pub const MAX_NAME_BYTES: usize = 256;
pub const MAX_TITLE_BYTES: usize = 256;
pub const MAX_DESCRIPTION_BYTES: usize = 1_024;
pub const MAX_CATEGORY_BYTES: usize = 64;
pub const MAX_URL_BYTES: usize = 512;
pub const MAX_CONTRACTS: usize = 32;
pub const MAX_HINT_BYTES: usize = 256;
pub const MAX_QUERY_BYTES: usize = 256;
pub const MAX_QUERY_USAGE_VISITS: usize = 2_000_000;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchMetadata {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub contracts: Vec<Address>,
}

impl SearchMetadata {
    /// Only on-chain JSON hints are accepted. Remote manifests are never read.
    pub fn from_hint(hint: &str) -> Self {
        if hint.len() > MAX_HINT_BYTES {
            return Self::default();
        }
        serde_json::from_str::<Self>(hint)
            .map(Self::bounded)
            .unwrap_or_default()
    }

    fn bounded(mut self) -> Self {
        self.title = bounded_text(&self.title, MAX_TITLE_BYTES);
        self.description = bounded_text(&self.description, MAX_DESCRIPTION_BYTES);
        self.category = bounded_text(&self.category, MAX_CATEGORY_BYTES);
        // Preserve the first declared addresses before sorting: oversized input
        // cannot force an allocation proportional to its unique address count.
        self.contracts.truncate(MAX_CONTRACTS);
        self.contracts.retain(|address| *address != Address::ZERO);
        self.contracts.sort_unstable();
        self.contracts.dedup();
        self.contracts.shrink_to_fit();
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchKind {
    App,
    Name,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchRecord {
    pub id: String,
    pub kind: SearchKind,
    pub name: String,
    pub title: String,
    pub description: String,
    pub category: String,
    pub publisher: Address,
    pub url: String,
    pub content_hash: Option<B256>,
    pub contracts: Vec<Address>,
    pub address: Option<Address>,
    pub created_at: u64,
    /// The resolver's effective expiry (including any contract grace period).
    pub expires_at: Option<u64>,
}

impl SearchRecord {
    fn set_metadata(&mut self, metadata: SearchMetadata) {
        let metadata = metadata.bounded();
        self.title = if metadata.title.is_empty() {
            bounded_text(&self.name, MAX_TITLE_BYTES)
        } else {
            metadata.title
        };
        self.description = metadata.description;
        self.category = metadata.category;
        self.contracts = metadata.contracts;
    }

    fn bounded(mut self) -> Self {
        self.name = bounded_text(&self.name, MAX_NAME_BYTES);
        self.url = bounded_text(&self.url, MAX_URL_BYTES);
        let metadata = SearchMetadata {
            title: std::mem::take(&mut self.title),
            description: std::mem::take(&mut self.description),
            category: std::mem::take(&mut self.category),
            contracts: std::mem::take(&mut self.contracts),
        };
        self.set_metadata(metadata);
        self.content_hash = nonzero_hash(self.content_hash);
        self.address = self.address.filter(|address| *address != Address::ZERO);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchPending {
    Release {
        content_hash: Option<B256>,
        metadata: SearchMetadata,
        activates_at: u64,
    },
    Remove {
        activates_at: u64,
    },
}

impl SearchPending {
    pub fn activates_at(&self) -> u64 {
        match self {
            Self::Release { activates_at, .. } | Self::Remove { activates_at } => *activates_at,
        }
    }

    fn bounded(self) -> Self {
        match self {
            Self::Release {
                content_hash,
                metadata,
                activates_at,
            } => Self::Release {
                content_hash: nonzero_hash(content_hash),
                metadata: metadata.bounded(),
                activates_at,
            },
            pending => pending,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchEvent {
    Tick {
        at: u64,
    },
    UsageIncomplete {
        at: u64,
    },
    AppPublished {
        id: String,
        publisher: Address,
        name: String,
        url: String,
        content_hash: Option<B256>,
        metadata: SearchMetadata,
        created_at: u64,
    },
    AppUpdated {
        id: String,
        content_hash: Option<B256>,
        metadata: SearchMetadata,
    },
    AppReleaseQueued {
        id: String,
        content_hash: Option<B256>,
        metadata: SearchMetadata,
        activates_at: u64,
    },
    AppUnlistQueued {
        id: String,
        activates_at: u64,
    },
    PendingCancelled {
        id: String,
    },
    AppTransferred {
        id: String,
        publisher: Address,
    },
    AppRemoved {
        id: String,
    },
    NameRegistered {
        id: String,
        name: String,
        owner: Address,
        expires_at: u64,
        created_at: u64,
    },
    NameAddress {
        id: String,
        address: Address,
    },
    NameText {
        id: String,
        key: String,
        value: String,
    },
    NameTransferred {
        id: String,
        owner: Address,
    },
    NameRenewed {
        id: String,
        expires_at: u64,
    },
    ContractCalled {
        contract: Address,
        caller: Address,
        at: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResult {
    pub name: String,
    pub title: String,
    pub description: String,
    pub category: String,
    pub publisher: String,
    pub url: String,
    /// Content-addressed integrity only; this is not an endorsement.
    pub verified: bool,
    pub usage_7d: u64,
    pub usage_complete: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub query_usage_limited: bool,
    pub created_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lookalike: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchInfo {
    pub records: usize,
    pub record_limit: usize,
    pub records_saturated: bool,
    pub rejected_records: u64,
    pub pending: usize,
    pub usage_pairs: usize,
    pub usage_pair_limit: usize,
    pub usage_saturated: bool,
    pub rejected_usage_pairs: u64,
    pub usage_incomplete_until: Option<u64>,
    pub usage_complete: bool,
    pub usage_window_seconds: u64,
    pub history_complete: bool,
    pub awaiting_clock: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchCheckpoint {
    pub clock: u64,
    pub rejected_records: u64,
    pub rejected_usage_pairs: u64,
    pub usage_incomplete_until: Option<u64>,
    pub history_complete: bool,
    #[serde(default)]
    /// A usage gap was detected before a usable finalized timestamp existed.
    pub awaiting_clock: bool,
}

impl Default for SearchCheckpoint {
    fn default() -> Self {
        Self {
            clock: 0,
            rejected_records: 0,
            rejected_usage_pairs: 0,
            usage_incomplete_until: None,
            history_complete: true,
            awaiting_clock: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchRecordChange {
    pub id: String,
    pub before: Option<SearchRecord>,
    pub after: Option<SearchRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchUsageChange {
    pub contract: Address,
    pub caller: Address,
    pub before: Option<u64>,
    pub after: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchPendingChange {
    pub id: String,
    pub before: Option<SearchPending>,
    pub after: Option<SearchPending>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchDelta {
    pub records: Vec<SearchRecordChange>,
    pub usage: Vec<SearchUsageChange>,
    pub pending: Vec<SearchPendingChange>,
    pub before: SearchCheckpoint,
    pub after: SearchCheckpoint,
}

#[derive(Default)]
struct Changes {
    records: BTreeMap<String, Option<SearchRecord>>,
    usage: BTreeMap<(Address, Address), Option<u64>>,
    pending: BTreeMap<String, Option<SearchPending>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchIndex {
    records: BTreeMap<String, SearchRecord>,
    usage: BTreeMap<(Address, Address), u64>,
    usage_expiry: BTreeSet<(u64, Address, Address)>,
    name_expiry: BTreeSet<(u64, String)>,
    pending: BTreeMap<String, SearchPending>,
    pending_due: BTreeSet<(u64, String)>,
    record_limit: usize,
    usage_pair_limit: usize,
    checkpoint: SearchCheckpoint,
}

impl Default for SearchIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchIndex {
    pub fn new() -> Self {
        Self::with_limits(MAX_RECORDS, MAX_USAGE_PAIRS)
    }

    /// Lower limits are useful for owned devnets; they are always public in info.
    pub fn with_limits(record_limit: usize, usage_pair_limit: usize) -> Self {
        Self {
            records: BTreeMap::new(),
            usage: BTreeMap::new(),
            usage_expiry: BTreeSet::new(),
            name_expiry: BTreeSet::new(),
            pending: BTreeMap::new(),
            pending_due: BTreeSet::new(),
            record_limit: record_limit.min(MAX_RECORDS),
            usage_pair_limit: usage_pair_limit.min(MAX_USAGE_PAIRS),
            checkpoint: SearchCheckpoint::default(),
        }
    }

    pub fn rebuild(events: impl IntoIterator<Item = SearchEvent>) -> Self {
        let mut index = Self::new();
        for event in events {
            index.apply(event);
        }
        index
    }

    /// Restore bounded durable rows; expiry indexes are derived locally.
    pub fn from_rows(
        records: impl IntoIterator<Item = SearchRecord>,
        usage: impl IntoIterator<Item = (Address, Address, u64)>,
        pending: impl IntoIterator<Item = (String, SearchPending)>,
        checkpoint: SearchCheckpoint,
    ) -> Self {
        let mut index = Self::new();
        index.checkpoint = checkpoint;
        let mut changes = None;
        for record in records {
            if index.admit(&record.id) {
                let id = record.id.clone();
                index.set_record(&id, Some(record.bounded()), &mut changes);
            }
        }
        for (contract, caller, at) in usage {
            index.record_call(contract, caller, at, &mut changes);
        }
        for (id, pending) in pending {
            if index.is_kind(&id, SearchKind::App) {
                index.set_pending(&id, Some(pending.bounded()), &mut changes);
            }
        }
        index.advance_inner(index.checkpoint.clock, &mut changes);
        index
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
    pub fn record(&self, id: &str) -> Option<&SearchRecord> {
        self.records.get(id)
    }
    pub fn records(&self) -> impl Iterator<Item = &SearchRecord> {
        self.records.values()
    }
    pub fn usage_rows(&self) -> impl Iterator<Item = (Address, Address, u64)> + '_ {
        self.usage
            .iter()
            .map(|((contract, caller), at)| (*contract, *caller, *at))
    }
    pub fn pending_rows(&self) -> impl Iterator<Item = (&String, &SearchPending)> {
        self.pending.iter()
    }
    pub fn checkpoint(&self) -> SearchCheckpoint {
        self.checkpoint.clone()
    }

    pub fn mark_history_incomplete(&mut self) {
        self.checkpoint.history_complete = false;
        self.mark_usage_incomplete();
    }

    pub fn mark_usage_incomplete(&mut self) {
        self.mark_usage_incomplete_at(self.checkpoint.clock);
    }

    pub fn info(&self) -> SearchInfo {
        let usage_complete = self.usage_complete(self.checkpoint.clock);
        SearchInfo {
            records: self.records.len(),
            record_limit: self.record_limit,
            records_saturated: self.checkpoint.rejected_records != 0,
            rejected_records: self.checkpoint.rejected_records,
            pending: self.pending.len(),
            usage_pairs: self.usage.len(),
            usage_pair_limit: self.usage_pair_limit,
            usage_saturated: !usage_complete && self.checkpoint.rejected_usage_pairs != 0,
            rejected_usage_pairs: self.checkpoint.rejected_usage_pairs,
            usage_incomplete_until: self.checkpoint.usage_incomplete_until,
            usage_complete,
            usage_window_seconds: USAGE_WINDOW_SECONDS,
            history_complete: self.checkpoint.history_complete,
            awaiting_clock: self.checkpoint.awaiting_clock,
        }
    }

    pub fn apply(&mut self, event: SearchEvent) {
        self.apply_inner(&event, &mut None);
    }

    pub fn advance(&mut self, at: u64) {
        self.apply(SearchEvent::Tick { at });
    }

    /// Captures only rows touched by this finalized batch, including expiration.
    pub fn apply_batch(&mut self, events: &[SearchEvent]) -> SearchDelta {
        let before = self.checkpoint();
        let mut changes = Some(Changes::default());
        for event in events {
            self.apply_inner(event, &mut changes);
        }
        let changes = changes.expect("batch tracking exists");
        SearchDelta {
            records: changes
                .records
                .into_iter()
                .map(|(id, before)| SearchRecordChange {
                    after: self.records.get(&id).cloned(),
                    id,
                    before,
                })
                .collect(),
            usage: changes
                .usage
                .into_iter()
                .map(|((contract, caller), before)| SearchUsageChange {
                    after: self.usage.get(&(contract, caller)).copied(),
                    contract,
                    caller,
                    before,
                })
                .collect(),
            pending: changes
                .pending
                .into_iter()
                .map(|(id, before)| SearchPendingChange {
                    after: self.pending.get(&id).cloned(),
                    id,
                    before,
                })
                .collect(),
            before,
            after: self.checkpoint(),
        }
    }

    /// Undo the latest batch after an atomic durable-write failure.
    pub fn undo(&mut self, delta: &SearchDelta) {
        let mut untracked = None;
        // Catalog rows and rolling usage rows are restored independently.
        for change in &delta.records {
            if let Some(record) = self.records.remove(&change.id) {
                if let Some(expires) = record.expires_at {
                    self.name_expiry.remove(&(expires, change.id.clone()));
                }
            }
            if let Some(record) = &change.before {
                if let Some(expires) = record.expires_at {
                    self.name_expiry.insert((expires, change.id.clone()));
                }
                self.records.insert(change.id.clone(), record.clone());
            }
        }
        for change in &delta.pending {
            self.set_pending(&change.id, change.before.clone(), &mut untracked);
        }
        for change in &delta.usage {
            self.set_usage(
                (change.contract, change.caller),
                change.before,
                &mut untracked,
            );
        }
        self.checkpoint = delta.before.clone();
    }

    pub fn search(&self, query: &str, limit: usize, now: u64) -> Vec<SearchResult> {
        self.search_with_budget(query, limit, now, MAX_QUERY_USAGE_VISITS)
    }

    #[cfg(feature = "test-seam")]
    pub fn search_with_usage_budget(
        &self,
        query: &str,
        limit: usize,
        now: u64,
        usage_budget: usize,
    ) -> Vec<SearchResult> {
        self.search_with_budget(query, limit, now, usage_budget.min(MAX_QUERY_USAGE_VISITS))
    }

    fn search_with_budget(
        &self,
        query: &str,
        limit: usize,
        now: u64,
        mut usage_budget: usize,
    ) -> Vec<SearchResult> {
        let query = bounded_text(query.trim(), MAX_QUERY_BYTES);
        if query.is_empty() || limit == 0 {
            return Vec::new();
        }
        let name_query = comparable_name(&query);
        let tokens = tokens(&query.to_lowercase())
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        let mut usage_complete = self.usage_complete(now);
        let mut query_usage_limited = false;
        let mut ranked = Vec::new();
        for record in self.records.values() {
            if record.expires_at.is_some_and(|expires| now >= expires) {
                continue;
            }
            let name = comparable_name(&record.name);
            let tier = if !name_query.is_empty()
                && (name == name_query || comparable_name(&record.url) == name_query)
            {
                0
            } else if !name_query.is_empty() && name.starts_with(&name_query) {
                1
            } else if !tokens.is_empty() && matches_tokens(record, &tokens) {
                2
            } else {
                continue;
            };
            let usage = if usage_complete {
                if let Some(count) = self.usage_count(record, now, &mut usage_budget) {
                    count
                } else {
                    usage_complete = false;
                    query_usage_limited = true;
                    0
                }
            } else {
                0
            };
            ranked.push((tier, usage, record));
        }
        if query_usage_limited {
            for (_, usage, _) in &mut ranked {
                *usage = 0;
            }
        }
        ranked.sort_unstable_by(|(at, au, ar), (bt, bu, br)| {
            at.cmp(bt)
                .then_with(|| bu.cmp(au))
                .then_with(|| ar.created_at.cmp(&br.created_at))
                .then_with(|| ar.id.cmp(&br.id))
        });

        let mut skeletons = BTreeMap::<String, SkeletonOwners>::new();
        let mut results = Vec::with_capacity(limit.min(MAX_RESULTS).min(ranked.len()));
        for (_, usage, record) in ranked.into_iter().take(limit.min(MAX_RESULTS)) {
            let name_key = comparable_name(&record.name);
            let keys = [display_name(&record.name), record.title.clone()]
                .map(|text| crate::search_unicode::skeleton(&text));
            let lookalike = keys
                .iter()
                .find_map(|key| skeletons.get(key)?.different(&name_key));
            for key in keys {
                skeletons
                    .entry(key)
                    .or_default()
                    .insert(&name_key, &record.name);
            }
            results.push(SearchResult {
                name: record.name.clone(),
                title: record.title.clone(),
                description: record.description.clone(),
                category: record.category.clone(),
                publisher: format!("{:#x}", record.publisher),
                url: record.url.clone(),
                verified: nonzero_hash(record.content_hash).is_some(),
                usage_7d: usage,
                usage_complete,
                query_usage_limited,
                created_at: record.created_at,
                lookalike,
            });
        }
        results
    }

    /// Sum of retained variable-size payloads; all individual fields are capped.
    pub fn retained_payload_bytes(&self) -> usize {
        self.records
            .values()
            .map(|record| {
                record.id.len()
                    + record.name.len()
                    + record.title.len()
                    + record.description.len()
                    + record.category.len()
                    + record.url.len()
                    + record.contracts.len() * 20
            })
            .sum::<usize>()
            + self
                .pending
                .iter()
                .map(|(id, pending)| {
                    id.len()
                        + match pending {
                            SearchPending::Release { metadata, .. } => {
                                metadata.title.len()
                                    + metadata.description.len()
                                    + metadata.category.len()
                                    + metadata.contracts.len() * 20
                            }
                            SearchPending::Remove { .. } => 0,
                        }
                })
                .sum::<usize>()
    }

    fn apply_inner(&mut self, event: &SearchEvent, changes: &mut Option<Changes>) {
        match event {
            SearchEvent::Tick { at } => self.advance_inner(*at, changes),
            SearchEvent::UsageIncomplete { at } => {
                self.advance_inner(*at, changes);
                self.mark_usage_incomplete_at(*at);
            }
            SearchEvent::AppPublished {
                id,
                publisher,
                name,
                url,
                content_hash,
                metadata,
                created_at,
            } => {
                self.advance_inner(*created_at, changes);
                if !self.admit(id) || self.records.contains_key(id) {
                    return;
                }
                let mut record = SearchRecord {
                    id: id.clone(),
                    kind: SearchKind::App,
                    name: bounded_text(&display_name(name), MAX_NAME_BYTES),
                    title: String::new(),
                    description: String::new(),
                    category: String::new(),
                    publisher: *publisher,
                    url: bounded_text(url, MAX_URL_BYTES),
                    content_hash: nonzero_hash(*content_hash),
                    contracts: Vec::new(),
                    address: None,
                    created_at: *created_at,
                    expires_at: None,
                };
                record.set_metadata(metadata.clone());
                self.set_record(id, Some(record), changes);
            }
            SearchEvent::AppUpdated {
                id,
                content_hash,
                metadata,
            } => {
                self.update_app(id, *content_hash, metadata.clone(), changes);
                if self.is_kind(id, SearchKind::App) {
                    self.set_pending(id, None, changes);
                }
            }
            SearchEvent::AppReleaseQueued {
                id,
                content_hash,
                metadata,
                activates_at,
            } => {
                if self.is_kind(id, SearchKind::App) {
                    self.set_pending(
                        id,
                        Some(SearchPending::Release {
                            content_hash: nonzero_hash(*content_hash),
                            metadata: metadata.clone().bounded(),
                            activates_at: *activates_at,
                        }),
                        changes,
                    );
                    self.activate_pending(changes);
                }
            }
            SearchEvent::AppUnlistQueued { id, activates_at } => {
                if self.is_kind(id, SearchKind::App) {
                    self.set_pending(
                        id,
                        Some(SearchPending::Remove {
                            activates_at: *activates_at,
                        }),
                        changes,
                    );
                    self.activate_pending(changes);
                }
            }
            SearchEvent::PendingCancelled { id } => {
                if self
                    .pending
                    .get(id)
                    .is_some_and(|pending| self.checkpoint.clock < pending.activates_at())
                {
                    self.set_pending(id, None, changes);
                }
            }
            SearchEvent::AppTransferred { id, publisher } => {
                if let Some(mut record) = self
                    .records
                    .get(id)
                    .filter(|record| record.kind == SearchKind::App)
                    .cloned()
                {
                    record.publisher = *publisher;
                    self.set_record(id, Some(record), changes);
                }
            }
            SearchEvent::AppRemoved { id } => {
                if self.is_kind(id, SearchKind::App) {
                    self.set_record(id, None, changes);
                    self.set_pending(id, None, changes);
                }
            }
            SearchEvent::NameRegistered {
                id,
                name,
                owner,
                expires_at,
                created_at,
            } => {
                self.advance_inner(*created_at, changes);
                if *expires_at <= self.checkpoint.clock
                    || !self.admit(id)
                    || self
                        .records
                        .get(id)
                        .is_some_and(|record| record.kind != SearchKind::Name)
                {
                    return;
                }
                let base = display_name(name);
                let base = bounded_text(&base, MAX_NAME_BYTES - 4);
                let name = format!("{base}.sea");
                let record = SearchRecord {
                    id: id.clone(),
                    kind: SearchKind::Name,
                    title: bounded_text(&name, MAX_TITLE_BYTES),
                    url: format!("sea://{name}/"),
                    name,
                    description: String::new(),
                    category: String::new(),
                    publisher: *owner,
                    content_hash: None,
                    contracts: Vec::new(),
                    address: None,
                    created_at: *created_at,
                    expires_at: Some(*expires_at),
                };
                self.set_record(id, Some(record), changes);
            }
            SearchEvent::NameAddress { id, address } => {
                if let Some(mut record) = self
                    .records
                    .get(id)
                    .filter(|record| record.kind == SearchKind::Name)
                    .cloned()
                {
                    record.address = (*address != Address::ZERO).then_some(*address);
                    self.set_record(id, Some(record), changes);
                }
            }
            SearchEvent::NameText { id, key, value } => {
                if let Some(mut record) = self
                    .records
                    .get(id)
                    .filter(|record| record.kind == SearchKind::Name)
                    .cloned()
                {
                    match key.as_str() {
                        "title" => {
                            record.title = if value.is_empty() {
                                bounded_text(&record.name, MAX_TITLE_BYTES)
                            } else {
                                bounded_text(value, MAX_TITLE_BYTES)
                            }
                        }
                        "description" => {
                            record.description = bounded_text(value, MAX_DESCRIPTION_BYTES)
                        }
                        "category" => record.category = bounded_text(value, MAX_CATEGORY_BYTES),
                        "content-hash" | "contenthash" => {
                            record.content_hash = value
                                .parse::<B256>()
                                .ok()
                                .and_then(|hash| nonzero_hash(Some(hash)))
                        }
                        "contracts" if value.len() <= MAX_HINT_BYTES => {
                            record.contracts = serde_json::from_str::<Vec<Address>>(value)
                                .ok()
                                .map(|contracts| {
                                    SearchMetadata {
                                        contracts,
                                        ..SearchMetadata::default()
                                    }
                                    .bounded()
                                    .contracts
                                })
                                .unwrap_or_default();
                        }
                        _ => return,
                    }
                    self.set_record(id, Some(record), changes);
                }
            }
            SearchEvent::NameTransferred { id, owner } => {
                if let Some(mut record) = self
                    .records
                    .get(id)
                    .filter(|record| record.kind == SearchKind::Name)
                    .cloned()
                {
                    record.publisher = *owner;
                    self.set_record(id, Some(record), changes);
                }
            }
            SearchEvent::NameRenewed { id, expires_at } => {
                if let Some(mut record) = self
                    .records
                    .get(id)
                    .filter(|record| record.kind == SearchKind::Name)
                    .cloned()
                {
                    record.expires_at = Some(*expires_at);
                    self.set_record(id, Some(record), changes);
                }
            }
            SearchEvent::ContractCalled {
                contract,
                caller,
                at,
            } => {
                self.advance_inner(*at, changes);
                self.record_call(*contract, *caller, *at, changes);
            }
        }
    }

    fn is_kind(&self, id: &str, kind: SearchKind) -> bool {
        self.records
            .get(id)
            .is_some_and(|record| record.kind == kind)
    }

    fn admit(&mut self, id: &str) -> bool {
        if id.is_empty()
            || id.len() > MAX_ID_BYTES
            || (!self.records.contains_key(id) && self.records.len() >= self.record_limit)
        {
            self.checkpoint.rejected_records = self.checkpoint.rejected_records.saturating_add(1);
            false
        } else {
            true
        }
    }

    fn set_record(
        &mut self,
        id: &str,
        record: Option<SearchRecord>,
        changes: &mut Option<Changes>,
    ) {
        if let Some(changes) = changes.as_mut() {
            changes
                .records
                .entry(id.to_owned())
                .or_insert_with(|| self.records.get(id).cloned());
        }
        let previous = self.records.remove(id);
        if let Some(expires) = previous.as_ref().and_then(|record| record.expires_at) {
            self.name_expiry.remove(&(expires, id.to_owned()));
        }
        if let Some(record) = record {
            if let Some(expires) = record.expires_at {
                self.name_expiry.insert((expires, id.to_owned()));
            }
            self.records.insert(id.to_owned(), record);
        }
    }

    fn set_pending(
        &mut self,
        id: &str,
        pending: Option<SearchPending>,
        changes: &mut Option<Changes>,
    ) {
        if let Some(changes) = changes.as_mut() {
            changes
                .pending
                .entry(id.to_owned())
                .or_insert_with(|| self.pending.get(id).cloned());
        }
        if let Some(previous) = self.pending.remove(id) {
            self.pending_due
                .remove(&(previous.activates_at(), id.to_owned()));
        }
        if let Some(pending) = pending {
            self.pending_due
                .insert((pending.activates_at(), id.to_owned()));
            self.pending.insert(id.to_owned(), pending);
        }
    }

    fn set_usage(
        &mut self,
        pair: (Address, Address),
        at: Option<u64>,
        changes: &mut Option<Changes>,
    ) {
        if let Some(changes) = changes.as_mut() {
            changes
                .usage
                .entry(pair)
                .or_insert_with(|| self.usage.get(&pair).copied());
        }
        if let Some(previous) = self.usage.remove(&pair) {
            self.usage_expiry.remove(&(previous, pair.0, pair.1));
        }
        if let Some(at) = at {
            self.usage.insert(pair, at);
            self.usage_expiry.insert((at, pair.0, pair.1));
        }
    }

    fn update_app(
        &mut self,
        id: &str,
        content_hash: Option<B256>,
        metadata: SearchMetadata,
        changes: &mut Option<Changes>,
    ) {
        if let Some(mut record) = self
            .records
            .get(id)
            .filter(|record| record.kind == SearchKind::App)
            .cloned()
        {
            record.content_hash = nonzero_hash(content_hash);
            record.set_metadata(metadata);
            self.set_record(id, Some(record), changes);
        }
    }

    fn activate_pending(&mut self, changes: &mut Option<Changes>) {
        while let Some((at, id)) = self.pending_due.first().cloned() {
            if at > self.checkpoint.clock {
                break;
            }
            let pending = self.pending.get(&id).cloned();
            self.set_pending(&id, None, changes);
            match pending {
                Some(SearchPending::Release {
                    content_hash,
                    metadata,
                    ..
                }) => self.update_app(&id, content_hash, metadata, changes),
                Some(SearchPending::Remove { .. }) => self.set_record(&id, None, changes),
                None => {
                    self.pending_due.remove(&(at, id));
                }
            }
        }
    }

    fn advance_inner(&mut self, at: u64, changes: &mut Option<Changes>) {
        self.checkpoint.clock = self.checkpoint.clock.max(at);
        if at > 0 && self.checkpoint.awaiting_clock {
            self.mark_usage_incomplete_at(self.checkpoint.clock);
        }
        while let Some((expires, id)) = self.name_expiry.first().cloned() {
            if expires > self.checkpoint.clock {
                break;
            }
            self.set_record(&id, None, changes);
        }
        self.activate_pending(changes);
        while let Some((at, contract, caller)) = self.usage_expiry.first().copied() {
            if in_window(at, self.checkpoint.clock) {
                break;
            }
            self.set_usage((contract, caller), None, changes);
        }
    }

    fn record_call(
        &mut self,
        contract: Address,
        caller: Address,
        at: u64,
        changes: &mut Option<Changes>,
    ) {
        if !in_window(at, self.checkpoint.clock) {
            return;
        }
        let pair = (contract, caller);
        if let Some(previous) = self.usage.get(&pair) {
            if at <= *previous {
                return;
            }
        } else if self.usage.len() >= self.usage_pair_limit {
            self.checkpoint.rejected_usage_pairs =
                self.checkpoint.rejected_usage_pairs.saturating_add(1);
            self.mark_usage_incomplete_at(at);
            return;
        }
        self.set_usage(pair, Some(at), changes);
    }

    fn mark_usage_incomplete_at(&mut self, at: u64) {
        if at == 0 {
            self.checkpoint.awaiting_clock = true;
            return;
        }
        self.checkpoint.awaiting_clock = false;
        let until = at.saturating_add(USAGE_WINDOW_SECONDS);
        self.checkpoint.usage_incomplete_until = Some(
            self.checkpoint
                .usage_incomplete_until
                .unwrap_or(0)
                .max(until),
        );
    }

    fn usage_complete(&self, now: u64) -> bool {
        !self.checkpoint.awaiting_clock
            && self
                .checkpoint
                .usage_incomplete_until
                .is_none_or(|until| now >= until)
    }

    fn usage_count(&self, record: &SearchRecord, now: u64, budget: &mut usize) -> Option<u64> {
        let mut callers = BTreeSet::new();
        for contract in record_contracts(record) {
            for ((_, caller), at) in self
                .usage
                .range((contract, Address::ZERO)..=(contract, Address::repeat_byte(0xff)))
            {
                if *budget == 0 {
                    return None;
                }
                *budget -= 1;
                if in_window(*at, now) {
                    callers.insert(*caller);
                }
            }
        }
        Some(callers.len() as u64)
    }
}

#[derive(Default)]
struct SkeletonOwners {
    first: Option<(String, String)>,
    other: Option<(String, String)>,
}

impl SkeletonOwners {
    fn insert(&mut self, key: &str, display: &str) {
        if let Some((first, _)) = &self.first {
            if first != key && self.other.is_none() {
                self.other = Some((key.to_owned(), display.to_owned()));
            }
        } else {
            self.first = Some((key.to_owned(), display.to_owned()));
        }
    }

    fn different(&self, key: &str) -> Option<String> {
        self.first
            .iter()
            .chain(self.other.iter())
            .find_map(|(owner, display)| (owner != key).then(|| display.clone()))
    }
}

fn bounded_text(text: &str, max: usize) -> String {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end]
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

fn nonzero_hash(hash: Option<B256>) -> Option<B256> {
    hash.filter(|hash| *hash != B256::ZERO)
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn display_name(name: &str) -> String {
    let name = name.trim();
    let name = if name
        .get(..6)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("sea://"))
    {
        &name[6..]
    } else {
        name
    };
    let name = name
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .trim_end_matches('.');
    if name.len() >= 4
        && name
            .get(name.len() - 4..)
            .is_some_and(|suffix| suffix.eq_ignore_ascii_case(".sea"))
    {
        name[..name.len() - 4].to_owned()
    } else {
        name.to_owned()
    }
}

fn comparable_name(name: &str) -> String {
    display_name(name).to_lowercase()
}

fn tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
}

fn matches_tokens(record: &SearchRecord, query: &BTreeSet<String>) -> bool {
    let title = record.title.to_lowercase();
    let description = record.description.to_lowercase();
    let words = tokens(&title)
        .chain(tokens(&description))
        .collect::<BTreeSet<_>>();
    query.iter().all(|word| words.contains(word.as_str()))
}

fn record_contracts(record: &SearchRecord) -> BTreeSet<Address> {
    record
        .contracts
        .iter()
        .copied()
        .chain(record.address)
        .filter(|address| *address != Address::ZERO)
        .collect()
}

/// The interval is (now - seven days, now]; the oldest boundary has expired.
fn in_window(at: u64, now: u64) -> bool {
    at <= now && now - at < USAGE_WINDOW_SECONDS
}
