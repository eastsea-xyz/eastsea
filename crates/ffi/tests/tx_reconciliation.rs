//! Receipt loss must not turn a successful offline payment into a final
//! replacement. Loopback fixtures only; this binary owns its FFI globals.

use aether_ffi::{tx_status_for, use_local_node};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex};

struct ReceiptNode {
    port: u16,
    receipt: Arc<Mutex<Value>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ReceiptNode {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let receipt = Arc::new(Mutex::new(Value::Null));
        let stop = Arc::new(AtomicBool::new(false));
        let (served, stopping) = (receipt.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            for connection in listener.incoming() {
                if stopping.load(Ordering::Acquire) { break; }
                let mut stream = connection.unwrap();
                let request = read_request(&mut stream);
                let result = match request["method"].as_str().unwrap() {
                    "eth_getTransactionCount" => json!("0x8"),
                    "aether_getReceipt" => served.lock().unwrap().clone(),
                    method => panic!("unexpected fixture method: {method}"),
                };
                let body = json!({ "jsonrpc": "2.0", "id": 1, "result": result }).to_string();
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        use_local_node(Some(port));
        Self { port, receipt, stop, thread: Some(thread) }
    }
}

impl Drop for ReceiptNode {
    fn drop(&mut self) {
        use_local_node(None);
        self.stop.store(true, Ordering::Release);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        let _ = self.thread.take().unwrap().join();
    }
}

fn read_request(stream: &mut TcpStream) -> Value {
    let (mut head, mut byte) = (Vec::new(), [0u8; 1]);
    while !head.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).unwrap();
        head.push(byte[0]);
    }
    let headers = String::from_utf8(head).unwrap();
    let len: usize = headers.lines().find(|line| line.to_ascii_lowercase().starts_with("content-length:"))
        .unwrap().split_once(':').unwrap().1.trim().parse().unwrap();
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[test]
fn r16_advanced_nonce_without_receipt_keeps_the_payment_unresolved() {
    let node = ReceiptNode::start();
    let hash = format!("0x{}", "15".repeat(32));
    let sender = format!("0x{}", "21".repeat(20));
    let status = tx_status_for(hash.clone(), sender.clone(), 7).unwrap();
    assert!(
        !status.is_final && status.state != "replaced",
        "R16: an advanced nonce and missing cached receipt do not prove this payment was replaced"
    );
    assert!(!status.can_resend, "nonce 7 is consumed even while this payment's result is unresolved");
    assert!(!status.message.contains("빠져나간 돈은 없어요"), "the wallet has no evidence that no money moved");

    // A receipt discovered on a later refresh resolves the same row as the
    // successful payment; the earlier absence never became a chain fact.
    *node.receipt.lock().unwrap() = json!({ "height": 12,
        "receipt": { "success": true, "gas_used": 21_000, "state_fee": "0" } });
    let resolved = tx_status_for(hash, sender, 7).unwrap();
    assert!(resolved.is_final && resolved.state == "included");
    assert!(resolved.receipt.unwrap().success);
}
