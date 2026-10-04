#!/usr/bin/env bash
# Compile and run every pure-Swift test under apps/wallet/Tests with the sources it needs.
# A new Tests/<name> directory must be added to the table below, or scripts/verify.sh will not run it.
cd "$(dirname "$0")/.."
W=apps/wallet/Sources; T=apps/wallet/Tests; bad=0
run() { n=$1; shift; files=(); for f in "$@"; do files+=("$W/$f"); done
  if swiftc -o tmp/sw-$n "${files[@]}" $T/$n/main.swift 2>tmp/sw-$n.err && AETHER_AGENT_TEST_TMP=$PWD/tmp ./tmp/sw-$n > tmp/sw-$n.out 2>&1; then echo "OK   $n"; else echo "FAIL $n :: $(head -c 160 tmp/sw-$n.err | tr '\n' ' ') $(tail -2 tmp/sw-$n.out | tr '\n' ' ')"; bad=$((bad+1)); fi; }
run account-history Brand.swift ChainActivity.swift
run assets EarningsModel.swift TokenAssets.swift
run balance-history BalanceHistory.swift
run earnings EarningsModel.swift
run install-location Brand.swift InstallLocation.swift
run network-upgrade Brand.swift NetworkUpgrade.swift
run release-approval ReleaseApproval.swift
run resources ProverFlags.swift
run reward-status EarningsModel.swift
run token-guard Brand.swift TokenSend.swift TokenAssets.swift EarningsModel.swift TokenGuard.swift
run token-icon Brand.swift TokenSend.swift TokenAssets.swift EarningsModel.swift TokenGuard.swift TokenIconSpec.swift
run token-send Brand.swift TokenSend.swift TokenAssets.swift EarningsModel.swift
run update-state Brand.swift UpdateTracker.swift
run watchdog Brand.swift Clock.swift NodeWatchdog.swift
exit $bad
