//! R18: inspect JSON handled and emitted by the actual RPC relay path.
use super::*;
use aether_crypto::{P256Signer, PublicKey, Signer as _};
use aether_net::registrar::{EncryptionKey, RecipientSecret};
use commonware_codec::Encode as _;
use commonware_cryptography::Signer as _;
use std::sync::Mutex;

type Captured = Arc<Mutex<Vec<Value>>>;

async fn downstream(axum::extract::State((st, seen)): axum::extract::State<(Arc<RpcState>, Captured)>, axum::Json(request): axum::Json<Value>) -> axum::Json<Value> {
    seen.lock().unwrap().push(request.clone());
    axum::Json(handle_remote_value(&st, request).await)
}

fn fixture() -> (RpcState, Arc<crate::devicecheck::Registrar>, PublicKey, std::path::PathBuf) {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let tmp = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp");
    std::fs::create_dir_all(&tmp).unwrap();
    let dir = tmp.join(format!("devicecheck-relay-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    std::fs::create_dir(&dir).unwrap();
    let signer = P256Signer::from_seed(&[4; 32]).unwrap();
    let public = signer.public_key();
    let (x, y) = aether_crypto::p256_xy(&public.bytes).unwrap();
    let mut st = bare_state();
    {
        let mut chain = st.chain.lock();
        aether_execution::registry::set_registrar(&mut Arc::make_mut(&mut chain.finalized).state, (x, y));
    }
    let registrar = Arc::new(crate::devicecheck::Registrar::new(None, crate::devicecheck::Registry::open(dir.join("registrations.json")),
        Arc::new(crate::registrar_signer::FileSigner::from_seed(&[4; 32]).unwrap()), 7781));
    st.registrar = Some(registrar.clone());
    (st, registrar, public, dir)
}

fn registration_public(k: &commonware_cryptography::ed25519::PrivateKey) -> Vec<Value> {
    let operator = Address::repeat_byte(1);
    let voting: [u8; 32] = k.public_key().as_ref().try_into().unwrap();
    let node = *aether_net::SecretKey::from_bytes(&[3; 32]).public().as_bytes();
    let beaconer = Address::repeat_byte(2);
    let msg = aether_execution::registry::attestation_message(7781, operator, voting, node, beaconer);
    vec![json!(operator), json!(hex::encode(voting)), json!(hex::encode(node)), json!(beaconer),
        json!(hex::encode(k.sign(crate::devicecheck::OWNERSHIP_NAMESPACE, &msg).encode()))]
}

async fn remote(st: &RpcState, method: &str, params: Value) -> Value {
    handle_remote_value(st, json!({"jsonrpc":"2.0", "id":1, "method":method, "params":params})).await
}

#[tokio::test]
async fn registration_and_reattest_relays_see_ciphertext_on_success_and_failure() {
    let token = "R18_PRIVATE_DEVICECHECK_TOKEN_must_not_reach_any_relay";
    let (st, registrar, pinned, dir) = fixture();
    let seen: Captured = Arc::new(Mutex::new(vec![]));
    let app = axum::Router::new().route("/", axum::routing::post(downstream)).with_state((Arc::new(st), seen.clone()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut relay = bare_state();
    relay.upstream = Some(Arc::new(crate::follow::Upstream::Http(vec![url])));

    let descriptor_reply = remote(&relay, "aether_registrarEncryptionKey", json!([])).await;
    let descriptor: EncryptionKey = serde_json::from_value(descriptor_reply["result"].clone()).unwrap();
    descriptor.authenticate(7781, &pinned).unwrap();
    let k = commonware_cryptography::ed25519::PrivateKey::from_seed(77);
    let public = registration_public(&k);
    let registration = crate::devicecheck::encrypt_token_request(token, &descriptor, 7781, &pinned, "aether_registerDevice", public.clone()).unwrap();
    assert_eq!(&**registrar.open_token("aether_registerDevice", &registration).unwrap(), token);
    let response = remote(&relay, "aether_registerDevice", registration.clone()).await;
    assert!(response.get("error").is_none(), "{response}");
    assert!(!response.to_string().contains(token));

    let voting: [u8; 32] = k.public_key().as_ref().try_into().unwrap();
    let msg = aether_rewards::beacons::reattest_message(7781, &voting, 0);
    let params = vec![json!(hex::encode(voting)), json!(0), json!(hex::encode(k.sign(crate::devicecheck::OWNERSHIP_NAMESPACE, &msg).encode()))];
    let daily = crate::devicecheck::encrypt_token_request(token, &descriptor, 7781, &pinned, "aether_reattest", params).unwrap();
    let response = remote(&relay, "eastsea_reattest", daily).await;
    assert!(response.get("error").is_none(), "{response}");
    assert!(!response.to_string().contains(token));

    let mut tampered = registration.clone();
    let mut bytes = hex::decode(tampered[0]["ciphertext"].as_str().unwrap()).unwrap();
    bytes[0] ^= 1;
    tampered[0]["ciphertext"] = json!(hex::encode(bytes));
    let mut changed_params = registration.clone();
    changed_params[4] = json!(Address::repeat_byte(9));
    for altered in [tampered, changed_params] {
        let response = remote(&relay, "aether_registerDevice", altered).await;
        assert_eq!(response["error"]["code"], -32000); // upstream wraps registrar rejection
        assert!(response["error"]["message"].as_str().unwrap().contains("could not be authenticated"));
        assert!(!response.to_string().contains(token));
    }

    let old_key = RecipientSecret::from_seed(&[5; 32]).unwrap();
    let signer = P256Signer::from_seed(&[4; 32]).unwrap();
    let stale_descriptor = EncryptionKey::signed(7781, &old_key.public_key(), |m| signer.sign(m).map_err(|e| e.to_string())).unwrap();
    let stale = crate::devicecheck::encrypt_token_request(token, &stale_descriptor, 7781, &pinned, "aether_registerDevice", public.clone()).unwrap();
    let response = remote(&relay, "aether_registerDevice", stale).await;
    assert!(response["error"]["message"].as_str().unwrap().contains("encryption key expired"));
    assert!(!response.to_string().contains(token));
    let impostor = P256Signer::from_seed(&[6; 32]).unwrap();
    let wrong = EncryptionKey::signed(7781, &old_key.public_key(), |m| impostor.sign(m).map_err(|e| e.to_string())).unwrap();
    assert!(crate::devicecheck::encrypt_token_request(token, &wrong, 7781, &pinned, "aether_registerDevice", public).is_err());

    let before = seen.lock().unwrap().len();
    let mut plaintext_registration = registration.clone();
    plaintext_registration[0] = json!(token);
    let mut plaintext_daily = json!([token, hex::encode(voting), 0, "00"]);
    let mut oversized = registration;
    oversized[0]["ciphertext"] = json!("aa".repeat(aether_net::registrar::MAX_TOKEN_BYTES + 17));
    for (method, p) in [("aether_registerDevice", plaintext_registration), ("aether_reattest", plaintext_daily.take()), ("aether_registerDevice", oversized)] {
        let response = remote(&relay, method, p).await;
        assert_eq!(response["error"]["code"], -32602);
        assert!(!response.to_string().contains(token));
    }
    assert_eq!(seen.lock().unwrap().len(), before, "legacy/plaintext/oversized input never reaches any upstream");
    let emitted = seen.lock().unwrap();
    for request in emitted.iter() {
        assert!(!request.to_string().contains(token), "relay serialized token plaintext");
        if request["method"] != "aether_registrarEncryptionKey" {
            assert!(request["params"][0].is_object());
            assert!(request["params"][0]["ciphertext"].is_string());
            crate::devicecheck::encrypted_token(&request["params"]).unwrap();
        }
    }
    drop(emitted);
    assert!(!std::fs::read_to_string(dir.join("registrations.json")).unwrap().contains(token));
    server.abort();
    let _ = server.await;
    std::fs::remove_dir_all(dir).unwrap();
}
