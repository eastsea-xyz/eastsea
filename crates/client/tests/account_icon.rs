use aether_client::{AccountIconError, AccountIconSpec, ACCOUNT_ICON_VERSION};
use serde_json::{json, Value};

fn features(spec: AccountIconSpec) -> Value {
    json!({"version": spec.version, "palette": spec.palette, "layout": spec.layout,
           "shape": spec.shape, "rotation": spec.rotation})
}

#[test]
fn shared_cross_language_goldens() {
    let fixture: Value = serde_json::from_str(include_str!("account-icon-vectors.json")).unwrap();
    let vectors = fixture["vectors"].as_array().unwrap();
    assert_eq!(vectors.len(), 16);
    assert_eq!(fixture["version"], ACCOUNT_ICON_VERSION);
    assert_eq!(fixture["domain"], "eastsea-account-icon-v2");
    assert_eq!(fixture["palettes"].as_array().unwrap().len(), 16);
    assert_eq!(fixture["silhouettes"].as_array().unwrap().len(), 16);
    for vector in vectors {
        let address = vector["address"].as_str().unwrap();
        let expected = &vector["features"];
        for text in [
            address.to_owned(),
            address.to_uppercase(),
            address[2..].to_owned(),
        ] {
            assert_eq!(
                &features(AccountIconSpec::from_address(&text).unwrap()),
                expected,
                "{text}"
            );
            assert_eq!(
                AccountIconSpec::from_address(&text)
                    .unwrap()
                    .silhouette_class(),
                vector["silhouetteClass"].as_u64().unwrap() as u8,
                "{text}"
            );
        }
    }
}

#[test]
fn rejects_malformed_input_and_unknown_versions() {
    let valid = format!("0x{}", "12".repeat(20));
    for version in (0..=u8::MAX).filter(|&version| version != ACCOUNT_ICON_VERSION) {
        assert_eq!(
            AccountIconSpec::from_address_version(&valid, version),
            Err(AccountIconError::UnsupportedVersion)
        );
    }
    for text in [
        "".to_owned(),
        "0x123".to_owned(),
        "12".repeat(19),
        "12".repeat(21),
        "gg".repeat(20),
        format!(" {valid}"),
        format!("{valid}\n"),
        format!("0x{}", "１".repeat(40)),
        "eastsea.eth".to_owned(),
    ] {
        assert_eq!(
            AccountIconSpec::from_address(&text),
            Err(AccountIconError::InvalidAddress),
            "{text}"
        );
    }
}

#[test]
fn byte_and_text_apis_agree_and_coastline_classes_are_bounded() {
    assert_eq!(
        AccountIconSpec::from_bytes(&[0; 20]),
        AccountIconSpec::from_address(&"00".repeat(20)).unwrap()
    );
    for value in 0u16..512 {
        let mut address = [0u8; 20];
        address[18..].copy_from_slice(&value.to_be_bytes());
        let spec = AccountIconSpec::from_bytes(&address);
        assert!(spec.palette < 16 && spec.layout < 16384 && spec.shape < 4 && spec.rotation < 4);
        assert!(spec.silhouette_class() < 16);
    }
}

#[test]
fn same_displayed_prefix_and_suffix_do_not_seed_the_same_icon() {
    let first = "0x1234567890abcdef1234567890abcdef12345678";
    let other = "0x1234567890abcdef0000000000abcdef12345678";
    assert_eq!(&first[..10], &other[..10]);
    assert_eq!(&first[first.len() - 6..], &other[other.len() - 6..]);
    assert_ne!(
        AccountIconSpec::from_address(first),
        AccountIconSpec::from_address(other)
    );
}
