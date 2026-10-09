import Foundation

/// A fresh token read for removing or hiding an account. Display discovery may
/// skip bad metadata and truncate lists; neither can establish a zero balance.
enum AccountTokenBalanceCheck {
    enum Failure: Error {
        case invalidAddress, malformedReturn, enumerationLimit
    }

    static func balances(owner: String, sources: TokenSources, knownTokens: Set<String> = [],
                         read: TokenScanner.Read) throws -> [String] {
        let owner = try validAddress(owner)
        var tokens = Set(try knownTokens.map(validAddress))
        for token in sources.seed { tokens.insert(try validAddress(token)) }
        if let token = sources.waeth { tokens.insert(try validAddress(token)) }

        if let factory = sources.tokenFactory {
            let factory = try validAddress(factory)
            let count = try listCount(factory, selector: TokenScanner.Sel.allTokensLength,
                                      cap: TokenScanner.maxFactoryTokens, read: read)
            for index in 0..<count {
                tokens.insert(try returnedAddress(read(factory, EVMABI.call(TokenScanner.Sel.allTokens, [EVMABI.word(uint: index)]))))
            }
        }
        if let factory = sources.pairFactory {
            let factory = try validAddress(factory)
            let count = try listCount(factory, selector: TokenScanner.Sel.allPairsLength,
                                      cap: TokenScanner.maxPools, read: read)
            for index in 0..<count {
                let pair = try returnedAddress(read(factory, EVMABI.call(TokenScanner.Sel.allPairs, [EVMABI.word(uint: index)])))
                for selector in [TokenScanner.Sel.token0, TokenScanner.Sel.token1] {
                    tokens.insert(try returnedAddress(read(pair, EVMABI.call(selector))))
                }
            }
        }
        if let launchpad = sources.launchpad {
            let launchpad = try validAddress(launchpad)
            let count = try listCount(launchpad, selector: TokenScanner.Sel.tokenCount,
                                      cap: TokenScanner.maxLaunches, read: read)
            for index in 0..<count {
                tokens.insert(try returnedAddress(read(launchpad, EVMABI.call(TokenScanner.Sel.tokens, [EVMABI.word(uint: index)]))))
            }
        }

        let call = EVMABI.call(TokenScanner.Sel.balanceOf, [EVMABI.word(address: owner)])
        return try tokens.sorted().map { token in
            let raw = try read(token, call)
            _ = try singleWord(raw)
            return try EVMABI.uint(raw)
        }
    }

    private static func listCount(_ contract: String, selector: String, cap: UInt64,
                                  read: TokenScanner.Read) throws -> UInt64 {
        let raw = try read(contract, EVMABI.call(selector))
        _ = try singleWord(raw)
        let count = try EVMABI.uint64(raw)
        guard count <= cap else { throw Failure.enumerationLimit }
        return count
    }

    private static func returnedAddress(_ raw: String) throws -> String {
        let word = try singleWord(raw)
        guard word.prefix(24).allSatisfy({ $0 == "0" }) else { throw Failure.malformedReturn }
        return try validAddress(EVMABI.address(raw))
    }

    private static func singleWord(_ raw: String) throws -> String {
        guard raw.hasPrefix("0x"), raw.count == 66,
              raw.dropFirst(2).allSatisfy({ $0.isASCII && $0.isHexDigit }) else {
            throw Failure.malformedReturn
        }
        return try EVMABI.words(raw)[0]
    }

    private static func validAddress(_ input: String) throws -> String {
        let address = input.lowercased()
        guard address.count == 42, address.hasPrefix("0x"),
              address.dropFirst(2).allSatisfy({ $0.isASCII && $0.isHexDigit }),
              address.dropFirst(2).contains(where: { $0 != "0" }) else {
            throw Failure.invalidAddress
        }
        return address
    }
}
