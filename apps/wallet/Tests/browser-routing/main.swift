// Checks the Explore tab's provider routing and transaction checks without
// an app or a node (mirrors apps/extension/test/methods.test.mjs):
//   swiftc -o ./tmp/browser-routing-check apps/wallet/Sources/Brand.swift apps/wallet/Sources/BrowserPolicy.swift apps/wallet/Tests/browser-routing/main.swift && ./tmp/browser-routing-check
import Foundation
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); exit(1) } }

let TO = "0x00000000000000000000000000000000000000aa"
let ME = "0x00000000000000000000000000000000000000bb"

// The method set is the extension's, exactly: READ ∪ ACCOUNT ∪ SEND plus the
// three background.js answers by hand. Anything else is refused (4200) — the
// extension refuses it too, so a page sees the same wallet on both surfaces.
check(ProviderMethod.read.count == 14, "14 read methods")
check(ProviderMethod.read.contains("eth_blockNumber") && ProviderMethod.read.contains("aether_accountHistory"), "read set contents")
check(!ProviderMethod.read.contains("eth_getTransactionReceipt"), "no eth_getTransactionReceipt (not in methods.js)")
check(ProviderRouter.maxPendingPerOrigin == 3, "pending cap matches the extension")

// Routing: read vs sign vs answered by hand, verified vs node.
check(ProviderRouter.route(method: "eth_chainId", params: []) == .chainId, "chainId")
check(ProviderRouter.route(method: "eth_accounts", params: []) == .accounts, "accounts")
check(ProviderRouter.route(method: "aether_accounts", params: []) == .accounts, "aether accounts")
check(ProviderRouter.route(method: "eth_requestAccounts", params: []) == .requestAccounts, "requestAccounts")
check(ProviderRouter.route(method: "wallet_disconnect", params: []) == .disconnect, "disconnect")
check(ProviderRouter.route(method: "aether_disconnect", params: []) == .disconnect, "aether disconnect")
check(ProviderRouter.route(method: "eth_sendTransaction", params: [[:]]) == .send, "send")
check(ProviderRouter.route(method: "aether_sendTransaction", params: [[:]]) == .send, "aether send")
// Verified paths: the FFI's certificate-backed reads.
check(ProviderRouter.route(method: "eth_getBalance", params: [TO, "latest"]) == .read(verified: true), "getBalance verified")
check(ProviderRouter.route(method: "eth_blockNumber", params: []) == .read(verified: true), "blockNumber verified")
check(ProviderRouter.route(method: "net_version", params: []) == .read(verified: true), "net_version verified")
check(ProviderRouter.route(method: "aether_status", params: []) == .read(verified: false), "aether_status node (shape parity)")
check(ProviderRouter.route(method: "aether_accountHistory", params: [TO]) == .read(verified: true), "accountHistory verified")
check(ProviderRouter.route(method: "aether_getReceipt", params: ["0xabc"]) == .read(verified: false), "aether_getReceipt node (shape parity)")
check(ProviderRouter.route(method: "eth_call", params: [["to": TO], "latest"]) == .read(verified: true), "call without from verified")
// Unverified paths: the local node answers, the tab says so.
check(ProviderRouter.route(method: "eth_call", params: [["from": ME, "to": TO], "latest"]) == .read(verified: false), "call with from goes to the node")
check(ProviderRouter.route(method: "eth_getTransactionCount", params: [TO, "latest"]) == .read(verified: false), "count unverified")
check(ProviderRouter.route(method: "eth_estimateGas", params: []) == .read(verified: false), "estimateGas unverified")
check(ProviderRouter.route(method: "eth_gasPrice", params: []) == .read(verified: false), "gasPrice unverified")
check(ProviderRouter.route(method: "eth_getCode", params: [TO, "latest"]) == .read(verified: false), "getCode unverified")
check(ProviderRouter.route(method: "aether_getAccount", params: [TO]) == .read(verified: false), "aether_getAccount node (shape parity)")
// Refused (4200): the signing methods methods.js does not list.
check(ProviderRouter.route(method: "personal_sign", params: []) == .refused, "personal_sign refused")
check(ProviderRouter.route(method: "eth_signTypedData_v4", params: []) == .refused, "signTypedData refused")
check(ProviderRouter.route(method: "wallet_switchEthereumChain", params: []) == .refused, "switchChain refused")
check(ProviderRouter.route(method: "eth_getTransactionReceipt", params: ["0xdead"]) == .refused, "getTransactionReceipt refused (aether_getReceipt is the one the chain answers)")

// The lock gate: locked, every method is refused with 4100 — reads included,
// exactly as the extension's vault answers nothing while locked.
check(ProviderGate.check(locked: true)?.code == ProviderErrorCode.locked, "locked refuses everything")
check(ProviderGate.check(locked: true) == ProviderGate.check(locked: true), "the same refusal every time")
check(ProviderGate.check(locked: false) == nil, "unlocked answers")

// normalizeTx, mirrored: hex quantities, decimal strings, safe integers.
func parse(_ d: Any?) -> Result<PageTransaction, ProviderError> { PageTransaction.parse(d, from: nil) }
check(parse(["to": TO, "value": "0xde0b6b3a7640000", "data": "0xA9059CBB", "gas": "0x5208"])
      == .success(PageTransaction(to: TO, valueWei: "1000000000000000000", data: "0xa9059cbb", gas: 21000)), "normalize hex forms")
check(parse(["to": TO]) == .success(PageTransaction(to: TO, valueWei: "0", data: "0x", gas: 0)), "defaults")
check(parse(["data": "0x6000"]) == .success(PageTransaction(to: "", valueWei: "0", data: "0x6000", gas: 0)), "creation")
check(parse(["to": TO, "value": "123", "gas": 30000]) == .success(PageTransaction(to: TO, valueWei: "123", data: "0x", gas: 30000)), "decimal and integer forms")
check(parse(["to": TO, "input": "0xa9059cbb"]).map { $0.data == "0xa9059cbb" } == .success(true), "input accepted like data")
check(parse(["to": TO, "gasLimit": "0x5208"]).map { $0.gas == 21000 } == .success(true), "gasLimit accepted like gas")
// A value past UInt64 still parses exactly.
check(parse(["to": TO, "value": "0x152d02c7e14af6800000"]).map { $0.valueWei == "100000000000000000000000" } == .success(true), "100k AETH in hex")

// Malformed requests are refused with -32602, each for its own reason.
func fails(_ d: Any?, _ code: Int = ProviderErrorCode.params) -> ProviderError? {
    if case .failure(let e) = parse(d) { return e.code == code ? e : nil }
    return nil
}
check(fails(nil) != nil, "not an object")
check(fails([]) != nil, "array is not an object")
check(fails(["to": "0x1234"]) != nil, "short address")
check(fails(["to": 42]) != nil, "numeric to")
check(fails(["to": TO, "data": "0x123"]) != nil, "odd hex")
check(fails(["to": TO, "data": "a9059cbb"]) != nil, "missing 0x")
check(fails(["to": TO, "value": -1]) != nil, "negative value")
check(fails(["to": TO, "value": true]) != nil, "boolean value")
check(fails(["to": TO, "value": "1.5"]) != nil, "fractional string value")
check(fails(["to": TO, "gas": 10_000_001]) != nil, "gas above the cap")
check(parse(["to": TO, "gas": "0x1e"]).map { $0.gas == 30 } == .success(true), "small hex gas fine")
check(fails([:]) != nil, "creation needs data")

// `from` must be the connected account (the extension's SEND rule).
check(PageTransaction.parse(["to": TO, "from": ME], from: ME).isSuccess, "own from accepted")
if case .failure(let e) = PageTransaction.parse(["to": TO, "from": TO], from: ME) {
    check(e.code == ProviderErrorCode.locked, "foreign from is a permission refusal")
} else { check(false, "foreign from refused") }
check(parse(["to": TO, "from": ME]).isSuccess, "no connected account: from unchecked at parse")

// Exact hex → decimal (values past UInt64 never round).
check(PageTransaction.decimal(fromHex: "0x0") == "0", "zero")
check(PageTransaction.decimal(fromHex: "0x1") == "1", "one")
check(PageTransaction.decimal(fromHex: "0xde0b6b3a7640000") == "1000000000000000000", "1e18")
check(PageTransaction.decimal(fromHex: "0x" + String(repeating: "f", count: 16)) == "18446744073709551615", "UInt64 max")
check(PageTransaction.decimal(fromHex: "0xffffffffffffffffffffffffffffffff") == "340282366920938463463374607431768211455", "128 bits of ones")

// Wei answers a page with: exact decimal → hex quantity, and back.
check(PageTransaction.hex(fromDecimal: "0") == "0x0", "zero hex")
check(PageTransaction.hex(fromDecimal: "1000000000000000000") == "0xde0b6b3a7640000", "1e18 to hex")
check(PageTransaction.hex(fromDecimal: "340282366920938463463374607431768211455") == "0xffffffffffffffffffffffffffffffff", "128 bits back to hex")
check(PageTransaction.hex(fromDecimal: "123") == "0x7b", "small decimal to hex")
for v in ["0", "1", "1000000000000000000", "999848293841", "340282366920938463463374607431768211455"] {
    check(PageTransaction.decimal(fromHex: PageTransaction.hex(fromDecimal: v)) == v, "round trip \(v)")
}

// The confirmation sheet's Action line (describeCall, mirrored).
check(CallDescribe.action(to: TO, data: "0x") == "Send AETH", "plain transfer")
check(CallDescribe.action(to: TO, data: "0x38ed1739aa") == "Swap tokens", "swap")
check(CallDescribe.action(to: TO, data: "0xcce7ec13") == "Buy on the launch curve", "launch buy")
check(CallDescribe.action(to: TO, data: "0x5cf66fe1") == "Buy with AETH (graduated pool)", "graduated buy")
check(CallDescribe.action(to: TO, data: "0xac344b4d") == "Swap AETH for tokens", "swap for tokens")
check(CallDescribe.action(to: "", data: "0x60006001") == "Deploy a contract (4 bytes)", "deploy")
check(CallDescribe.action(to: TO, data: "0x12345678").hasPrefix("Contract call 0x12345678"), "unknown selector")

extension Result {
    var isSuccess: Bool { if case .success = self { return true }; return false }
}

print("browser-routing OK")
