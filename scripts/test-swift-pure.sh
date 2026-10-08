#!/usr/bin/env bash
# Compile and run every pure-Swift test under apps/wallet/Tests with the sources it needs.
# A new Tests/<name> directory must be added to the table below, or scripts/verify.sh will not run it.
# The Aether 0.6.7 bridge (apps/bridge) has its own Sources/Tests: the `bridge` lines at the end.
set -uo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
mkdir -p "$root/tmp/swift-module-cache"
export TMPDIR="$root/tmp"
localizations="$root/tmp/wallet-languages/WalletLocalizations.bundle"
/usr/bin/python3 scripts/wallet-l10n.py prepare-tests --out "$localizations" || exit 1
compile_gate="$HOME/.claude/playbooks/aether-team/wait-compile.sh"
W=apps/wallet/Sources; T=apps/wallet/Tests; bad=0
run() {
  n=$1; shift
  files=()
  for f in "$@"; do files+=("$W/$f"); done
  if [ "$W" = apps/wallet/Sources ]; then
    # AppLanguage/Brand stay Foundation-only. The helper never reaches the
    # agent or bridge test modules, which keep their existing behavior.
    if [[ " $* " != *" AppLanguage.swift "* ]]; then files+=("$W/AppLanguage.swift"); fi
    files+=(apps/wallet/Tests/LocalizationTestSupport.swift)
  fi
  : > "tmp/sw-$n.err"
  : > "tmp/sw-$n.out"
  compiler_flags=(-Onone)
  if [ "$n" = rename-migration ]; then
    # Large migration fixtures hash hundreds of MB with the release code.
    # Optimize that code while keeping Swift assertions and preconditions on.
    compiler_flags=(-O -assert-config Debug)
  fi
  # A separate shell owns each compile slot and exits with the compiler.
  # Reusing this runner's PID would retain multiple slots and deadlock it.
  if (
      if [ -x "$compile_gate" ]; then
        perl -e 'alarm 1200; exec @ARGV' "$compile_gate" || exit 125
      fi
      swiftc "${compiler_flags[@]}" -module-cache-path "$root/tmp/swift-module-cache" -o "tmp/sw-$n" "${files[@]}" "$T/$n/main.swift"
    ) 2>"tmp/sw-$n.err" \
      && AETHER_AGENT_TEST_TMP="$root/tmp" WALLET_TEST_BUNDLE="$localizations" "./tmp/sw-$n" >"tmp/sw-$n.out" 2>&1; then
    echo "OK   $n"
  else
    if [ "$?" = 125 ]; then
      echo "compile slot wait exceeded 20 minutes; stop the lane and report remaining gates" >&2
      exit 125
    fi
    echo "FAIL $n :: $(head -c 160 "tmp/sw-$n.err" | tr '\n' ' ') $(tail -2 "tmp/sw-$n.out" | tr '\n' ' ')"
    bad=$((bad+1))
  fi
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
run country-sharing LivePresence.swift UnattendedDecision.swift
run diagnostic-report Brand.swift Clock.swift NodeWatchdog.swift NodeStopReason.swift HealthCheck.swift DiagnosticReport.swift
run design-effects DesignTokens.swift Design/DesignEffects.swift Design/DesignEventEffect.swift Design/BalanceCountUp.swift Design/RewardShine.swift Design/PresentationMotion.swift Design/NavyPlateDepth.swift Design/SuccessFeedback.swift Design/NodeStatusPulse.swift Design/DesignSurface.swift Design/MenuBarPanel.swift
run earnings EarningsModel.swift
run earnings-export Brand.swift EarningsModel.swift ChainActivity.swift EarningsExport.swift
run fee-confirm EarningsModel.swift
run health-check Brand.swift Clock.swift NodeWatchdog.swift NodeStopReason.swift HealthCheck.swift
run history-notice HistoryFailure.swift
run install-location Brand.swift InstallLocation.swift
run key-exposure KeyExposureNotice.swift
run legacy-aether Brand.swift LegacyAether.swift
run live-presence LivePresence.swift UnattendedDecision.swift
run live-globe LivePresence.swift LiveGlobePresence.swift
run live-globe-bundle LiveGlobeBundlePolicy.swift
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
# Native identity fixtures need signed task-owned executables and arguments.
if [ "$(uname -s)" = Darwin ]; then
  if bash "apps/wallet/Tests/country-sharing/test-wrapper.sh" > "tmp/sw-country-wrapper.out" 2> "tmp/sw-country-wrapper.err"; then
    echo "OK   country-wrapper"
  else
    echo "FAIL country-wrapper :: $(tail -2 "tmp/sw-country-wrapper.err") $(tail -2 "tmp/sw-country-wrapper.out")"
    bad=$((bad+1))
  fi
  for fixture in update-daemon update-listener; do
    if bash "scripts/test-$fixture.sh" > "tmp/sw-$fixture.out" 2> "tmp/sw-$fixture.err"; then
      echo "OK   $fixture"
    else
      echo "FAIL $fixture :: $(tail -2 "tmp/sw-$fixture.err") $(tail -2 "tmp/sw-$fixture.out")"
      bad=$((bad+1))
    fi
  done
fi
exit $bad
