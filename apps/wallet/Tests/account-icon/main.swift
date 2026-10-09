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
        let seedSha256: String
        let svg64Sha256: String
    }
    let domain: String
    let version: UInt8
    let palettes: [String]
    let ink: String
    let backgrounds: [String: String]
    let vectors: [Vector]
}

let path = CommandLine.arguments.dropFirst().first ?? "crates/client/tests/account-icon-vectors.json"
let fixture = try JSONDecoder().decode(Fixture.self, from: Data(contentsOf: URL(fileURLWithPath: path)))
check(fixture.domain == AccountIconSpec.domain && fixture.version == 1, "v1 domain and explicit version")
check(fixture.palettes == AccountIconSpec.palettes && fixture.ink == AccountIconSpec.ink, "frozen drawing colors")
check(fixture.vectors.count == 16, "all sixteen shared vectors")

func sha256(_ data: Data) -> String {
    SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
}

for vector in fixture.vectors {
    guard let spec = AccountIconSpec.of(address: vector.address) else { fatalError("valid shared vector") }
    let expected = vector.features
    check(spec.version == expected.version && spec.palette == expected.palette && spec.layout == expected.layout &&
          spec.shape == expected.shape && spec.rotation == expected.rotation, "feature tuple: \(vector.address)")
    check(sha256(Data(spec.svg64.utf8)) == vector.svg64Sha256, "canonical drawing hash: \(vector.address)")
    let text = Array(vector.address.dropFirst(2))
    let bytes = stride(from: 0, to: 40, by: 2).map { UInt8(String(text[$0...($0 + 1)]), radix: 16)! }
    check(sha256(Data(fixture.domain.utf8) + Data(bytes)) == vector.seedSha256, "raw-byte seed fixture")
    for spelling in [vector.address.uppercased(), String(vector.address.dropFirst(2)),
                     "0X" + vector.address.dropFirst(2), vector.address.replacingOccurrences(of: "a", with: "A")] {
        check(AccountIconSpec.of(address: spelling) == spec, "ASCII case/prefix invariance")
    }
    check(AccountIconSpec.of(address: vector.address, version: 1) == spec, "default remains v1")
    check(spec.isOccupied(0) && !spec.isOccupied(15), "fixed island/sea anchors")
    check(!spec.isOccupied(-1) && !spec.isOccupied(16), "out-of-grid cells are sea")
    check(spec.occupiedCells.count == spec.layout.nonzeroBitCount + 1, "exact occupancy count")
    check(spec.layout < 1 << 14 && spec.palette < 8 && spec.shape < 4 && spec.rotation < 4, "feature ranges")
    var rotated: UInt16 = 0
    for cell in 0..<16 {
        let occupied = cell == 0 || (cell != 15 && expected.layout & (UInt16(1) << (cell - 1)) != 0)
        check(spec.isOccupied(cell) == occupied, "every occupancy bit: \(cell)")
        if occupied {
            let x = cell % 4, y = cell / 4
            let index: Int
            switch expected.rotation {
            case 0: index = cell
            case 1: index = x * 4 + 3 - y
            case 2: index = 15 - cell
            default: index = (3 - x) * 4 + y
            }
            rotated |= UInt16(1) << index
        }
    }
    check(spec.rotatedMask == rotated, "actual clockwise mask")
    check(spec.rotatedMask.nonzeroBitCount == spec.occupiedCells.count, "rotation preserves occupancy")
}

let valid = fixture.vectors[7].address
let invalid: [String?] = [nil, "", "0x", "0X", String(valid.dropLast()), valid + "0",
                         " " + valid, valid + "\n", valid + " ", valid + "\0",
                         "alice.eth", "0x1234…5678", "0x" + String(repeating: "g", count: 40),
                         "0x" + String(repeating: "０", count: 40), "0x" + String(repeating: "١", count: 40),
                         "0х" + valid.dropFirst(2), "0x+" + valid.dropFirst(3),
                         "0x" + String(repeating: "a", count: 39) + "\n"]
for address in invalid { check(AccountIconSpec.of(address: address) == nil, "strict malformed-input rejection") }
for version in UInt8.min...UInt8.max where version != 1 {
    check(AccountIconSpec.of(address: valid, version: version) == nil, "unsupported version \(version)")
}
check(Set(fixture.vectors.map { $0.features.palette }).count == 8, "all palettes covered")
check(Set(fixture.vectors.map { $0.features.shape }).count == 4, "all silhouettes covered")
check(Set(fixture.vectors.map { $0.features.rotation }).count == 4, "all rotations covered")
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
    check(contrast(palette, AccountIconSpec.ink) >= 3, "internal ink contrast >= 3:1")
    for background in fixture.backgrounds.values {
        check(contrast(palette, background) >= 3, "both enclosing-surface contrasts >= 3:1")
    }
}
print("account-icon OK (16 frozen vectors, canonical SVG, strict input/version rejection, occupancy and contrast)")
