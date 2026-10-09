#!/usr/bin/env bash
# Compile and run every pure-Swift test under apps/wallet/Tests with the sources it needs.
# A new Tests/<name> directory must be added to the table below, or scripts/verify.sh will not run it.
# The Aether 0.6.7 bridge (apps/bridge) has its own Sources/Tests: the `bridge` lines at the end.
set -uo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root" || exit 1
mkdir -p "$root/tmp/swift-module-cache"
export TMPDIR="$root/tmp"
localizations="$root/tmp/wallet-languages/WalletLocalizations.bundle"
/usr/bin/python3 scripts/wallet-l10n.py prepare-tests --out "$localizations" || exit 1
# Optional positional test names select a subset; no arguments still runs every test.
selected="$*"
manifest=$(mktemp "$root/tmp/swift-tests.XXXXXX")
trap 'rm -f "$manifest"' EXIT
W=apps/wallet/Sources; T=apps/wallet/Tests; known=""
run() {
  local n=$1; shift
  known="$known $n"
  if [ "$n" = update-daemon ] || [ "$n" = update-daemon-tree ]; then
    local fixture=update-daemon main=apps/wallet/Tests/update-daemon/main.swift
    if [ "$n" = update-daemon-tree ]; then
      fixture=update-listener; main=apps/wallet/Tests/update-daemon-tree/main.swift
      known="$known $fixture"
    fi
    if [ -n "$selected" ] && [[ " $selected " != *" $n "* ]] && [[ " $selected " != *" $fixture "* ]]; then return; fi
    printf '%s\t' "$fixture" apps/wallet/Sources/NodeReleaseIdentity.swift "$main" --command bash "scripts/test-$fixture.sh" >> "$manifest"
    printf '\n' >> "$manifest"
    return
  fi
  if [ -n "$selected" ] && [[ " $selected " != *" $n "* ]]; then return; fi
  local files=() compiler_flags=(-Onone)
  for f in "$@"; do files+=("$W/$f"); done
  if [ "$W" = apps/wallet/Sources ]; then
    # AppLanguage/Brand stay Foundation-only; agent/bridge keep existing behavior.
    if [[ " $* " != *" AppLanguage.swift "* ]]; then files+=("$W/AppLanguage.swift"); fi
    files+=(apps/wallet/Tests/LocalizationTestSupport.swift)
  fi
  if [ "$n" = rename-migration ]; then
    # Keep assertions/preconditions enabled for the optimized large migration fixtures.
    compiler_flags=(-O -assert-config Debug)
  fi
  printf '%s\t' "$n" "${compiler_flags[@]}" -module-cache-path "$root/tmp/swift-module-cache" "${files[@]}" "$T/$n/main.swift" >> "$manifest"
  printf '\n' >> "$manifest"
}

run multi-account AccountStore.swift
run account-selection AccountStore.swift AccountDataStore.swift AccountControls.swift
run account-isolation AccountStore.swift AccountDataStore.swift AccountControls.swift
run account-retire-guard AccountStore.swift AccountDataStore.swift AccountControls.swift
run account-data AccountDataStore.swift
run account-removal EarningsModel.swift TokenAssets.swift AccountRemovalBalance.swift
run account-operations WalletOperationGate.swift
run account-history Brand.swift ChainActivity.swift
run assets EarningsModel.swift TokenAssets.swift
run balance-sources Brand.swift EarningsModel.swift ChainActivity.swift BalanceBreakdown.swift EarningsExport.swift
run balance-history BalanceHistory.swift
run browser-origin BrowserOriginPolicy.swift
run browser-permissions SitePermissions.swift
run browser-routing Brand.swift BrowserPolicy.swift
run browser-verify Brand.swift BrowserOriginPolicy.swift BrowserPolicy.swift VerifyBridge.swift
run candidate-eligibility CandidateEligibilityText.swift
run diagnostic-report Brand.swift Clock.swift NodeWatchdog.swift NodeStopReason.swift HealthCheck.swift DiagnosticReport.swift
run earnings EarningsModel.swift
run earnings-export Brand.swift EarningsModel.swift ChainActivity.swift EarningsExport.swift
run fee-confirm EarningsModel.swift
run health-check Brand.swift Clock.swift NodeWatchdog.swift NodeStopReason.swift HealthCheck.swift
run history-notice HistoryFailure.swift
run install-location Brand.swift InstallLocation.swift
run key-exposure KeyExposureNotice.swift
run legacy-aether Brand.swift LegacyAether.swift
run network-upgrade Brand.swift NetworkUpgrade.swift
run node-stop Brand.swift Clock.swift NodeWatchdog.swift NodeStopReason.swift
run block-data Brand.swift Clock.swift NodeWatchdog.swift NodeStopReason.swift UnattendedDecision.swift ArchiveMeasurement.swift BlockDataLocation.swift KeySafety.swift DataMigration.swift BlockDataMove.swift NodeStorageMove.swift
run prover-menu ProverMenuText.swift
run localization ProverMenuText.swift
run key-safety Brand.swift Clock.swift NodeWatchdog.swift NodeStopReason.swift ArchiveMeasurement.swift BlockDataLocation.swift KeySafety.swift
run release-approval ReleaseApproval.swift
run rename-migration DataMigration.swift
run resources ProverFlags.swift
run resend ResendIntent.swift
run tx-track TxTrack.swift
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
run update-window UpdateWindow.swift
run watchdog Brand.swift Clock.swift NodeWatchdog.swift
# The agent's payment history (aether-agent): pending context, drops, receipts.
W=apps/agent/Sources; T=apps/agent/Tests
run history History.swift AgentPolicy.swift
# The Aether -> EastSea bridge app.
W=apps/bridge/Sources; T=apps/bridge/Tests
run bridge-plan BridgePlan.swift
# Native identity fixtures are compiled in the same bounded batch. Their scripts
# still create fresh signed executable identities and own their process cleanup.
if [ "$(uname -s)" = Darwin ]; then
run update-daemon NodeReleaseIdentity.swift
run update-daemon-tree NodeReleaseIdentity.swift
fi
for requested in $selected; do
  if [[ " $known " != *" $requested "* ]]; then echo "unknown Swift test: $requested" >&2; exit 2; fi
done
python3 scripts/swift-test-cache.py --manifest "$manifest"
exit $?
