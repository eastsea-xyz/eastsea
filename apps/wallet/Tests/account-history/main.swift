import Foundation

func check(_ value: Bool, _ message: String) {
    if !value { fputs("FAIL: \(message)\n", stderr); exit(1) }
}

let own = "0x" + String(repeating: "12", count: 20)
let other = "0x" + String(repeating: "34", count: 20)
let router = "0x" + String(repeating: "56", count: 20)
let token = "0x" + String(repeating: "78", count: 20)
let json = """
{"entries":[{"address":"\(own)","height":7,"tx_index":0,"tx_hash":"0xabc","timestamp_ms":7000,
"direction":"in","kind":"native_transfer","from":"\(other)","to":"\(own)",
"value_wei":"5000000000000000000","method":null,"contract_address":null,"success":true,"tokens":[]}],
"next_cursor":null,"history_start":3,"indexed_height":7}
"""
let page = try ChainHistoryPage.decode(json)
check(page.entries.count == 1 && page.historyStart == 3, "decode page")
check(ChainActivity.title(page.entries[0], names: ChainNames()) == "Received 5 DBLN from \(ChainActivity.short(other))", "received DBLN")
check(ChainActivity.historyKey(hash: "0xABC", address: own) == ChainActivity.historyKey(hash: "0xabc", address: own.uppercased()), "same wallet dedupes a hash")
check(ChainActivity.historyKey(hash: "0xabc", address: own) != ChainActivity.historyKey(hash: "0xabc", address: other), "linked wallets keep both sides of a transaction")

let swapJSON = """
{"entries":[{"address":"\(own)","height":8,"tx_index":0,"tx_hash":"0xdef","timestamp_ms":8000,
"direction":"out","kind":"contract_call","from":"\(own)","to":"\(router)",
"value_wei":"10000000000000000000","method":"0xac344b4d","contract_address":null,"success":true,
"tokens":[{"token":"\(token)","from":"\(router)","to":"\(own)","amount":"250000000000000000000"}],
"pair_swaps":[{"pair":"\(router)","amount0_in":"10","amount1_in":"0","amount0_out":"0","amount1_out":"250"}]}],
"next_cursor":null,"history_start":3,"indexed_height":8}
"""
let swap = try ChainHistoryPage.decode(swapJSON).entries[0]
let names = ChainNames(router: router, tokens: [token: ChainTokenName(symbol: "NEB", decimals: 18, origin: "seed")])
check(ChainActivity.title(swap, names: names).hasPrefix("Swapped 10 DBLN → 250 NEB ·"), "swap title")
let revoke = try ChainHistoryPage.decode(swapJSON
    .replacingOccurrences(of: "0xac344b4d", with: "0x095ea7b3")
    .replacingOccurrences(of: "\"contract_address\":null", with: "\"approval_amount\":\"0\",\"approval_spender\":\"\(other)\",\"contract_address\":null")).entries[0]
check(ChainActivity.title(revoke, names: names).hasPrefix("Revoked token"), "revoke title")

check(ChainActivity.validLinkedAddress(other, own: own, existing: []) == other, "link another address")
check(ChainActivity.validLinkedAddress(own, own: own, existing: []) == nil, "cannot link own address")
check(ChainActivity.validLinkedAddress(other, own: own, existing: [other]) == nil, "no duplicate link")
check(ChainActivity.rise("10000000000000000001", over: "9999999999999999999") == "2", "exact balance rise")
check(ChainActivity.rise("9", over: "10") == nil, "no negative rise")

var notice = IncomingNoticeState(height: 7, hashes: ["0xabc"])
check(notice.consume(page.entries).isEmpty, "same payment does not notify again")
let newReceipt = try ChainHistoryPage.decode(json.replacingOccurrences(of: "0xabc", with: "0xnew")
    .replacingOccurrences(of: "\"height\":7", with: "\"height\":8")).entries[0]
check(notice.consume([newReceipt]).count == 1, "new payment notifies")
check(notice.consume([newReceipt]).isEmpty, "new payment only notifies once")
print("OK")
