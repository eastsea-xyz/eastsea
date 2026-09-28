// Checks the `aether_rewardStatus` parsing behind the Network page's standing
// card (no app, no node):
//   swiftc -o /tmp/reward-status-check apps/wallet/Sources/EarningsModel.swift \
//     apps/wallet/Tests/reward-status/main.swift && /tmp/reward-status-check
import Foundation
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); exit(1) } }

// The testnet's whole answer: the card must show nothing new for it.
let off = RewardStatus(json: ["enabled": false])
check(!off.enabled, "testnet disabled")
check(off.operatorsOnline == 0 && off.warmupPercent == nil && off.expectedShareWei == nil, "testnet bare")

// Enabled, no operator named: N and the cap still come from the chain.
let noOperator = RewardStatus(json: [
    "enabled": true, "operators_online_last_epoch": 20, "max_share": 16,
])
check(noOperator.enabled, "enabled")
check(noOperator.operatorsOnline == 20, "N")
check(noOperator.maxShare == 16, "cap")
check(noOperator.warmupPercent == nil && noOperator.warmupDaysLeft == nil, "no macs")

// With the operator: this Mac's best warm-up counts ("정상 몫의 %" =
// (14 + level) / 28), and days left runs one level a day down to zero.
let withOperator = RewardStatus(json: [
    "enabled": true,
    "operators_online_last_epoch": 9,
    "max_share": 16,
    "operator": [
        "macs": [
            ["index": 1, "warmup_level": 2, "warmup_percent": 57],
            ["index": 2, "warmup_level": 4, "warmup_percent": 64],
        ],
        "weight_last_epoch": 792,
        "expected_share_last_epoch": "140000000000000000",
        "received_last_distribution": "140000000000000000",
        "capped": true,
    ],
] as [String: Any])
check(withOperator.warmupPercent == 64, "best mac warm-up percent")
check(withOperator.warmupDaysLeft == 10, "days left from the best level")
check(withOperator.expectedShareWei == "140000000000000000", "expected share")
check(withOperator.capped, "capped")

// Full warm-up: level 14 (100%) leaves no days, so the card drops the line.
let full = RewardStatus(json: [
    "enabled": true,
    "operator": ["macs": [["warmup_level": 14, "warmup_percent": 100]]] as [String: Any],
] as [String: Any])
check(full.warmupDaysLeft == 0, "full share")
check(full.warmupPercent == 100, "full percent")

// Numbers may arrive as strings too (the same parser as RewardEntry).
let strings = RewardStatus(json: [
    "enabled": true, "operators_online_last_epoch": "3", "max_share": "16",
] as [String: Any])
check(strings.operatorsOnline == 3 && strings.maxShare == 16, "string numbers")

print("OK")
