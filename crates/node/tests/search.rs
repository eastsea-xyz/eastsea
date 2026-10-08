use aether_node::search::{
    SearchEvent, SearchIndex, SearchMetadata, SearchResult, MAX_CATEGORY_BYTES, MAX_CONTRACTS,
    MAX_DESCRIPTION_BYTES, MAX_HINT_BYTES, MAX_ID_BYTES, MAX_NAME_BYTES, MAX_RECORDS, MAX_RESULTS,
    MAX_TITLE_BYTES, MAX_URL_BYTES, USAGE_WINDOW_SECONDS,
};
use aether_types::{Address, B256};

fn address(n: u64) -> Address {
    let mut bytes = [0; 20];
    bytes[12..].copy_from_slice(&n.to_be_bytes());
    Address::from(bytes)
}

fn app(id: &str, name: &str, at: u64, contracts: &[Address]) -> SearchEvent {
    SearchEvent::AppPublished {
        id: id.to_owned(),
        publisher: address(1),
        name: name.to_owned(),
        url: format!("sea://{id}/"),
        content_hash: Some(B256::repeat_byte(1)),
        metadata: SearchMetadata {
            title: format!("{name} shared fixture"),
            description: "chain application and wallet tools".to_owned(),
            category: "tools".to_owned(),
            contracts: contracts.to_vec(),
        },
        created_at: at,
    }
}

fn name(id: &str, name: &str, at: u64, expires: u64) -> SearchEvent {
    SearchEvent::NameRegistered {
        id: id.to_owned(),
        name: name.to_owned(),
        owner: address(2),
        expires_at: expires,
        created_at: at,
    }
}

fn call(contract: Address, caller: u64, at: u64) -> SearchEvent {
    SearchEvent::ContractCalled {
        contract,
        caller: address(caller),
        at,
    }
}

fn names(rows: &[SearchResult]) -> Vec<&str> {
    rows.iter().map(|row| row.name.as_str()).collect()
}

#[test]
fn incremental_and_deterministic_rebuild_are_equal() {
    let events = vec![
        app("apps/1", "harbor", 1, &[address(10), address(11)]),
        name("names/1", "store.harbor.sea", 2, 200),
        SearchEvent::NameAddress {
            id: "names/1".into(),
            address: address(12),
        },
        SearchEvent::NameText {
            id: "names/1".into(),
            key: "description".into(),
            value: "shared fixture".into(),
        },
        SearchEvent::NameText {
            id: "names/1".into(),
            key: "content-hash".into(),
            value: format!("{:#x}", B256::repeat_byte(2)),
        },
        call(address(10), 20, 3),
        call(address(11), 20, 4),
        call(address(12), 21, 5),
        SearchEvent::AppReleaseQueued {
            id: "apps/1".into(),
            content_hash: Some(B256::repeat_byte(3)),
            metadata: SearchMetadata {
                title: "harbor shared fixture".into(),
                contracts: vec![address(10)],
                ..Default::default()
            },
            activates_at: 10,
        },
        SearchEvent::AppTransferred {
            id: "apps/1".into(),
            publisher: address(3),
        },
        SearchEvent::NameTransferred {
            id: "names/1".into(),
            owner: address(4),
        },
        SearchEvent::NameRenewed {
            id: "names/1".into(),
            expires_at: 300,
        },
        SearchEvent::Tick { at: 10 },
        app("apps/removed", "removed", 11, &[]),
        SearchEvent::AppRemoved {
            id: "apps/removed".into(),
        },
        SearchEvent::Tick { at: 300 },
    ];
    let mut incremental = SearchIndex::new();
    for batch in events.chunks(4) {
        incremental.apply_batch(batch);
    }
    let rebuilt = SearchIndex::rebuild(events.clone());
    assert_eq!(incremental, rebuilt);
    assert_eq!(rebuilt, SearchIndex::rebuild(events));
    assert_eq!(
        incremental.search("fixture", 50, 300),
        rebuilt.search("fixture", 50, 300)
    );
    assert_eq!(names(&rebuilt.search("fixture", 50, 300)), ["harbor"]);
    let record = rebuilt.record("apps/1").unwrap();
    assert_eq!(record.publisher, address(3));
    assert_eq!(record.created_at, 1);
    assert_eq!(record.description, "");
}

#[test]
fn exact_then_prefix_then_whole_token_match() {
    let mut index = SearchIndex::new();
    index.apply(app("apps/token", "token", 1, &[address(10)]));
    index.apply(SearchEvent::AppUpdated {
        id: "apps/token".into(),
        content_hash: None,
        metadata: SearchMetadata {
            title: "wallet".into(),
            contracts: vec![address(10)],
            ..Default::default()
        },
    });
    index.apply(app("apps/prefix", "wallet-tools", 2, &[address(11)]));
    index.apply(app("apps/exact", "wallet", 3, &[]));
    for caller in 20..30 {
        index.apply(call(address(10), caller, 4));
    }
    index.apply(call(address(11), 30, 4));
    assert_eq!(
        names(&index.search("wallet", 50, 4)),
        ["wallet", "wallet-tools", "token"]
    );
    assert_eq!(index.search("wallet", 50, 4)[2].usage_7d, 10);
    assert!(index.search("wall tools", 50, 4).is_empty());
    assert_eq!(
        names(&index.search("APPLICATION WALLET", 50, 4)),
        ["wallet-tools", "wallet"]
    );
}

#[test]
fn usage_then_older_age_then_scoped_id_breaks_ties() {
    let mut index = SearchIndex::new();
    index.apply(app("apps/z", "older", 1, &[]));
    index.apply(app("apps/b", "later-b", 2, &[]));
    index.apply(app("apps/a", "later-a", 2, &[]));
    index.apply(app(
        "apps/popular",
        "popular",
        3,
        &[address(10), address(11)],
    ));
    index.apply(call(address(10), 20, 4));
    index.apply(call(address(11), 20, 5));
    index.apply(call(address(11), 21, 6));
    let rows = index.search("fixture", 50, 6);
    assert_eq!(names(&rows), ["popular", "older", "later-a", "later-b"]);
    assert_eq!(rows[0].usage_7d, 2);
    assert!(rows.iter().all(|row| row.usage_complete));
}

#[test]
fn sea_urls_and_subdomains_match_case_insensitively() {
    let mut index = SearchIndex::new();
    index.apply(name("names/1", "shop.harbor.sea", 1, 100));
    let rows = index.search("  SEA://SHOP.HARBOR.SEA/path?ignored  ", 50, 2);
    assert_eq!(names(&rows), ["shop.harbor.sea"]);
    assert_eq!(rows[0].url, "sea://shop.harbor.sea/");
    assert!(!rows[0].verified);
    assert_eq!(names(&index.search("shop.har", 50, 2)), ["shop.harbor.sea"]);
    assert_eq!(
        names(&index.search("harbor.sea", 50, 2)),
        ["shop.harbor.sea"]
    );
    // Non-ASCII final bytes must not panic while testing for an ASCII suffix.
    assert!(index.search("항구", 50, 2).is_empty());
}

#[test]
fn exact_app_key_and_sea_url_lookup_rank_ahead_of_name_prefixes() {
    let mut index = SearchIndex::new();
    let app_key = "a".repeat(52);
    index.apply(app("apps/decoy", &format!("{app_key}-tools"), 1, &[]));
    index.apply(SearchEvent::AppPublished {
        id: "apps/target".into(),
        publisher: address(1),
        name: "harbor".into(),
        url: format!("sea://{app_key}/"),
        content_hash: Some(B256::repeat_byte(1)),
        metadata: SearchMetadata::default(),
        created_at: 2,
    });
    for query in [
        app_key.clone(),
        format!("sea://{app_key}/"),
        format!("SEA://{}/view?ignored", app_key.to_uppercase()),
    ] {
        let rows = index.search(&query, 1, 3);
        assert_eq!(names(&rows), ["harbor"], "{query}");
        assert_eq!(rows[0].url, format!("sea://{app_key}/"));
    }
}

#[test]
fn content_hash_means_integrity_only_and_zero_is_absent() {
    let mut index = SearchIndex::new();
    index.apply(app("apps/1", "app", 1, &[]));
    assert!(index.search("app", 1, 2)[0].verified);
    index.apply(SearchEvent::AppUpdated {
        id: "apps/1".into(),
        content_hash: Some(B256::ZERO),
        metadata: SearchMetadata::default(),
    });
    assert!(!index.search("app", 1, 2)[0].verified);
    assert_eq!(index.search("app", 1, 2)[0].title, "app");
}

#[test]
fn confusable_names_warn_only_the_lower_ranked_distinct_name() {
    for (first, second) in [("m", "rn"), ("l", "1"), ("paypal", "pаypal")] {
        let mut index = SearchIndex::new();
        index.apply(app("apps/first", first, 1, &[]));
        index.apply(app("apps/second", second, 2, &[]));
        let rows = index.search("fixture", 50, 3);
        assert_eq!(rows[0].lookalike, None, "{first}/{second}");
        assert_eq!(
            rows[1].lookalike.as_deref(),
            Some(first),
            "{first}/{second}"
        );
        assert_eq!(index.search("fixture", 1, 3)[0].lookalike, None);
        assert_eq!(index.search("fixture", 2, 3), rows);
    }
}

#[test]
fn unicode_display_titles_and_zero_uppercase_o_are_compared() {
    let mut index = SearchIndex::new();
    for (id, slug, title, at) in [
        ("apps/first", "first", "PayPal TokenO", 1),
        ("apps/second", "second", "PayPal Token0", 2),
    ] {
        index.apply(app(id, slug, at, &[]));
        index.apply(SearchEvent::AppUpdated {
            id: id.into(),
            content_hash: None,
            metadata: SearchMetadata {
                title: title.into(),
                description: "shared fixture".into(),
                ..Default::default()
            },
        });
    }
    let rows = index.search("fixture", 50, 3);
    assert_eq!(rows[0].lookalike, None);
    assert_eq!(rows[1].lookalike.as_deref(), Some("first"));
}

#[test]
fn same_name_duplicates_are_not_a_lookalike_but_other_names_remain_visible() {
    let mut index = SearchIndex::new();
    index.apply(app("apps/a", "m", 1, &[]));
    index.apply(app("apps/b", "rn", 2, &[]));
    index.apply(app("apps/c", "m", 3, &[]));
    let rows = index.search("fixture", 50, 4);
    assert_eq!(rows[1].lookalike.as_deref(), Some("m"));
    assert_eq!(rows[2].lookalike.as_deref(), Some("rn"));
    let mut duplicates = SearchIndex::new();
    duplicates.apply(app("apps/a", "m", 1, &[]));
    duplicates.apply(app("apps/b", "m", 2, &[]));
    assert!(duplicates
        .search("m", 50, 3)
        .iter()
        .all(|row| row.lookalike.is_none()));
}

#[test]
fn usage_is_a_sliding_last_seen_window_and_deduplicates_contracts() {
    let mut index = SearchIndex::new();
    index.apply(app("apps/1", "app", 1, &[address(10), address(11)]));
    index.apply(call(address(10), 20, 2));
    index.apply(call(address(10), 20, 3));
    index.apply(call(address(11), 20, 4));
    index.apply(call(address(10), 21, 5));
    assert_eq!(index.info().usage_pairs, 3);
    assert_eq!(index.search("app", 1, 6)[0].usage_7d, 2);
    index.advance(USAGE_WINDOW_SECONDS + 4);
    assert_eq!(index.info().usage_pairs, 1);
    assert_eq!(
        index.search("app", 1, USAGE_WINDOW_SECONDS + 4)[0].usage_7d,
        1
    );
    index.advance(USAGE_WINDOW_SECONDS + 5);
    assert_eq!(index.info().usage_pairs, 0);
    assert_eq!(
        index.search("app", 1, USAGE_WINDOW_SECONDS + 5)[0].usage_7d,
        0
    );
}

#[test]
fn all_observed_calls_are_retained_independently_of_catalog_bindings() {
    let mut index = SearchIndex::with_limits(10, 32);
    for caller in 1..20 {
        index.apply(call(address(10), caller, 1));
    }
    assert_eq!(index.info().usage_pairs, 19);
    assert_eq!(index.info().rejected_usage_pairs, 0);
    index.apply(app("apps/1", "app", 2, &[address(10)]));
    index.apply(call(address(10), 20, 3));
    assert_eq!(index.search("app", 1, 3)[0].usage_7d, 20);
    assert!(index.search("app", 1, 3)[0].usage_complete);
    index.apply(SearchEvent::AppUpdated {
        id: "apps/1".into(),
        content_hash: None,
        metadata: SearchMetadata {
            contracts: vec![address(11)],
            ..Default::default()
        },
    });
    assert_eq!(index.info().usage_pairs, 20);
    assert_eq!(index.search("app", 1, 3)[0].usage_7d, 0);
    index.apply(call(address(10), 21, 4));
    assert_eq!(index.info().usage_pairs, 21);
    assert_eq!(index.search("app", 1, 4)[0].usage_7d, 0);
    index.apply(call(address(11), 22, 5));
    assert_eq!(index.info().usage_pairs, 22);
    assert_eq!(index.search("app", 1, 5)[0].usage_7d, 1);
    index.apply(SearchEvent::AppRemoved {
        id: "apps/1".into(),
    });
    assert_eq!(index.info().usage_pairs, 22);
}

#[test]
fn late_publication_uses_prior_seven_day_contract_callers_after_durable_reload() {
    let mut index = SearchIndex::new();
    index.apply(call(address(10), 20, 10));
    index.apply(call(address(10), 21, 11));
    index.apply(call(address(10), 21, 12));
    index.apply(call(address(11), 21, 13));
    index.apply(call(address(11), 22, 14));
    assert!(index.is_empty());
    assert_eq!(index.info().usage_pairs, 4);
    let mut restored = SearchIndex::from_rows(
        index.records().cloned(),
        index.usage_rows(),
        index
            .pending_rows()
            .map(|(id, pending)| (id.clone(), pending.clone())),
        index.checkpoint(),
    );
    assert_eq!(restored, index);
    let published_at = USAGE_WINDOW_SECONDS + 10;
    restored.apply(app(
        "apps/late",
        "harbor",
        published_at,
        &[address(10), address(11)],
    ));
    let rows = restored.search("harbor", 1, published_at);
    assert_eq!(rows[0].usage_7d, 2);
    assert!(rows[0].usage_complete);
    assert_eq!(restored.info().usage_pairs, 3);
    let rpc_json = serde_json::to_value(&rows).unwrap();
    assert_eq!(rpc_json[0]["usage_7d"], 2);
    assert_eq!(rpc_json[0]["usage_complete"], true);
}

#[test]
fn shared_contract_activity_counts_for_each_app_declaring_it() {
    let mut index = SearchIndex::new();
    index.apply(app("apps/old", "old", 1, &[address(10)]));
    index.apply(call(address(10), 20, 2));
    index.apply(app("apps/new", "new", 3, &[address(10)]));
    assert_eq!(index.search("old", 1, 3)[0].usage_7d, 1);
    assert_eq!(index.search("new", 1, 3)[0].usage_7d, 1);
    index.apply(call(address(10), 20, 4));
    assert_eq!(index.search("new", 1, 4)[0].usage_7d, 1);
    index.apply(SearchEvent::AppRemoved {
        id: "apps/old".into(),
    });
    assert_eq!(index.info().usage_pairs, 1);
}

#[test]
fn name_address_is_counted_while_rolling_calls_survive_binding_changes() {
    let mut index = SearchIndex::new();
    index.apply(name("names/1", "harbor", 1, 100));
    index.apply(SearchEvent::NameAddress {
        id: "names/1".into(),
        address: address(10),
    });
    index.apply(call(address(10), 20, 2));
    assert_eq!(index.search("harbor", 1, 2)[0].usage_7d, 1);
    index.apply(SearchEvent::NameAddress {
        id: "names/1".into(),
        address: Address::ZERO,
    });
    assert_eq!(index.info().usage_pairs, 1);
    assert_eq!(index.search("harbor", 1, 2)[0].usage_7d, 0);
}

#[test]
fn usage_overflow_is_public_and_disables_partial_counts_for_every_result() {
    let mut index = SearchIndex::with_limits(10, 2);
    index.apply(app("apps/older", "older", 1, &[]));
    index.apply(app("apps/later", "later", 2, &[address(10)]));
    for caller in 20..23 {
        index.apply(call(address(10), caller, 3));
    }
    let info = index.info();
    assert_eq!(info.usage_pairs, 2);
    assert_eq!(info.rejected_usage_pairs, 1);
    assert!(info.usage_saturated);
    assert!(!info.usage_complete);
    let rows = index.search("fixture", 50, 3);
    assert_eq!(names(&rows), ["older", "later"]);
    assert!(rows
        .iter()
        .all(|row| row.usage_7d == 0 && !row.usage_complete));
    index.advance(3 + USAGE_WINDOW_SECONDS);
    assert!(index.info().usage_complete);
    assert!(!index.info().usage_saturated);
    index.apply(call(address(10), 23, 4 + USAGE_WINDOW_SECONDS));
    assert_eq!(
        names(&index.search("fixture", 50, 4 + USAGE_WINDOW_SECONDS)),
        ["later", "older"]
    );
}

#[test]
fn unrelated_chain_calls_share_the_same_cap_and_disable_all_usage_fairly() {
    let mut index = SearchIndex::with_limits(10, 2);
    index.apply(app("apps/older", "older", 1, &[]));
    index.apply(app("apps/popular", "popular", 2, &[address(10)]));
    index.apply(call(address(10), 20, 3));
    assert_eq!(names(&index.search("fixture", 50, 3)), ["popular", "older"]);
    index.apply(call(address(99), 21, 3));
    index.apply(call(address(100), 22, 3));
    assert_eq!(index.info().usage_pairs, 2);
    assert_eq!(index.info().rejected_usage_pairs, 1);
    let rows = index.search("fixture", 50, 3);
    assert_eq!(names(&rows), ["older", "popular"]);
    assert!(rows
        .iter()
        .all(|row| row.usage_7d == 0 && !row.usage_complete));
}

#[test]
fn query_work_overflow_never_promotes_partial_usage_counts() {
    let mut index = SearchIndex::new();
    index.apply(app("apps/z", "older", 1, &[]));
    for n in 1..5 {
        index.apply(app(
            &format!("apps/{n}"),
            &format!("shared-{n}"),
            2 + n,
            &[address(10)],
        ));
    }
    index.apply(call(address(10), 20, 10));
    index.apply(call(address(10), 21, 10));
    let full = index.search("fixture", 50, 10);
    assert_eq!(full[0].name, "shared-1");
    assert_eq!(full.last().unwrap().name, "older");
    let limited = index.search_with_usage_budget("fixture", 50, 10, 3);
    assert_eq!(limited[0].name, "older");
    assert!(limited
        .iter()
        .all(|row| row.usage_7d == 0 && !row.usage_complete && row.query_usage_limited));
    assert!(index.info().usage_complete);
    assert_eq!(index.info().usage_pairs, 2);
}

#[test]
fn expiry_and_removal_free_admission_capacity_without_category_preferences() {
    let mut index = SearchIndex::with_limits(2, 10);
    index.apply(name("names/1", "renewed", 1, 10));
    index.apply(app("apps/1", "app", 2, &[]));
    index.apply(app("apps/overflow", "overflow", 3, &[]));
    assert_eq!(index.len(), 2);
    assert_eq!(index.info().rejected_records, 1);
    index.apply(SearchEvent::NameRenewed {
        id: "names/1".into(),
        expires_at: 20,
    });
    index.advance(10);
    assert!(index.record("names/1").is_some());
    assert!(index.search("renewed", 1, 20).is_empty());
    index.advance(20);
    assert!(index.record("names/1").is_none());
    index.apply(app("apps/new", "new", 21, &[]));
    assert_eq!(index.len(), 2);
    index.apply(SearchEvent::AppRemoved {
        id: "apps/1".into(),
    });
    index.apply(app("apps/last", "last", 22, &[]));
    assert_eq!(index.len(), 2);
    assert!(index.search("app", 1, 22).is_empty());
    assert!(index.info().records_saturated);
}

#[test]
fn delayed_release_and_unlist_use_finalized_time_and_cancellation() {
    let mut index = SearchIndex::new();
    index.apply(app("apps/1", "app", 1, &[address(10)]));
    let release = SearchEvent::AppReleaseQueued {
        id: "apps/1".into(),
        content_hash: Some(B256::repeat_byte(2)),
        metadata: SearchMetadata {
            title: "new release".into(),
            ..Default::default()
        },
        activates_at: 10,
    };
    index.apply(release.clone());
    assert_eq!(index.info().pending, 1);
    index.advance(9);
    assert_eq!(
        index.record("apps/1").unwrap().description,
        "chain application and wallet tools"
    );
    index.apply(SearchEvent::PendingCancelled {
        id: "apps/1".into(),
    });
    index.advance(10);
    assert_eq!(
        index.record("apps/1").unwrap().content_hash,
        Some(B256::repeat_byte(1))
    );
    index.apply(release);
    let record = index.record("apps/1").unwrap();
    assert_eq!(record.content_hash, Some(B256::repeat_byte(2)));
    assert_eq!(record.title, "new release");
    assert_eq!(record.description, "");
    assert!(record.contracts.is_empty());
    assert_eq!(record.created_at, 1);
    assert_eq!(index.info().pending, 0);
    index.apply(SearchEvent::AppUnlistQueued {
        id: "apps/1".into(),
        activates_at: 20,
    });
    index.advance(20);
    index.apply(SearchEvent::PendingCancelled {
        id: "apps/1".into(),
    });
    assert!(index.record("apps/1").is_none());
    assert!(index.search("app", 1, 20).is_empty());
}

#[test]
fn row_deltas_restore_exactly_on_write_failure_and_durable_reload() {
    let mut index = SearchIndex::new();
    index.apply(app("apps/z", "old", 1, &[address(10)]));
    index.apply(name("names/1", "name", 2, 10));
    index.apply(call(address(10), 20, 3));
    index.apply(SearchEvent::AppReleaseQueued {
        id: "apps/z".into(),
        content_hash: None,
        metadata: SearchMetadata::default(),
        activates_at: 50,
    });
    let before = index.clone();
    let delta = index.apply_batch(&[
        app("apps/a", "new", 4, &[address(10)]),
        SearchEvent::AppUpdated {
            id: "apps/z".into(),
            content_hash: None,
            metadata: SearchMetadata::default(),
        },
        SearchEvent::Tick { at: 10 },
    ]);
    // Catalog binding changes leave rolling usage untouched and out of the
    // delta. Undo restores catalog rows without disturbing that history.
    assert!(delta.usage.is_empty());
    assert_eq!(delta.records.len(), 3);
    assert_eq!(delta.pending.len(), 1);
    index.undo(&delta);
    assert_eq!(index, before);
    let restored = SearchIndex::from_rows(
        index.records().cloned(),
        index.usage_rows(),
        index
            .pending_rows()
            .map(|(id, pending)| (id.clone(), pending.clone())),
        index.checkpoint(),
    );
    assert_eq!(restored, index);
    let bytes = serde_json::to_vec(&delta).unwrap();
    assert_eq!(
        serde_json::from_slice::<aether_node::search::SearchDelta>(&bytes).unwrap(),
        delta
    );
}

#[test]
fn delta_contains_expired_usage_and_mature_pending_rows() {
    let mut index = SearchIndex::new();
    index.apply(app("apps/1", "app", 1, &[address(10)]));
    index.apply(call(address(10), 20, 2));
    index.apply(SearchEvent::AppUnlistQueued {
        id: "apps/1".into(),
        activates_at: USAGE_WINDOW_SECONDS + 2,
    });
    let before = index.clone();
    let delta = index.apply_batch(&[SearchEvent::Tick {
        at: USAGE_WINDOW_SECONDS + 2,
    }]);
    assert_eq!(delta.records.len(), 1);
    assert!(delta.records[0].after.is_none());
    assert_eq!(delta.usage.len(), 1);
    assert!(delta.usage[0].after.is_none());
    assert_eq!(delta.pending.len(), 1);
    assert!(delta.pending[0].after.is_none());
    index.undo(&delta);
    assert_eq!(index, before);
}

#[test]
fn incomplete_history_and_usage_are_visible_and_time_bounded() {
    let mut index = SearchIndex::new();
    index.advance(10);
    index.mark_usage_incomplete();
    assert!(index.info().history_complete);
    assert!(!index.info().usage_complete);
    index.mark_history_incomplete();
    assert!(!index.info().history_complete);
    index.advance(10 + USAGE_WINDOW_SECONDS);
    assert!(index.info().usage_complete);
    assert!(!index.info().history_complete);
}

#[test]
fn incomplete_usage_event_is_durable_and_rolls_back_with_the_batch() {
    let mut index = SearchIndex::new();
    index.apply(app("apps/1", "app", 1, &[address(10)]));
    index.apply(call(address(10), 20, 2));
    let before = index.clone();
    let delta = index.apply_batch(&[SearchEvent::UsageIncomplete { at: 3 }]);
    assert!(delta.records.is_empty());
    assert!(delta.usage.is_empty());
    assert_eq!(delta.after.clock, 3);
    assert_eq!(
        delta.after.usage_incomplete_until,
        Some(3 + USAGE_WINDOW_SECONDS)
    );
    assert!(index.info().history_complete);
    assert!(!index.info().usage_complete);
    let result = &index.search("app", 1, 3)[0];
    assert_eq!(result.usage_7d, 0);
    assert!(!result.usage_complete);
    assert!(!result.query_usage_limited);
    let restored = SearchIndex::from_rows(
        index.records().cloned(),
        index.usage_rows(),
        index
            .pending_rows()
            .map(|(id, pending)| (id.clone(), pending.clone())),
        index.checkpoint(),
    );
    assert_eq!(restored, index);
    index.undo(&delta);
    assert_eq!(index, before);
    index.apply(SearchEvent::UsageIncomplete { at: 3 });
    index.advance(3 + USAGE_WINDOW_SECONDS);
    assert!(index.info().usage_complete);
}

#[test]
fn snapshot_without_chain_time_waits_for_a_full_week_after_the_first_finalized_tick() {
    let mut snapshot = SearchIndex::new();
    snapshot.apply(app("apps/1", "app", 0, &[]));
    snapshot.mark_usage_incomplete();
    assert!(snapshot.info().awaiting_clock);
    assert!(!snapshot.info().usage_complete);
    assert_eq!(snapshot.checkpoint().usage_incomplete_until, None);
    assert!(!snapshot.search("app", 1, 1_790_000_000)[0].usage_complete);

    let mut restored = SearchIndex::from_rows(
        snapshot.records().cloned(),
        snapshot.usage_rows(),
        snapshot
            .pending_rows()
            .map(|(id, pending)| (id.clone(), pending.clone())),
        snapshot.checkpoint(),
    );
    assert_eq!(restored, snapshot);
    assert!(restored.info().awaiting_clock);
    let first_finalized_at = 1_790_000_000;
    let delta = restored.apply_batch(&[SearchEvent::Tick {
        at: first_finalized_at,
    }]);
    assert!(delta.before.awaiting_clock);
    assert!(!delta.after.awaiting_clock);
    assert_eq!(
        delta.after.usage_incomplete_until,
        Some(first_finalized_at + USAGE_WINDOW_SECONDS)
    );
    assert!(!restored.info().usage_complete);
    restored.undo(&delta);
    assert_eq!(restored, snapshot);

    restored.advance(first_finalized_at);
    restored.advance(first_finalized_at + USAGE_WINDOW_SECONDS - 1);
    assert!(!restored.info().usage_complete);
    restored.advance(first_finalized_at + USAGE_WINDOW_SECONDS);
    assert!(restored.info().usage_complete);
    assert!(!restored.info().awaiting_clock);
}

#[test]
fn unknown_time_incomplete_event_is_captured_and_undoable() {
    let mut index = SearchIndex::new();
    let before = index.clone();
    let delta = index.apply_batch(&[SearchEvent::UsageIncomplete { at: 0 }]);
    assert!(delta.after.awaiting_clock);
    assert!(!index.info().usage_complete);
    assert!(delta.after.usage_incomplete_until.is_none());
    index.advance(0);
    assert!(index.info().awaiting_clock);
    index.undo(&delta);
    assert_eq!(index, before);
}

#[test]
fn metadata_and_all_persisted_variable_fields_are_bounded() {
    let mut index = SearchIndex::new();
    index.apply(SearchEvent::AppPublished {
        id: "apps/1".into(),
        publisher: address(1),
        name: "港".repeat(500),
        url: "x".repeat(1_000),
        content_hash: None,
        created_at: 1,
        metadata: SearchMetadata {
            title: "港".repeat(500),
            description: "港".repeat(1_000),
            category: "港".repeat(100),
            contracts: (1..100).map(address).collect(),
        },
    });
    let record = index.record("apps/1").unwrap();
    assert!(record.id.len() <= MAX_ID_BYTES);
    assert!(record.name.len() <= MAX_NAME_BYTES);
    assert!(record.title.len() <= MAX_TITLE_BYTES);
    assert!(record.description.len() <= MAX_DESCRIPTION_BYTES);
    assert!(record.category.len() <= MAX_CATEGORY_BYTES);
    assert!(record.url.len() <= MAX_URL_BYTES);
    assert_eq!(record.contracts.len(), MAX_CONTRACTS);
    assert!(serde_json::to_string(record).unwrap().contains("港"));
    assert_eq!(
        SearchMetadata::from_hint(&"x".repeat(MAX_HINT_BYTES + 1)),
        SearchMetadata::default()
    );
    assert_eq!(
        SearchMetadata::from_hint("not json"),
        SearchMetadata::default()
    );
    assert_eq!(
        SearchMetadata::from_hint(
            r#"{"title":"Harbor","description":"tools","category":"finance"}"#
        )
        .category,
        "finance"
    );
    index.apply(SearchEvent::NameText {
        id: "apps/1".into(),
        key: "unknown".into(),
        value: "x".repeat(10_000),
    });
    assert_eq!(index.len(), 1);
}

#[test]
fn hint_json_boundary_is_checked_before_parsing_and_keeps_all_supported_fields() {
    let expected = SearchMetadata {
        title: "Harbor".into(),
        description: "wallet tools".into(),
        category: "tools".into(),
        contracts: vec![address(10)],
    };
    let hint = serde_json::to_string(&expected).unwrap();
    assert!(hint.len() <= MAX_HINT_BYTES);
    assert_eq!(SearchMetadata::from_hint(&hint), expected);

    let overhead = r#"{"title":""}"#.len();
    let exact = format!(r#"{{"title":"{}"}}"#, "x".repeat(MAX_HINT_BYTES - overhead));
    assert_eq!(exact.len(), MAX_HINT_BYTES);
    assert_eq!(
        SearchMetadata::from_hint(&exact).title.len(),
        MAX_HINT_BYTES - overhead
    );

    // This is valid JSON that would otherwise produce a nonempty title.
    let oversized = format!(
        r#"{{"title":"{}"}}"#,
        "x".repeat(MAX_HINT_BYTES - overhead + 1)
    );
    assert_eq!(oversized.len(), MAX_HINT_BYTES + 1);
    assert_eq!(
        SearchMetadata::from_hint(&oversized),
        SearchMetadata::default()
    );
}

#[test]
fn multiline_hint_and_text_records_remain_searchable_with_visible_spacing() {
    let metadata = SearchMetadata::from_hint(
        "{\n  \"title\": \"Harbor\\nTools\",\n  \"description\": \"chain\\tapps\"\n}",
    );
    assert_eq!(metadata.title, "Harbor Tools");
    assert_eq!(metadata.description, "chain apps");
    let mut index = SearchIndex::new();
    index.apply(SearchEvent::AppPublished {
        id: "apps/1".into(),
        publisher: address(1),
        name: "harbor".into(),
        url: "sea://harbor-app/".into(),
        content_hash: None,
        metadata,
        created_at: 1,
    });
    assert_eq!(names(&index.search("tools chain", 1, 2)), ["harbor"]);
    index.apply(name("names/1", "shop", 2, 100));
    index.apply(SearchEvent::NameText {
        id: "names/1".into(),
        key: "description".into(),
        value: "wallet\nshop\u{0000}".into(),
    });
    let rows = index.search("wallet shop", 1, 3);
    assert_eq!(names(&rows), ["shop.sea"]);
    assert_eq!(rows[0].description, "wallet shop ");
}

#[test]
fn one_hundred_thousand_records_have_a_fixed_retention_bound() {
    let mut index = SearchIndex::new();
    for n in 0..MAX_RECORDS + 5 {
        index.apply(app(
            &format!("apps/{n:06}"),
            &format!("record-{n:06}"),
            1,
            &[],
        ));
    }
    assert_eq!(index.len(), MAX_RECORDS);
    assert_eq!(index.info().rejected_records, 5);
    assert_eq!(index.info().pending, 0);
    assert_eq!(index.info().usage_pairs, 0);
    assert!(index.retained_payload_bytes() < MAX_RECORDS * 256);
    assert!(index.record("apps/099999").is_some());
    assert!(index.record("apps/100000").is_none());
    assert_eq!(index.search("fixture", usize::MAX, 1).len(), MAX_RESULTS);
    assert!(index.search("fixture", 0, 1).is_empty());
    assert!(index.search("", 10, 1).is_empty());
}
