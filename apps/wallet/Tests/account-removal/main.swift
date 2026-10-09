import Foundation

var checks = 0
var failures = 0
func check(_ label: String, _ body: () throws -> Bool) {
    checks += 1
    do {
        if try body() { print("ok   \(label)") }
        else { failures += 1; print("FAIL account-removal: \(label)") }
    } catch {
        failures += 1
        print("FAIL account-removal: \(label): \(error)")
    }
}

func blocked(_ label: String, _ body: () throws -> Void) {
    check(label) {
        do { try body(); return false }
        catch { return true }
    }
}

func address(_ number: UInt64) -> String {
    let hex = String(number, radix: 16)
    return "0x" + String(repeating: "0", count: 40 - hex.count) + hex
}
func uint(_ value: UInt64) -> String { "0x" + EVMABI.word(uint: value) }
func addressWord(_ value: String) -> String { "0x" + EVMABI.word(address: value) }
func request(_ to: String, _ selector: String, _ words: [String] = []) -> String {
    to.lowercased() + "|" + EVMABI.call(selector, words)
}

enum MockFailure: Error { case unavailable, unexpectedRequest }
final class MockRPC {
    var answers: [String: String] = [:]
    var unavailable: Set<String> = []
    var calls: [String] = []

    func read(_ to: String, _ data: String) throws -> String {
        let key = to.lowercased() + "|" + data
        calls.append(key)
        if unavailable.contains(key) { throw MockFailure.unavailable }
        guard let raw = answers[key] else { throw MockFailure.unexpectedRequest }
        return raw
    }
}

let owner = address(1)
let factory = address(101), pairFactory = address(102), launchpad = address(103), pair = address(201)
let allTokens = (11...18).map { address(UInt64($0)) }
let sources = TokenSources(waeth: allTokens[1], tokenFactory: factory, pairFactory: pairFactory,
                           launchpad: launchpad, seed: [allTokens[0]])
let known: Set<String> = [allTokens[7]]
let balanceWords = [EVMABI.word(address: owner)]

func fixture() -> MockRPC {
    let rpc = MockRPC()
    rpc.answers[request(factory, TokenScanner.Sel.allTokensLength)] = uint(2)
    rpc.answers[request(factory, TokenScanner.Sel.allTokens, [EVMABI.word(uint: 0)])] = addressWord(allTokens[2])
    rpc.answers[request(factory, TokenScanner.Sel.allTokens, [EVMABI.word(uint: 1)])] = addressWord(allTokens[3])
    rpc.answers[request(pairFactory, TokenScanner.Sel.allPairsLength)] = uint(1)
    rpc.answers[request(pairFactory, TokenScanner.Sel.allPairs, [EVMABI.word(uint: 0)])] = addressWord(pair)
    rpc.answers[request(pair, TokenScanner.Sel.token0)] = addressWord(allTokens[4])
    rpc.answers[request(pair, TokenScanner.Sel.token1)] = addressWord(allTokens[5])
    rpc.answers[request(launchpad, TokenScanner.Sel.tokenCount)] = uint(1)
    rpc.answers[request(launchpad, TokenScanner.Sel.tokens, [EVMABI.word(uint: 0)])] = addressWord(allTokens[6])
    for token in allTokens { rpc.answers[request(token, TokenScanner.Sel.balanceOf, balanceWords)] = uint(0) }
    // Metadata is deliberately unavailable: an absent symbol or decimals must
    // never let a balance guard skip a discovered token.
    return rpc
}

func balances(_ rpc: MockRPC) throws -> [String] {
    try AccountTokenBalanceCheck.balances(owner: owner, sources: sources, knownTokens: known, read: rpc.read)
}

check("normal zero checks every discovered and persisted token") {
    try balances(fixture()) == Array(repeating: "0", count: allTokens.count)
}
check("fresh enumeration reads each distinct balance exactly once") {
    let rpc = fixture()
    _ = try balances(rpc)
    let expected = Set(allTokens.map { request($0, TokenScanner.Sel.balanceOf, balanceWords) })
    let actual = rpc.calls.filter { $0.contains("|0x" + TokenScanner.Sel.balanceOf) }
    return actual.count == allTokens.count && Set(actual) == expected
}
for (index, origin) in ["seed", "wrapped native", "factory first", "factory second", "pool token0", "pool token1", "launchpad", "persisted token"].enumerated() {
    check("\(origin) funds block a zero conclusion despite missing metadata") {
        let rpc = fixture()
        rpc.answers[request(allTokens[index], TokenScanner.Sel.balanceOf, balanceWords)] = uint(123)
        return try balances(rpc).contains("123")
    }
}
check("full uint256 balance stays exact") {
    let rpc = fixture()
    rpc.answers[request(allTokens[0], TokenScanner.Sel.balanceOf, balanceWords)] = "0x" + String(repeating: "f", count: 64)
    return try balances(rpc).contains("115792089237316195423570985008687907853269984665640564039457584007913129639935")
}
check("casing and duplicate sources cannot omit or duplicate a token") {
    let token = address(0xabc)
    let rpc = MockRPC()
    rpc.answers[request(token, TokenScanner.Sel.balanceOf, balanceWords)] = uint(0)
    let values = try AccountTokenBalanceCheck.balances(owner: owner,
        sources: TokenSources(waeth: token.uppercased(), seed: [token, token.uppercased()]),
        knownTokens: [token.uppercased()], read: rpc.read)
    return values == ["0"] && rpc.calls == [request(token, TokenScanner.Sel.balanceOf, balanceWords)]
}
check("owner casing is normalized in balanceOf") {
    let token = allTokens[0], rpc = MockRPC()
    rpc.answers[request(token, TokenScanner.Sel.balanceOf, balanceWords)] = uint(0)
    return try AccountTokenBalanceCheck.balances(owner: owner.uppercased(), sources: TokenSources(seed: [token]), read: rpc.read) == ["0"]
}

let lists: [(String, String, UInt64, String)] = [
    (factory, TokenScanner.Sel.allTokensLength, TokenScanner.maxFactoryTokens, "factory"),
    (pairFactory, TokenScanner.Sel.allPairsLength, TokenScanner.maxPools, "pools"),
    (launchpad, TokenScanner.Sel.tokenCount, TokenScanner.maxLaunches, "launchpad")
]
for (contract, selector, cap, label) in lists {
    blocked("\(label) count above the display cap blocks removal") {
        let rpc = fixture()
        rpc.answers[request(contract, selector)] = uint(cap + 1)
        _ = try balances(rpc)
    }
}
for (contract, selector, cap, label) in lists {
    check("\(label) list at its cap is fully read") {
        let rpc = MockRPC()
        rpc.answers[request(contract, selector)] = uint(cap)
        let itemSelector = label == "factory" ? TokenScanner.Sel.allTokens : (label == "pools" ? TokenScanner.Sel.allPairs : TokenScanner.Sel.tokens)
        for index in 0..<cap {
            let token = address(2_000 + index)
            rpc.answers[request(contract, itemSelector, [EVMABI.word(uint: index)])] = addressWord(token)
            if label == "pools" {
                rpc.answers[request(token, TokenScanner.Sel.token0)] = addressWord(token)
                rpc.answers[request(token, TokenScanner.Sel.token1)] = addressWord(token)
            }
            rpc.answers[request(token, TokenScanner.Sel.balanceOf, balanceWords)] = uint(0)
        }
        let isolated = TokenSources(tokenFactory: label == "factory" ? contract : nil,
            pairFactory: label == "pools" ? contract : nil, launchpad: label == "launchpad" ? contract : nil)
        return try AccountTokenBalanceCheck.balances(owner: owner, sources: isolated, read: rpc.read) == Array(repeating: "0", count: Int(cap))
    }
}

let failingRequests: [(String, String)] = [
    (request(factory, TokenScanner.Sel.allTokensLength), "factory count"),
    (request(factory, TokenScanner.Sel.allTokens, [EVMABI.word(uint: 1)]), "factory item after an earlier success"),
    (request(pairFactory, TokenScanner.Sel.allPairsLength), "pool count"),
    (request(pairFactory, TokenScanner.Sel.allPairs, [EVMABI.word(uint: 0)]), "pool item"),
    (request(pair, TokenScanner.Sel.token0), "pool token0"),
    (request(pair, TokenScanner.Sel.token1), "pool token1"),
    (request(launchpad, TokenScanner.Sel.tokenCount), "launchpad count"),
    (request(launchpad, TokenScanner.Sel.tokens, [EVMABI.word(uint: 0)]), "launchpad item"),
    (request(allTokens[0], TokenScanner.Sel.balanceOf, balanceWords), "discovered balance"),
    (request(allTokens[7], TokenScanner.Sel.balanceOf, balanceWords), "persisted balance")
]
for (key, label) in failingRequests {
    blocked("unavailable \(label) blocks removal") {
        let rpc = fixture()
        rpc.unavailable.insert(key)
        _ = try balances(rpc)
    }
}

let malformedCounts: [(String, String)] = [
    ("0x", "empty"), ("0x01", "short"),
    (uint(1) + EVMABI.word(uint: 0), "extra word"),
    ("0x" + String(repeating: "g", count: 64), "nonhex"),
    ("0x" + String(repeating: "0", count: 47) + "1" + String(repeating: "0", count: 16), "uint64 overflow"),
    (EVMABI.word(uint: 0), "missing hex prefix")
]
for (raw, label) in malformedCounts {
    blocked("malformed \(label) count blocks removal") {
        let rpc = fixture()
        rpc.answers[request(factory, TokenScanner.Sel.allTokensLength)] = raw
        _ = try balances(rpc)
    }
}
let malformedAddresses: [(String, String)] = [
    (uint(0), "zero"),
    ("0x01" + String(repeating: "0", count: 22) + String(allTokens[0].dropFirst(2)), "nonzero padding"),
    ("0x01", "short"), (addressWord(allTokens[0]) + EVMABI.word(address: allTokens[1]), "extra word"),
    ("0x", "empty"), ("0x" + String(repeating: "g", count: 64), "nonhex"),
    (EVMABI.word(address: allTokens[0]), "missing hex prefix")
]
for (raw, label) in malformedAddresses {
    blocked("malformed \(label) list address blocks removal") {
        let rpc = fixture()
        rpc.answers[request(factory, TokenScanner.Sel.allTokens, [EVMABI.word(uint: 0)])] = raw
        _ = try balances(rpc)
    }
}
for (key, label) in [
    (request(pairFactory, TokenScanner.Sel.allPairs, [EVMABI.word(uint: 0)]), "pool address"),
    (request(pair, TokenScanner.Sel.token1), "pool token address"),
    (request(launchpad, TokenScanner.Sel.tokens, [EVMABI.word(uint: 0)]), "launchpad token address")
] {
    blocked("zero \(label) blocks removal") {
        let rpc = fixture()
        rpc.answers[key] = uint(0)
        _ = try balances(rpc)
    }
}
for (raw, label) in [
    ("0x", "empty"), ("0x01", "short"), (uint(0) + EVMABI.word(uint: 0), "extra word"),
    ("0x" + String(repeating: "g", count: 64), "nonhex"), (EVMABI.word(uint: 0), "missing hex prefix")
] {
    blocked("malformed \(label) balance blocks removal") {
        let rpc = fixture()
        rpc.answers[request(allTokens[0], TokenScanner.Sel.balanceOf, balanceWords)] = raw
        _ = try balances(rpc)
    }
}

for (invalidSources, invalidKnown, invalidOwner, label) in [
    (TokenSources(seed: [address(0)]), Set<String>(), owner, "zero seed"),
    (TokenSources(seed: ["0x1234"]), Set<String>(), owner, "short seed"),
    (TokenSources(waeth: address(0)), Set<String>(), owner, "zero wrapped native"),
    (TokenSources(), Set([address(0)]), owner, "zero persisted token"),
    (TokenSources(tokenFactory: address(0)), Set<String>(), owner, "zero factory"),
    (TokenSources(seed: [allTokens[0]]), Set<String>(), address(0), "zero owner"),
    (TokenSources(seed: [allTokens[0]]), Set<String>(), "bad-owner", "malformed owner")
] {
    blocked("\(label) address blocks removal") {
        _ = try AccountTokenBalanceCheck.balances(owner: invalidOwner, sources: invalidSources,
            knownTokens: invalidKnown, read: fixture().read)
    }
}

print("account-removal: \(checks) checks, \(failures) failures")
exit(failures == 0 ? 0 : 1)
