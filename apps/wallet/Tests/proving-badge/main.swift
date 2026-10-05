import Foundation

func check(_ condition: Bool, _ message: String) {
    if !condition { print("FAIL", message); exit(1) }
}

check(ProvingBadgeText.line(proofsFailing: true, today: "0") == "Proofs failing", "failure overrides zero earnings")
check(ProvingBadgeText.line(proofsFailing: false, today: "1.5") == "Proving · +1.5 today", "healthy badge")
check(ProvingBadgeText.failing(reported: true, proverRunning: true, runningSeconds: 20), "reported rejection")
check(!ProvingBadgeText.failing(reported: false, proverRunning: true, runningSeconds: 20), "healthy prover")
check(ProvingBadgeText.failing(reported: false, proverRunning: false, runningSeconds: 2), "stopped prover")
check(!ProvingBadgeText.failing(reported: false, proverRunning: nil, runningSeconds: 5), "startup grace")
check(ProvingBadgeText.failing(reported: false, proverRunning: nil, runningSeconds: 16), "status missing after startup")
print("OK")
