// "새 가격으로 다시 보내기" (contracts-live bug #5): a resend signs the dropped
// transfer's nonce again only while the sheet holds exactly that transfer.
// Pure Foundation: no RPC, no FFI, no SwiftUI.
import Foundation

func check(_ name: String, _ ok: Bool) {
    if ok { print("ok   \(name)") } else { print("FAIL \(name)"); exit(1) }
}

let r = ResendIntent(to: "0xAbC0000000000000000000000000000000000001", valueWei: "1000000000000000000", nonce: 7)
check("same recipient, any case, same wei", r.matches(to: "0xabc0000000000000000000000000000000000001", valueWei: "1000000000000000000"))
check("another amount is a new send", !r.matches(to: r.to, valueWei: "1000000000000000001"))
check("another recipient is a new send", !r.matches(to: "0xabc0000000000000000000000000000000000002", valueWei: r.valueWei))

// Activity rows are stored as JSON: the intent survives a restart exactly.
let data = try! JSONEncoder().encode(r)
check("round-trips through the saved activity", (try? JSONDecoder().decode(ResendIntent.self, from: data)) == r)
print("resend: all ok")
