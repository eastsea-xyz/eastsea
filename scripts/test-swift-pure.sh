#!/usr/bin/env bash
# Compile and run every pure-Swift test under apps/wallet/Tests with the sources it needs.
# A new Tests/<name> directory must be added to the table below, or scripts/verify.sh will not run it.
# The Aether 0.6.7 bridge (apps/bridge) has its own Sources/Tests: the `bridge` lines at the end.
cd "$(dirname "$0")/.."
W=apps/wallet/Sources; T=apps/wallet/Tests; bad=0
run() { n=$1; shift; files=(); for f in "$@"; do files+=("$W/$f"); done
  if swiftc -o tmp/sw-$n "${files[@]}" $T/$n/main.swift 2>tmp/sw-$n.err && AETHER_AGENT_TEST_TMP=$PWD/tmp ./tmp/sw-$n > tmp/sw-$n.out 2>&1; then echo "OK   $n"; else echo "FAIL $n :: $(head -c 160 tmp/sw-$n.err | tr '\n' ' ') $(tail -2 tmp/sw-$n.out | tr '\n' ' ')"; bad=$((bad+1)); fi; }
run account-history Brand.swift ChainActivity.swift
run assets EarningsModel.swift TokenAssets.swift
run balance-sources Brand.swift EarningsModel.swift ChainActivity.swift BalanceBreakdown.swift EarningsExport.swift
run balance-history BalanceHistory.swift
run browser-origin BrowserOriginPolicy.swift
run browser-permissions SitePermissions.swift
run browser-routing Brand.swift BrowserPolicy.swift
run browser-verify Brand.swift BrowserOriginPolicy.swift BrowserPolicy.swift VerifyBridge.swift
run diagnostic-report Brand.swift Clock.swift NodeWatchdog.swift HealthCheck.swift DiagnosticReport.swift
run earnings EarningsModel.swift
run earnings-export Brand.swift EarningsModel.swift ChainActivity.swift EarningsExport.swift
run fee-confirm EarningsModel.swift
run health-check Brand.swift Clock.swift NodeWatchdog.swift HealthCheck.swift
run history-notice HistoryFailure.swift
run install-location Brand.swift InstallLocation.swift
run key-exposure KeyExposureNotice.swift
run legacy-aether Brand.swift LegacyAether.swift
run network-upgrade Brand.swift NetworkUpgrade.swift
run release-approval ReleaseApproval.swift
run rename-migration DataMigration.swift
run resources ProverFlags.swift
run resend ResendIntent.swift
run storage StorageSetting.swift UnattendedDecision.swift
run reward-status EarningsModel.swift
run proving-badge EarningsModel.swift
run token-guard Brand.swift TokenSend.swift TokenAssets.swift EarningsModel.swift TokenGuard.swift
run token-icon Brand.swift TokenSend.swift TokenAssets.swift EarningsModel.swift TokenGuard.swift TokenIconSpec.swift
run token-send Brand.swift TokenSend.swift TokenAssets.swift EarningsModel.swift
run unattended UnattendedDecision.swift
run tx-status-text TxStatusText.swift
run update-channel UpdateChannel.swift
run update-state Brand.swift DataMigration.swift UpdateTracker.swift
run watchdog Brand.swift Clock.swift NodeWatchdog.swift
# The Aether -> EastSea bridge app.
W=apps/bridge/Sources; T=apps/bridge/Tests
run bridge-plan BridgePlan.swift
exit $bad
