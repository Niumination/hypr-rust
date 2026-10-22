# macOS Host Information Collection (Read-Only)

**Purpose:** give a local OpenCode/Hermes agent the minimum reproducible context for the macOS Tahoe implementation spike. Run from the repository root on the user's current macOS installation.

## Collection procedure

1. Review the script `scripts/collect-macos-env.sh` before running it.
2. Run `sh scripts/collect-macos-env.sh` from the repo root.
3. Copy its output into the current spike report/PR, after reviewing it for anything unexpected.
4. Add manually: whether the test display is internal/external, whether a second display was attached, and which app/process identity has Accessibility permission.
5. Include branch/full commit SHA and test commands/results in the same report.

The script is read-only: it queries OS/tool versions, model fields, active display geometry, Accessibility trust for the short-lived Swift process, toolchain availability and Git state. It does **not** list app windows, read titles, access serials/UUIDs, alter settings, request permissions, or access credentials. Running the script does not grant Accessibility permission; trust is process-identity-specific.

## Safe command output

Expected fields:

- `sw_vers`: macOS product version and build.
- `uname -m` and CPU brand string.
- System-reported model name/identifier, processor and memory (filtered; no serial/UUID).
- Active displays' frame, visible frame and backing scale (no display serials).
- Swift, Xcode Command Line Tools path, Rust/Cargo versions if installed.
- Repo branch, full commit and working-tree status.
- `AXIsProcessTrusted` for the script's Swift process only.

## Do not provide

Do **not** paste or commit serial numbers, hardware UUIDs, Apple IDs, passwords, tokens, full EFI/OpenCore configs, `nvram -p`/raw `ioreg` output, screenshots, or unredacted window titles. If a tool accidentally prints such data, delete it from the report and rotate exposed credentials.

## Additional required context for a native test

For each test case, write down:

```text
Commit SHA:
Test app (prefer self-owned HYPR-SPIKE-TARGET):
Permission identity granted in System Settings:
Operation: enumerate / read frame / move / resize / focus / stale handle / denied permission
Expected result:
Observed result:
Native status/error:
Repeated runs and count:
Known limitation:
```

Use the self-owned test target only. The previous upstream probe contains a Dock mutation attempt; do not run that code path until removed and reviewed. Ordinary user/system windows must remain read-only during the probe.
