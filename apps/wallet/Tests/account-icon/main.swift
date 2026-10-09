// The frozen shared vectors are independently checked by Swift, JS and Rust.
// Run: scripts/test-swift-pure.sh account-icon
import CryptoKit
import Foundation

func check(_ condition: Bool, _ message: String) {
    if !condition { print("FAIL", message); exit(1) }
}

struct Fixture: Decodable {
    struct Features: Decodable {
        let version: UInt8
        let palette: UInt8
        let layout: UInt16
        let shape: UInt8
        let rotation: UInt8
    }
    struct Vector: Decodable {
        let address: String
        let features: Features
        let silhouetteClass: UInt8
        let seedSha256: String
        let svg16Sha256: String
        let svg32Sha256: String
        let svg64Sha256: String
    }
    let domain: String
    let version: UInt8
    let palettes: [AccountIconSpec.Palette]
    let silhouettes: [AccountIconSpec.Silhouette]
    let backgrounds: [String: String]
    let vectors: [Vector]
}

let path = CommandLine.arguments.dropFirst().first ?? "crates/client/tests/account-icon-vectors.json"
let fixture = try JSONDecoder().decode(Fixture.self, from: Data(contentsOf: URL(fileURLWithPath: path)))
check(fixture.domain == AccountIconSpec.domain && fixture.version == 3, "v2 seed domain and explicit v3 rendering version")
check(fixture.palettes == AccountIconSpec.palettes && fixture.palettes.count == 16, "sixteen frozen gradient color pairs")
check(fixture.silhouettes == AccountIconSpec.silhouettes && fixture.silhouettes.count == 16, "sixteen frozen coastlines")
check(Set(fixture.silhouettes.map(\.path)).count == 16, "coastline classes have distinct paths")
check(fixture.vectors.count == 16, "all sixteen shared vectors")

func sha256(_ data: Data) -> String {
    SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
}

for vector in fixture.vectors {
    guard let spec = AccountIconSpec.of(address: vector.address) else { fatalError("valid shared vector") }
    let expected = vector.features
    check(spec.version == expected.version && spec.palette == expected.palette && spec.layout == expected.layout &&
          spec.shape == expected.shape && spec.rotation == expected.rotation, "feature tuple: \(vector.address)")
    check(spec.silhouetteClass == vector.silhouetteClass, "coastline class: \(vector.address)")
    guard let svg16 = spec.svg(size: 16), let svg32 = spec.svg(size: 32) else { fatalError("valid drawing size") }
    check(sha256(Data(svg16.utf8)) == vector.svg16Sha256, "canonical 16px drawing hash: \(vector.address)")
    check(sha256(Data(svg32.utf8)) == vector.svg32Sha256, "canonical 32px drawing hash: \(vector.address)")
    check(sha256(Data(spec.svg64.utf8)) == vector.svg64Sha256, "canonical drawing hash: \(vector.address)")
    check(svg16.components(separatedBy: "<path ").count - 1 == 1, "16px contains one bold silhouette")
    check(spec.svg(size: 31)!.components(separatedBy: "<path ").count - 1 == 1, "simplification continues below 32px")
    check(svg32.components(separatedBy: "<path ").count - 1 == 3, "32px adds two unequal satellite islands")
    check(spec.svg(size: 0) == nil && spec.svg(size: -1) == nil, "nonpositive drawing sizes rejected")
    let text = Array(vector.address.dropFirst(2))
    let bytes = stride(from: 0, to: 40, by: 2).map { UInt8(String(text[$0...($0 + 1)]), radix: 16)! }
    check(sha256(Data(fixture.domain.utf8) + Data(bytes)) == vector.seedSha256, "raw-byte seed fixture")
    for spelling in [vector.address.uppercased(), String(vector.address.dropFirst(2)),
                     "0X" + vector.address.dropFirst(2), vector.address.replacingOccurrences(of: "a", with: "A")] {
        check(AccountIconSpec.of(address: spelling) == spec, "ASCII case/prefix invariance")
    }
    check(AccountIconSpec.of(address: vector.address, version: 3) == spec, "default is v3")
    check(spec.layout < 1 << 14 && spec.palette < 16 && spec.shape < 4 && spec.rotation < 4, "feature ranges")
    check(spec.silhouetteClass < 16, "coastline class range")
}

let valid = fixture.vectors[7].address
let invalid: [String?] = [nil, "", "0x", "0X", String(valid.dropLast()), valid + "0",
                         " " + valid, valid + "\n", valid + " ", valid + "\0",
                         "alice.eth", "0x1234…5678", "0x" + String(repeating: "g", count: 40),
                         "0x" + String(repeating: "０", count: 40), "0x" + String(repeating: "١", count: 40),
                         "0х" + valid.dropFirst(2), "0x+" + valid.dropFirst(3),
                         "0x" + String(repeating: "a", count: 39) + "\n"]
for address in invalid { check(AccountIconSpec.of(address: address) == nil, "strict malformed-input rejection") }
for version in UInt8.min...UInt8.max where version != 3 {
    check(AccountIconSpec.of(address: valid, version: version) == nil, "unsupported version \(version)")
}
let similar = fixture.vectors[8].address
check(valid.prefix(10) == similar.prefix(10) && valid.suffix(8) == similar.suffix(8), "matching displayed ends")
check(AccountIconSpec.of(address: valid) != AccountIconSpec.of(address: similar), "full address influences icon")
check(AccountIconSpec.of(address: fixture.vectors[0].address) != AccountIconSpec.of(address: fixture.vectors[2].address),
      "single-bit input change")

func luminance(_ hex: String) -> Double {
    let value = UInt32(hex.dropFirst(), radix: 16)!
    let channels = [16, 8, 0].map { shift -> Double in
        let s = Double((value >> shift) & 255) / 255
        return s <= 0.04045 ? s / 12.92 : pow((s + 0.055) / 1.055, 2.4)
    }
    return channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722
}

func contrast(_ a: String, _ b: String) -> Double {
    let x = luminance(a), y = luminance(b)
    return (max(x, y) + 0.05) / (min(x, y) + 0.05)
}

for palette in AccountIconSpec.palettes {
    for color in [palette.start, palette.end] {
        check(contrast(color, palette.ink) >= 3, "internal endpoint/ink contrast >= 3:1")
        for background in fixture.backgrounds.values {
            check(contrast(color, background) >= 3, "both enclosing-surface endpoint contrasts >= 3:1")
        }
    }
}
print("account-icon OK (16 frozen vectors, per-size canonical SVG, strict input/version rejection, coastlines and gradient contrast)")
