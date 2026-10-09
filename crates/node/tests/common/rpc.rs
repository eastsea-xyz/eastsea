//! RPC listeners held by integration tests, with one bind-only startup retry.

use aether_node::rpc::{self, RpcState};
use aether_test_support::Port;
use std::io::ErrorKind;
use std::time::Duration;

pub async fn serve(port: Port, state: RpcState) {
    let addr = port.addr();
    let _port = port;
    let first = rpc::serve(addr, state.clone()).await;
    let result = match first {
        Err(err) if err.kind() == ErrorKind::AddrInUse => {
            eprintln!("TEST RPC BIND RETRY: {addr}: {err}; retrying startup once");
            tokio::time::sleep(Duration::from_millis(100)).await;
            rpc::serve(addr, state).await
        }
        result => result,
    };
    result.expect("test RPC server failed");
}
