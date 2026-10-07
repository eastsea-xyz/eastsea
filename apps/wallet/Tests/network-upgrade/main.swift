import Foundation

func check(_ condition: Bool, _ message: String) {
    if !condition { print("FAIL: \(message)"); exit(1) }
}

let certifiedAt = Date(timeIntervalSince1970: 1_780_000_000)
let certifiedTimestampMs = UInt64(certifiedAt.timeIntervalSince1970 * 1_000)
let json = """
[{"protocol":4,"activate_at":604801,"emergency":false,"notes":"Improve proofs","certified_height":1,"certified_timestamp_ms":\(certifiedTimestampMs)},
 {"protocol":5,"activate_at":7200,"emergency":true,"notes":"Critical security fix","certified_height":1,"certified_timestamp_ms":\(certifiedTimestampMs)}]
"""
let notices = NetworkUpgrade.parse(json, height: 1, now: certifiedAt)
check(notices.map(\.protocol) == [5, 4], "notices sort by activation height")
check(notices[1].daysLeft(height: 1) == 7, "seven days of blocks")
check(notices[0].daysLeft(height: 7_199) == 1, "partial day rounds up")
check(notices[0].notice(height: 1).contains("Critical security fix"), "signed notes are visible")
check(notices[0].emergency, "emergency flag is visible")
check(notices[0].requiresAppUpdate(supportedProtocol: 3), "newer protocol needs an app update")
check(!notices[1].requiresAppUpdate(supportedProtocol: 4), "supported protocol needs no update")
check(notices[1].updateDeadline(height: 1, now: Date(timeIntervalSince1970: 0)).contains("Update \(Brand.project) before"), "unsupported protocol has an update deadline")
let activated = json.replacingOccurrences(of: "\"certified_height\":1", with: "\"certified_height\":604801")
check(NetworkUpgrade.parse(activated, height: 604_801, now: certifiedAt).isEmpty, "activated upgrades disappear")
check(NetworkUpgrade.parse("broken", height: 0).isEmpty, "malformed status does not show a false notice")

let uncertified = """
[{"protocol":4,"activate_at":604801,"emergency":false,"notes":"Improve proofs"}]
"""
check(NetworkUpgrade.parse(uncertified, height: 1, now: certifiedAt).isEmpty,
      "an upgrade without a certified head is not current")
check(NetworkUpgrade.parse(json, height: 2, now: certifiedAt).isEmpty,
      "a certified head below the verified height floor is not current")

let stale = """
[{"protocol":4,"activate_at":604801,"emergency":false,"notes":"Improve proofs","certified_height":1,"certified_timestamp_ms":\(certifiedTimestampMs - 660_000)}]
"""
check(NetworkUpgrade.parse(stale, height: 1, now: certifiedAt).isEmpty,
      "an old certified head is not current")
check(notices[1].updateDeadline(height: 1, now: certifiedAt) ==
      notices[1].updateDeadline(height: 1, now: certifiedAt.addingTimeInterval(86_400)),
      "the upgrade deadline does not move with the wallet clock")
let deadline = certifiedAt.addingTimeInterval(604_800)
check(notices[1].updateDeadline(height: 600_000, now: certifiedAt)
        .contains(deadline.formatted(date: .abbreviated, time: .shortened)),
      "the deadline uses the certified timestamp and height")
check(notices[1].daysLeft(height: 600_000) == 7,
      "the countdown uses certified height")
print("OK")
