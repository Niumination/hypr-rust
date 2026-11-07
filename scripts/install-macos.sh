#!/usr/bin/env bash
# Install or update the LOCAL ax-shell.app on this Mac (owner-run).
#
# Honest scope (docs/USING-AX-SHELL.md, docs/ROADMAP.md): a local install/
# update path for the ad-hoc development bundle — not distribution. The
# bundle stays unsigned/ad-hoc; after every install the ad-hoc cdhash has
# changed, so TCC (Privacy & Security → Accessibility) must remove (`-`) and
# re-add (`+`) the entry — a toggle alone is not enough (owner-confirmed
# 2026-10-26, ARENA Path N). This script cannot and does not touch TCC.
#
# Usage (from the repo root, or it cd's itself there):
#   scripts/install-macos.sh             # build + install + LaunchAgent
#   scripts/install-macos.sh --dry-run   # print the actions, change nothing
#   scripts/install-macos.sh --uninstall # stop + remove agent, plist, bundle
#
# Install location: ~/Applications/ax-shell.app (owner decision 2026-11-07;
# no sudo). Autostart: LaunchAgent at login (owner decision 2026-11-07).
# Config (~/.hypr-rust/config.json) and logs are NEVER touched here.
# Env overrides: AX_SHELL_APP_NAME, AX_SHELL_BUNDLE_ID, AX_SHELL_BIN_NAME
set -euo pipefail
cd "$(dirname "$0")/.."

APP_NAME="${AX_SHELL_APP_NAME:-ax-shell}"
BUNDLE_ID="${AX_SHELL_BUNDLE_ID:-io.github.niumination.ax-shell}"
BIN_NAME="${AX_SHELL_BIN_NAME:-ax-shell}"

INSTALL_DIR="$HOME/Applications"
BUNDLE="$INSTALL_DIR/$APP_NAME.app"
DIST_BUNDLE="crates/platform-macos/target/dist/$APP_NAME.app"
AGENT_PLIST="$HOME/Library/LaunchAgents/$BUNDLE_ID.plist"

DRY_RUN=0
UNINSTALL=0
for arg in "$@"; do
    case "$arg" in
        --dry-run) DRY_RUN=1 ;;
        --uninstall) UNINSTALL=1 ;;
        *)
            echo "error: unknown argument: $arg (try --dry-run or --uninstall)" >&2
            exit 2
            ;;
    esac
done

if [ "$(uname)" != "Darwin" ]; then
    echo "error: this script installs a macOS bundle; run it on macOS" >&2
    exit 2
fi

UID_N="$(id -u)"
BOOTSTRAP_DOMAIN="gui/$UID_N"

# Run a mutating step (or print it under --dry-run).
run() {
    if [ "$DRY_RUN" -eq 1 ]; then
        echo "dry-run: $*"
    else
        "$@"
    fi
}

# True when the LaunchAgent label is loaded in this user's domain.
agent_loaded() {
    launchctl print "$BOOTSTRAP_DOMAIN/$BUNDLE_ID" >/dev/null 2>&1
}

# True when an ax-shell process is running (any launch path).
shell_running() {
    pgrep -x "$BIN_NAME" >/dev/null 2>&1
}

stop_shell() {
    if shell_running; then
        # No SIGTERM handler in the shell (2026-11-07): the process exits at
        # once; windows keep their last OS-accepted frames (quit semantics)
        # and a stale IPC socket is removed by the next start.
        run pkill -TERM -x "$BIN_NAME" || true
        # Wait for the exit; a straggler is reported, never force-killed.
        if [ "$DRY_RUN" -eq 0 ]; then
            for _ in 1 2 3 4 5 6 7 8 9 10; do
                shell_running || break
                sleep 0.5
            done
            if shell_running; then
                echo "warning: ax-shell is still running; the installed bundle" >&2
                echo "will not take effect until it exits (not force-killed)" >&2
            fi
        fi
    fi
}

uninstall() {
    if agent_loaded; then
        run launchctl bootout "$BOOTSTRAP_DOMAIN/$BUNDLE_ID"
    else
        echo "agent not loaded: $BUNDLE_ID"
    fi
    stop_shell
    if [ -e "$AGENT_PLIST" ]; then
        run rm -f "$AGENT_PLIST"
    else
        echo "no agent plist: $AGENT_PLIST"
    fi
    if [ -e "$BUNDLE" ]; then
        run rm -rf "$BUNDLE"
    else
        echo "no installed bundle: $BUNDLE"
    fi
    echo "uninstalled. Config and logs under ~/.hypr-rust/ were not touched."
}

install() {
    # 1. Fresh bundle first: never install a stale artifact.
    if [ "$DRY_RUN" -eq 1 ]; then
        echo "dry-run: scripts/package-macos.sh"
    else
        scripts/package-macos.sh
    fi
    if [ "$DRY_RUN" -eq 0 ] && [ ! -d "$DIST_BUNDLE" ]; then
        echo "error: packaging produced no bundle at $DIST_BUNDLE" >&2
        exit 1
    fi

    # 2. Stop the running instance before replacing its bundle.
    stop_shell

    # 3. Install (replace) the bundle.
    run mkdir -p "$INSTALL_DIR"
    run rm -rf "$BUNDLE"
    if [ "$DRY_RUN" -eq 1 ]; then
        echo "dry-run: ditto $DIST_BUNDLE $BUNDLE"
    else
        ditto "$DIST_BUNDLE" "$BUNDLE"
    fi

    # 4. LaunchAgent: start the installed binary at login.
    run mkdir -p "$(dirname "$AGENT_PLIST")"
    if [ "$DRY_RUN" -eq 1 ]; then
        echo "dry-run: write $AGENT_PLIST (RunAtLoad, $BUNDLE/Contents/MacOS/$BIN_NAME)"
    else
        cat > "$AGENT_PLIST" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>$BUNDLE_ID</string>
	<key>ProgramArguments</key>
	<array>
		<string>$BUNDLE/Contents/MacOS/$BIN_NAME</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
</dict>
</plist>
PLIST
        plutil -lint "$AGENT_PLIST"
    fi

    # 5. Reload the agent (update = bootout then bootstrap).
    if agent_loaded; then
        run launchctl bootout "$BOOTSTRAP_DOMAIN/$BUNDLE_ID"
    fi
    run launchctl bootstrap "$BOOTSTRAP_DOMAIN" "$AGENT_PLIST"

    # 6. Verify what can be verified from here.
    if [ "$DRY_RUN" -eq 1 ]; then
        echo "dry-run: verify bundle + launchctl print $BOOTSTRAP_DOMAIN/$BUNDLE_ID"
        return
    fi
    if [ ! -x "$BUNDLE/Contents/MacOS/$BIN_NAME" ]; then
        echo "error: installed binary missing/not executable: $BUNDLE/Contents/MacOS/$BIN_NAME" >&2
        exit 1
    fi
    if ! agent_loaded; then
        echo "error: LaunchAgent not loaded after bootstrap: $BUNDLE_ID" >&2
        exit 1
    fi
    echo "installed: $BUNDLE"
    echo "agent loaded: $BUNDLE_ID (RunAtLoad at login)"

    cat <<EOF

NEXT STEPS (manual, this script cannot do them):
1. TCC re-grant — the ad-hoc cdhash changed with this install:
   System Settings → Privacy & Security → Accessibility → remove (-) the old
   ax-shell entry, then add (+) $BUNDLE. A toggle alone is not enough.
2. Verify: open $BUNDLE --args --self-test
   then check ~/.hypr-rust/shell.log for the "self-test: exit" line.
3. Update later = re-run this script (same steps, fresh build).
EOF
}

if [ "$UNINSTALL" -eq 1 ]; then
    uninstall
else
    install
fi
