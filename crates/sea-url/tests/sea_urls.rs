//! Shared golden contract with the pure Swift and browser helpers.
use aether_sea_url::{browser_input, parse, suggested_https, BrowserInput, Link, RESERVED_HOSTS};
use serde_json::{json, Value};

fn result(input: &str, operation: &str, chain_id: u64) -> Value {
    if operation == "browser" {
        match browser_input(input, chain_id) {
            Ok(BrowserInput::Name(name)) => {
                let mut value = serde_json::to_value(name).unwrap();
                value["kind"] = json!("name");
                value
            }
            Ok(BrowserInput::Action { host, raw }) => {
                json!({"kind":"action","host":host,"raw":raw})
            }
            Ok(BrowserInput::Web(url)) => json!({"kind":"web","url":url}),
            Err(error) => json!({"error":error.as_str()}),
        }
    } else {
        match parse(input, chain_id) {
            Ok(Link::Name(name)) => {
                let mut value = serde_json::to_value(name).unwrap();
                value["kind"] = json!("name");
                value
            }
            Ok(Link::Action { host, raw }) => json!({"kind":"action","host":host,"raw":raw}),
            Err(error) => json!({"error":error.as_str()}),
        }
    }
}

#[test]
fn shared_sea_url_golden_contract() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/sea-urls.json")).unwrap();
    assert_eq!(json!(RESERVED_HOSTS), fixture["reservedHosts"]);
    for row in fixture["cases"].as_array().unwrap() {
        let input = row["input"].as_str().unwrap();
        assert_eq!(
            result(
                input,
                row["operation"].as_str().unwrap(),
                row["chainID"].as_u64().unwrap()
            ),
            row["expected"],
            "{}: {input:?}",
            row["id"]
        );
        if let Some(https) = row["suggestedHTTPS"].as_str() {
            assert_eq!(
                suggested_https(input).as_deref(),
                Some(https),
                "{} HTTPS offer",
                row["id"]
            );
        }
    }
}

#[test]
fn https_offer_never_reinterprets_an_invalid_authority() {
    for input in [
        "sea://user@harbor.com",
        "sea://harbor.com:443",
        "sea://harbor.com\\evil",
        "sea://harbor.com/#pay",
    ] {
        assert_eq!(suggested_https(input), None, "{input}");
    }
}
