#!/bin/sh
# Read-only environment report for macOS development/spike runs.
# Does not enumerate windows, inspect titles, request permissions, or read serials/UUIDs.

set -u

section() {
  printf '\n== %s ==\n' "$1"
}

section "OS"
if command -v sw_vers >/dev/null 2>&1; then
  sw_vers
else
  echo "sw_vers: unavailable"
fi
printf 'architecture: '
uname -m 2>/dev/null || echo "unavailable"
printf 'kernel: '
uname -r 2>/dev/null || echo "unavailable"
printf 'cpu_brand: '
if command -v sysctl >/dev/null 2>&1; then
  sysctl -n machdep.cpu.brand_string 2>/dev/null || echo "unavailable"
else
  echo "unavailable"
fi

section "System-reported model (serial/UUID intentionally excluded)"
if command -v system_profiler >/dev/null 2>&1; then
  system_profiler SPHardwareDataType 2>/dev/null | awk -F ': ' '
    /Model Name|Model Identifier|Processor Name|Processor Speed|Total Number of Cores|Memory/ {
      print $1 ": " $2
    }
  '
else
  echo "system_profiler: unavailable"
fi

section "Displays (geometry only)"
if command -v swift >/dev/null 2>&1; then
  swift -e 'import AppKit; for (i, s) in NSScreen.screens.enumerated() { let f = s.frame; let v = s.visibleFrame; print("display[\(i)] frame=\(f) visible=\(v) scale=\(s.backingScaleFactor)") }' 2>&1 || echo "display query failed"
else
  echo "Swift unavailable; record display count, frame and scale manually"
fi

section "Accessibility trust (for this short-lived Swift process only)"
if command -v swift >/dev/null 2>&1; then
  swift -e 'import ApplicationServices; print("AXIsProcessTrusted=\(AXIsProcessTrusted())")' 2>&1 || echo "AX trust query failed"
else
  echo "Swift unavailable; check System Settings > Privacy & Security > Accessibility manually"
fi

section "Toolchain"
printf 'xcode-select: '
if command -v xcode-select >/dev/null 2>&1; then
  xcode-select -p 2>&1 || echo "not configured"
else
  echo "unavailable"
fi
printf 'swift: '
if command -v swift >/dev/null 2>&1; then swift --version 2>&1 | head -n 2; else echo "unavailable"; fi
printf 'rustc: '
if command -v rustc >/dev/null 2>&1; then rustc --version 2>&1; else echo "unavailable"; fi
printf 'cargo: '
if command -v cargo >/dev/null 2>&1; then cargo --version 2>&1; else echo "unavailable"; fi

section "Repository state"
if git rev-parse --show-toplevel >/dev/null 2>&1; then
  printf 'root: '
  git rev-parse --show-toplevel
  printf 'branch: '
  git branch --show-current
  printf 'commit: '
  git rev-parse HEAD
  printf 'working_tree:\n'
  git status --short
else
  echo "not run from a Git repository"
fi

section "End"
echo "Review output before sharing. This script does not grant permissions or validate app behavior."
