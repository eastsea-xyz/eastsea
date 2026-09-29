import Foundation

func check(_ condition: Bool, _ message: String) {
    if !condition { print("FAIL: \(message)"); exit(1) }
}

let json = """
[{"protocol":4,"activate_at":604801,"emergency":false,"notes":"Improve proofs"},
 {"protocol":5,"activate_at":7200,"emergency":true,"notes":"Critical security fix"}]
"""
let notices = NetworkUpgrade.parse(json, height: 1)
check(notices.map(\.protocol) == [5, 4], "notices sort by activation height")
check(notices[1].daysLeft(height: 1) == 7, "seven days of blocks")
check(notices[0].daysLeft(height: 7_199) == 1, "partial day rounds up")
check(notices[0].notice(height: 1).contains("Critical security fix"), "signed notes are visible")
check(notices[0].emergency, "emergency flag is visible")
check(notices[0].requiresAppUpdate(supportedProtocol: 3), "newer protocol needs an app update")
check(!notices[1].requiresAppUpdate(supportedProtocol: 4), "supported protocol needs no update")
check(notices[1].updateDeadline(height: 1, now: Date(timeIntervalSince1970: 0)).contains("Update \(Brand.project) before"), "unsupported protocol has an update deadline")
check(NetworkUpgrade.parse(json, height: 604_801).isEmpty, "activated upgrades disappear")
check(NetworkUpgrade.parse("broken", height: 0).isEmpty, "malformed status does not show a false notice")
print("OK")
