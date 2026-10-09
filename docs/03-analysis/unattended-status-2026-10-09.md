# Unattended status on the 0.7.3.1 / 0.7.3.2 builds

The launch warning was a registration bug, not evidence that notarization
disabled unattended restart. The `.notFound` lookup result can be expected
before macOS has ever seen a service. The app interpreted it as a broken build
and also excluded it from the registration path.

## What the check actually tests

`UnattendedDaemon.refreshStatus()` read
`SMAppService.daemon(plistName: "com.pipln.eastsea.node.plist").status`.
This is the system's registration/authorization lookup. It does not verify
the app's signature, its notarization ticket, or whether its helper files
exist. Apple documents `.notFound` as a failed lookup; [Apple DTS explains
that an unseen service may also have that status](https://developer.apple.com/forums/thread/719862).
The same discussion distinguishes it from `.notRegistered` after a service
has been registered and then unregistered.

Two callers made that result persist: startup called only `refreshStatus()`
for a saved opt-in, and `applyEnabledChange()` called `register()` only for
`.notRegistered`. Neither path registered an unseen `.notFound` service.

## Release evidence (read-only inspection)

Both local release build inputs contain
`Contents/Library/LaunchDaemons/com.pipln.eastsea.node.plist`, with label
`com.pipln.eastsea.node` and `BundleProgram` set to
`Contents/Resources/eastsea-node-daemon.sh`. That script exists and is
executable. The bundle location matches [Apple's daemon API requirements](https://developer.apple.com/documentation/servicemanagement/smappservice/daemon%28plistname%3A%29).

| Release | Build | Signature | DMG notarization ticket | Published DMG SHA-256 |
| --- | --- | --- | --- | --- |
| 0.7.3.1 | 17 | Pipln Developer ID, team `45WU468FZE`; `codesign --verify --deep --strict` passed | `xcrun stapler validate` passed | `581633fa4e4d71d89712d391ae3ae71fbbdc89d4f73fdc80858ea75e0345df72` |
| 0.7.3.2 | 18 | Pipln Developer ID, team `45WU468FZE`; `codesign --verify --deep --strict` passed | `xcrun stapler validate` passed | `a6f0f96b99780adb7a1e57bcf1d7b008c74745a5b9dc728fd3938498cbbe9f65` |

The local DMG hashes match the asset digests returned by the GitHub release
API for `app-v0.7.3.1` and `app-v0.7.3.2`. The inspected build inputs are in
the `hotfix-0731` and `hotfix-0732` worktrees. The DMGs were not mounted and
the apps were not launched. Successful signing/notarization establishes the
packaging prerequisites; it does not establish local registration or consent.

## Behavior after the fix

Startup restores a saved opt-in. A complete bundled service in `.notFound`
or `.notRegistered` is registered. A pending or withheld approval becomes
`.needsApproval`: Settings and the approval row give the existing Login Items
instruction, rather than the orange failed-build warning. The normal log
transition is `off -> needsApproval` or `off -> approved`.

A missing/malformed helper bundle or an actual registration exception still
becomes `.failed`. The exception is recorded as `unattended_registration_failed`;
its message remains visible on later refreshes. A failed/off state removes
the respawn marker. When approval is observed later, refresh recreates the
marker subject to the existing node, location, migration, suspension and
external-disk guards. External block data retains its app-open fallback text.

Pure Swift coverage includes the unseen/registered/denied/missing/error/off
states, all five languages for the missing-build explanation, marker
eligibility, and the startup/refresh adapter bindings. The new startup binding
regression failed before the adapter fix and passed afterward. Actual system
registration and reboot remain lead integration checks: this lane neither
builds nor launches EastSea.
