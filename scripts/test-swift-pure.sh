#!/usr/bin/env bash
# Compile and run every pure-Swift test under apps/wallet/Tests with the sources it needs.
# A new Tests/<name> directory must be added to the table below, or scripts/verify.sh will not run it.
# The Aether 0.6.7 bridge (apps/bridge) has its own Sources/Tests: the `bridge` lines at the end.
set -uo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root" || exit 1
mkdir -p "$root/tmp"
export TMPDIR="$root/tmp"
# Optional names retain the original subset interface. A path list selects from
# the same source table; --list reads it without preparing bundles or compiling.
selected=""; affected_file=""; list_only=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --affected-file)
      if [ "$#" -lt 2 ]; then echo "--affected-file requires a newline path-list file" >&2; exit 2; fi
      affected_file="$2"; shift 2 ;;
    --list) list_only=1; shift ;;
    --help|-h)
      echo "usage: scripts/test-swift-pure.sh [--list] [--affected-file PATH_LIST] [TEST ...]"
      exit 0 ;;
    --) shift; if [ "$#" -gt 0 ]; then selected="$selected $*"; fi; break ;;
    --*) echo "unknown Swift test option: $1" >&2; exit 2 ;;
    *) selected="$selected $1"; shift ;;
  esac
done
if [ -n "$affected_file" ] && [ -n "$selected" ]; then
  echo "select Swift tests with either --affected-file or positional names" >&2; exit 2
fi
manifest=$(mktemp "$root/tmp/swift-tests.XXXXXX") || exit 1
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
  if [ "$n" = rename-migration ] || [ "$n" = hash-memory ] || [ "$n" = block-data-progress ]; then
    # Large file fixtures hash hundreds of MB or more with the release code.
    # Keep assertions/preconditions enabled for the optimized large fixtures.
    compiler_flags=(-O -assert-config Debug)
  fi
  if [ "$n" = chain-release-sparkle ]; then
    local framework="${SPARKLE_FRAMEWORK_DIR:-$root/apps/wallet/build/SourcePackages/artifacts/sparkle/Sparkle/Sparkle.xcframework/macos-arm64_x86_64}"
    compiler_flags+=(-F "$framework" -framework Sparkle -Xlinker -rpath -Xlinker "$framework")
  fi
  printf '%s\t' "$n" "${compiler_flags[@]}" -module-cache-path "$root/tmp/swift-module-cache" "${files[@]}" "$T/$n/main.swift" >> "$manifest"
  if [ "$n" = chain-release-sparkle ]; then
    printf '%s\t' --command bash scripts/test-chain-release-sparkle.sh >> "$manifest"
  fi
  printf '\n' >> "$manifest"
}

run multi-account AccountStore.swift
run account-icon AccountIconSpec.swift
run account-selection AccountStore.swift AccountDataStore.swift AccountControls.swift
run account-isolation AccountStore.swift AccountDataStore.swift AccountControls.swift
run account-retire-guard AccountStore.swift AccountDataStore.swift AccountControls.swift
run account-data AccountDataStore.swift
run account-removal EarningsModel.swift TokenAssets.swift AccountRemovalBalance.swift
run account-operations WalletOperationGate.swift
run app-content AppContent.swift
run app-identity AppBrowserIdentity.swift
run sea-url SeaURL.swift
run sea-resolution SeaURL.swift SeaNameResolver.swift SeaRegistryReader.swift
run account-history Brand.swift ChainActivity.swift
run assets EarningsModel.swift TokenAssets.swift
run app-search AppSearch.swift BrowserOriginPolicy.swift
run sea-search SeaSearch.swift SeaAppLink.swift BrowserInput.swift SeaURL.swift
run balance-sources Brand.swift EarningsModel.swift ChainActivity.swift BalanceBreakdown.swift EarningsExport.swift
run balance-history BalanceHistory.swift
run browser-origin BrowserOriginPolicy.swift
run browser-plus BrowserInput.swift BrowserData.swift BrowserConfusables.swift
run browser-permissions SitePermissions.swift
run browser-routing Brand.swift BrowserPolicy.swift
run dapp-signing Brand.swift BrowserPolicy.swift EarningsModel.swift DappSigning.swift
run browser-verify Brand.swift BrowserOriginPolicy.swift BrowserPolicy.swift VerifyBridge.swift
run candidate-eligibility CandidateEligibilityText.swift
run country-sharing LivePresence.swift UnattendedDecision.swift
run diagnostic-report Brand.swift Clock.swift NodeWatchdog.swift NodeStopReason.swift HealthCheck.swift DiagnosticReport.swift
run design-effects DesignTokens.swift Design/DesignEffects.swift Design/DesignEventEffect.swift Design/BalanceCountUp.swift Design/RewardShine.swift Design/PresentationMotion.swift Design/NavyPlateDepth.swift Design/SuccessFeedback.swift Design/NodeStatusPulse.swift Design/DesignSurface.swift Design/MenuBarPanel.swift
run earnings EarningsModel.swift
run earnings-export Brand.swift EarningsModel.swift ChainActivity.swift EarningsExport.swift
run fee-confirm EarningsModel.swift
run health-check Brand.swift Clock.swift NodeWatchdog.swift NodeStopReason.swift HealthCheck.swift
run history-notice HistoryFailure.swift
run install-location Brand.swift InstallLocation.swift
run launch-tracker LaunchTracker.swift
run key-exposure KeyExposureNotice.swift
run legacy-aether Brand.swift LegacyAether.swift
run live-presence LivePresence.swift UnattendedDecision.swift
run live-globe LivePresence.swift LiveGlobePresence.swift
run live-globe-bundle LiveGlobeBundlePolicy.swift
run network-upgrade Brand.swift NetworkUpgrade.swift
run node-stop Brand.swift Clock.swift NodeWatchdog.swift NodeStopReason.swift
run block-data Brand.swift Clock.swift NodeWatchdog.swift NodeStopReason.swift UnattendedDecision.swift ArchiveMeasurement.swift BlockDataLocation.swift KeySafety.swift DataMigration.swift BlockDataMove.swift NodeStorageMove.swift
run block-data-progress DataMigration.swift
run block-data-full-volume Brand.swift Clock.swift NodeWatchdog.swift NodeStopReason.swift ArchiveMeasurement.swift BlockDataLocation.swift KeySafety.swift DataMigration.swift BlockDataMove.swift
run prover-menu ProverMenuText.swift
run public-read PublicReadSettings.swift
run localization ProverMenuText.swift
run key-safety Brand.swift Clock.swift NodeWatchdog.swift NodeStopReason.swift ArchiveMeasurement.swift BlockDataLocation.swift KeySafety.swift
run release-approval ReleaseApproval.swift
run rename-migration DataMigration.swift
run hash-memory DataMigration.swift ReleaseUpdateGate.swift ReleaseApproval.swift ChainRelease.swift ReleaseArtifact.swift UpdateChannel.swift
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
run update-channel ReleaseApproval.swift UpdateChannel.swift
run chain-release ReleaseApproval.swift ChainRelease.swift ReleaseArtifact.swift UpdateChannel.swift
run update-state Brand.swift DataMigration.swift UpdateTracker.swift
run update-window UpdateWindow.swift
run watchdog Brand.swift Clock.swift NodeWatchdog.swift
run wallet-push WalletPush.swift
run wallet-push-wiring
# The agent's payment history (aether-agent): pending context, drops, receipts.
W=apps/agent/Sources; T=apps/agent/Tests
run history History.swift AgentPolicy.swift
# The Aether -> EastSea bridge app.
W=apps/bridge/Sources; T=apps/bridge/Tests
run bridge-plan BridgePlan.swift
# Native identity fixtures are compiled in the same bounded batch. Their scripts
# still create fresh signed executable identities and own their process cleanup.
if [ "$(uname -s)" = Darwin ]; then
  W=apps/wallet/Sources; T=apps/wallet/Tests
  known="$known country-wrapper"
  if [ -z "$selected" ] || [[ " $selected " == *" country-wrapper "* ]]; then
    printf '%s\t' country-wrapper -Onone -module-cache-path "$root/tmp/swift-module-cache" apps/wallet/Sources/LivePresence.swift apps/wallet/Sources/UnattendedDecision.swift apps/wallet/Sources/AppLanguage.swift apps/wallet/Tests/LocalizationTestSupport.swift apps/wallet/Tests/country-sharing/main.swift --command bash apps/wallet/Tests/country-sharing/test-wrapper.sh >> "$manifest"
    printf '\n' >> "$manifest"
  fi
run update-daemon NodeReleaseIdentity.swift
run update-daemon-tree NodeReleaseIdentity.swift
run chain-release-sparkle ReleaseApproval.swift UpdateChannel.swift ChainRelease.swift ReleaseArtifact.swift ReleaseUpdateGate.swift
fi
for requested in $selected; do
  if [[ " $known " != *" $requested "* ]]; then echo "unknown Swift test: $requested" >&2; exit 2; fi
done
cache_command=(python3 scripts/swift-test-cache.py --manifest "$manifest")
if [ -n "$affected_file" ]; then cache_command+=(--affected-file "$affected_file"); fi
if [ "$list_only" -eq 1 ]; then cache_command+=(--list); fi
"${cache_command[@]}"
exit $?
