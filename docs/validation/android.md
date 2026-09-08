# Post-merge Android device/APK smoke validation

Status: **BLOCKED — no accessible device or runnable emulator.** This is not a
runtime pass. Checked revision `3e6beaa17ace3a8d90b47ed7416f00bf0e6a29ef` on
2026-09-07 at 10:30 UTC, in the isolated `validation-android` worktree.

## Read-only environment evidence

| Check | Observed result |
| --- | --- |
| `command -v adb`, `emulator`, `java`, `qemu-system-aarch64`, `qemu-system-x86_64` | None found on PATH |
| `ANDROID_HOME`, `ANDROID_SDK_ROOT`, `JAVA_HOME` | Unset |
| `ADB_SERVER_SOCKET`, `ANDROID_SERIAL`, `ANDROID_ADB_SERVER_PORT` | Unset; no configured remote target |
| Existing ADB server at `127.0.0.1:5037` | TCP connection refused (`errno 111`) |
| `/dev/bus/usb` | Absent; no accessible USB device interface |
| `/sys/bus/usb/devices` | Only four USB root hubs and their root interfaces; no attached phone listed |
| `/dev/kvm` | Absent |
| Host architecture | `x86_64` |
| `/home/phire/.android`, `/home/phire/Android/Sdk` | Absent; no standard local AVD/SDK installation |
| `/opt/android-sdk`, `/opt/android`, `/usr/lib/android-sdk`, `/usr/lib/jvm` | Absent |
| Executable search under `/opt` and `/usr/local` | No matching adb/emulator/java; `/opt/containerd` inaccessible |
| Worktree `android/assets/Data` and `target` | Absent; no staged game payload or built APK |

The ADB check used a two-second socket connection to the existing local server,
with the read-only `host:devices-l` request prepared. Connection failed before
any request was sent. It did **not** start an ADB server, generate keys, pair,
authorize, install, launch, stop, reset, or modify a device. Process enumeration
is sandbox-local and cannot prove absence of processes outside the sandbox;
the endpoint/device-interface checks above are the relevant access evidence.

The tracked Gradle wrapper exists, but it cannot supply the missing Java/SDK or
an execution target. The package declares only `arm64-v8a`, minimum API 26,
and requires OpenGL ES 3.0. A generic x86 emulator would not directly validate
this ARM64 package. Provisioning a new software-emulated ARM64 environment,
SDK/JDK/NDK, and full game payload is a substantial separate operation; it was
not attempted without discussion.

## Scope and next action

No new native build, APK assembly, installation, game launch, or device smoke
test ran. Earlier pre-merge native-link/JNI checks do not establish post-merge APK boot.
No production systems, application data, device settings, or user tooling were
changed. Only this validation report was added.

TODO(validation): obtain an authorized physical ARM64 Android API 26+ device
or an explicitly supplied ADB endpoint. First record the exact device serial,
model, API and ABI using read-only queries; obtain target approval before
installation/launch. Then build the merged revision with the complete native
shipping payload, install without clearing app data, and capture startup,
rendering/input/audio behavior and scoped application logs. Existing install
or signature conflicts must be reported, not resolved by uninstalling/wiping.
Any dedicated emulator result must be labeled emulator-only, not physical
device validation.

Evidence snapshot retained outside the worktree at
`/tmp/robin-android-validation.KpibP8/android-environment.txt` so worktree
cleanup does not remove the observations. The worktree remains held pending
coordinator cleanup or newly provided device access.
