// spikes/macos/spike-macos.swift
//
// Hypr-Rust native platform spike — macOS (Intel), public APIs only.
// Implements the protocol in docs/PLATFORM-SPIKES.md:
//   Spike 0 (record), Spike 1 (ordinary window management), Spike 2 (Spaces contract).
//
// Modes:
//   record                 Spike 0 environment record + window discovery (read-only)
//   spawn-target           Start a controlled target window; registers it as the
//                          self-owned mutation fixture (token in owner-only file
//                          + liveness lock), prints its PID, stays alive.
//                          `--no-register` starts the same self-owned window
//                          WITHOUT the lock/registration: an auxiliary batch
//                          target only — it can never act as the batch master.
//   space-observe          Listen for NSWorkspace.activeSpaceDidChangeNotification (30 s)
//   space-switch --dir right|left [--pid <n>]  Post public CGEvent Ctrl+Arrow, observe space change
//
// Mutation safety (M1 closeout): the Swift harness is a fixture spawner +
// read-only probe ONLY. The old mutating `run-tests --pid` path was removed so
// no Swift path mutates an arbitrary PID; the Rust `ax-probe` is the sole
// mutator, gated by the fixture registration below. A normal command cannot
// mutate Dock, Finder, system UI, or an unrelated user process.
//
// Rules honored (see AGENTS.md):
// - No private APIs (no CGS/SkyLight). CGWindowList, AXUIElement, NSWorkspace, CGEvent are public.
// - No hide/minimize workspace fallback; no telemetry; no network.
// - Only mutates the controlled spike target window (never user windows).
// - Never prints user window titles; discovery output is owner name + PID + bounds only.

import AppKit
import ApplicationServices
import CoreGraphics
import Foundation

// MARK: - Spike 0 helpers

func osRecord() {
    print("== Spike 0 — environment record ==")
    print("date: \(ISO8601DateFormatter().string(from: Date()))")
    print("os_version: \(ProcessInfo.processInfo.operatingSystemVersionString)")
    if let sv = NSDictionary(contentsOfFile: "/System/Library/CoreServices/SystemVersion.plist"),
       let build = (sv["BuildVersion"] ?? sv["ProductBuildVersion"]) as? String {
        print("build: \(build)")
    }
    print("arch: \(unameMachine())")
    print("host_cpu: \(hostName())")
    for (i, screen) in NSScreen.screens.enumerated() {
        let f = screen.frame
        let scale = screen.backingScaleFactor
        print("display[\(i)]: frame=\(Int(f.width))x\(Int(f.height)) origin=(\(Int(f.minX)),\(Int(f.minY))) scale=\(scale)x \(screen == NSScreen.main ? "[main]" : "")")
    }
    let trusted = AXIsProcessTrusted()
    print("accessibility_permission: \(trusted ? "GRANTED" : "DENIED") (AXIsProcessTrusted)")
    print("signing: ad-hoc CLI built with swiftc, run from terminal host")
}

func unameMachine() -> String {
    var sysinfo = utsname()
    uname(&sysinfo)
    return withUnsafeBytes(of: &sysinfo.machine) { raw in
        String(cString: raw.baseAddress!.assumingMemoryBound(to: CChar.self))
    }
}

func hostName() -> String {
    var size = 0
    sysctlbyname("machdep.cpu.brand_string", nil, &size, nil, 0)
    var buf = [CChar](repeating: 0, count: size)
    sysctlbyname("machdep.cpu.brand_string", &buf, &size, nil, 0)
    return String(cString: buf)
}

// MARK: - Window discovery (Spike 1 discovery case)

func discover() {
    print("== Spike 1 — discovery (CGWindowListCopyWindowInfo, layer 0, on-screen) ==")
    guard let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] else {
        print("discovery: FAILED"); exit(2)
    }
    var count = 0
    for info in list where (info[kCGWindowLayer as String] as? Int) == 0 {
        let owner = info[kCGWindowOwnerName as String] as? String ?? "?"
        let pid = info[kCGWindowOwnerPID as String] as? Int ?? -1
        guard let b = info[kCGWindowBounds as String] as? [String: CGFloat] else { continue }
        // Titles are intentionally NOT read (no kCGWindowName): user data redaction rule.
        print("window: owner=\(owner) pid=\(pid) bounds=(x:\(Int(b["x"] ?? 0)), y:\(Int(b["y"] ?? 0)), w:\(Int(b["Width"] ?? 0)), h:\(Int(b["Height"] ?? 0)))")
        count += 1
    }
    print("discovery: \(count > 0 ? "PASS" : "EMPTY") (\(count) layer-0 on-screen windows)")
}

// MARK: - AX helpers

func axApp(_ pid: pid_t) -> AXUIElement { AXUIElementCreateApplication(pid) }

func axAttr(_ el: AXUIElement, _ name: String) -> CFTypeRef? {
    var v: CFTypeRef?
    let err = AXUIElementCopyAttributeValue(el, name as CFString, &v)
    return err == .success ? v : nil
}

func axWindows(_ pid: pid_t) -> [AXUIElement] {
    guard let raw = axAttr(axApp(pid), kAXWindowsAttribute), CFGetTypeID(raw) == CFArrayGetTypeID() else { return [] }
    let arr = (raw as! CFArray) as! [AXUIElement]
    return arr
}

func axPoint(_ el: AXUIElement, _ name: String) -> CGPoint? {
    guard let v = axAttr(el, name), CFGetTypeID(v) == AXValueGetTypeID() else { return nil }
    var p = CGPoint.zero
    guard AXValueGetValue(v as! AXValue, .cgPoint, &p) else { return nil }
    return p
}

// MARK: - Spike 2 — Spaces contract (public surface only)

final class SpaceObserver {
    var fired = 0
    init() {
        NSWorkspace.shared.notificationCenter.addObserver(
            self, selector: #selector(changed), name: NSWorkspace.activeSpaceDidChangeNotification, object: nil)
    }
    @objc func changed() { fired += 1 }
    deinit { NotificationCenter.default.removeObserver(self) }
}

func spaceObserve(seconds: Double) {
    print("== Spike 2 — observe: NSWorkspace.activeSpaceDidChangeNotification (public) ==")
    print("listening \(Int(seconds))s — switch Space manually or trigger space-switch mode in another terminal…")
    let obs = SpaceObserver()
    let deadline = Date().addingTimeInterval(seconds)
    while Date() < deadline {
        RunLoop.current.run(until: Date().addingTimeInterval(0.2))
        if obs.fired > 0 {
            print("observe: PASS — activeSpaceDidChangeNotification fired \(obs.fired)x (user/externally initiated space change observed, no polling of private APIs)")
            return
        }
    }
    print("observe: NO EVENT in \(Int(seconds))s (not proof of unsupported — see protocol §Evidence; rerun with a real space switch)")
}

func spaceSwitch(dir: String, pid: pid_t?, execute: Bool) {
    print("== Spike 2 — detect/switch/move-to-space via public APIs ==")
    print("detect space identity: NOT AVAILABLE via public API (no CGS/SkyLight). Public proxy = on-screen window-set snapshot:")
    discover()
    if let pid = pid, AXIsProcessTrusted() {
        let wins = axWindows(pid)
        if let win = wins.first, let _ = axPoint(win, kAXPositionAttribute) {
            print("move focused/target window to another Space: NOT AVAILABLE via reviewed public API (no public 'move window to space'; drag gestures and CGS are not usable here). NOT VERIFIED = unsupported under project rules.")
        }
    }
    guard execute else {
        print("switch space: requires --execute to post CGEvent Ctrl+\(dir == "right" ? "→" : "←") (public API; needs Accessibility trust to have effect)")
        return
    }
    let keycode: CGKeyCode = dir == "right" ? 124 : 123 // Right arrow / Left arrow
    for down in [true, false] {
        guard let ev = CGEvent(keyboardEventSource: nil, virtualKey: keycode, keyDown: down) else { continue }
        ev.flags = .maskControl
        ev.post(tap: .cghidEventTap)
        usleep(80_000)
    }
    print("switch posted (Ctrl+\(dir == "right" ? "→" : "←")). Verify with space-observe mode; if Space changed, this is key-chord synthesis — a DEGRADED mechanism, not a direct workspace API (must be labeled as such).")
    if let pid = pid, AXIsProcessTrusted() {
        // Confirm the controlled target is still in its last OS-accepted position (no feedback loop).
        if let win = axWindows(pid).first, let p = axPoint(win, kAXPositionAttribute) {
            print("target window after switch: still addressable at (\(Int(p.x)),\(Int(p.y)))")
        }
    }
}

// MARK: - Fixture registration (lifecycle-bounded, owner-only, fail closed)
//
// The registration is a local test guard, not a multi-user security boundary.
// It exists so the Rust probe cannot mutate an arbitrary pid — only the
// registered self-owned fixture. Process-instance identity is provided by an
// exclusive flock held by the fixture for its whole lifetime: the probe refuses
// mutation unless the lock is held by a live process, so a stale record or a
// reused PID cannot authorize mutation. (macOS exposes no public process
// start-time API — proc_pidinfo/libproc is private and the sysctl extern_proc
// layout is not in public headers — so the flock is the public, robust
// process-instance identity here.)

/// Owner-only directory holding the fixture lock + registration.
func fixtureDir() -> String {
    return NSHomeDirectory() + "/.hypr-rust"
}

/// Acquire the fixture liveness lock (held for the fixture's lifetime).
/// Returns the open file descriptor, or nil on failure (fail closed).
func acquireFixtureLock() -> Int32? {
    let dir = fixtureDir()
    do {
        try FileManager.default.createDirectory(
            atPath: dir, withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o700])
    } catch {
        return nil
    }
    let lockPath = (dir as NSString).appendingPathComponent("fixture.lock")
    let fd = open(lockPath, O_RDWR | O_CREAT, 0o600)
    if fd < 0 { return nil }
    if flock(fd, LOCK_EX) != 0 {
        close(fd)
        return nil
    }
    return fd
}

/// Atomically write the registration (temp + rename) with owner-only perms.
/// Returns true on success (fail closed otherwise).
func writeRegistration(token: String) -> Bool {
    let dir = fixtureDir()
    let regPath = (dir as NSString).appendingPathComponent("fixture-target.json")
    let tmpPath = (dir as NSString).appendingPathComponent("fixture-target.json.tmp")
    let body = "{\"pid\":\(getpid()),\"token\":\"\(token)\"}"
    guard let data = body.data(using: .utf8) else { return false }
    let fd = open(tmpPath, O_WRONLY | O_CREAT | O_EXCL, 0o600)
    if fd < 0 { return false }
    defer { close(fd) }
    guard data.withUnsafeBytes({ write(fd, $0.baseAddress, data.count) }) == data.count else {
        unlink(tmpPath)
        return false
    }
    fsync(fd)
    if rename(tmpPath, regPath) != 0 {
        unlink(tmpPath)
        return false
    }
    return true
}

/// Remove the registration on normal termination. (Abrupt death releases the
/// flock, so the probe rejects the stale record via the lock check.)
func removeRegistration() {
    let regPath = (fixtureDir() as NSString).appendingPathComponent("fixture-target.json")
    let tmpPath = (fixtureDir() as NSString).appendingPathComponent("fixture-target.json.tmp")
    try? FileManager.default.removeItem(atPath: regPath)
    try? FileManager.default.removeItem(atPath: tmpPath)
}

// MARK: - Controlled target window (our own app; never a user window)

func spawnTarget(register: Bool) {
    let app = NSApplication.shared
    app.setActivationPolicy(.regular)

    if register {
        // Fail closed: no lock and no registration -> do not expose a fixture.
        guard let lockFD = acquireFixtureLock() else {
            FileHandle.standardError.write("error: fixture lock acquisition failed\n".data(using: .utf8)!)
            exit(5)
        }
        let token = UUID().uuidString.replacingOccurrences(of: "-", with: "")
        guard writeRegistration(token: token) else {
            FileHandle.standardError.write("error: fixture registration failed\n".data(using: .utf8)!)
            close(lockFD)
            exit(5)
        }

        // Remove the registration on normal termination; the flock backstop covers
        // abrupt termination.
        NotificationCenter.default.addObserver(
            forName: NSApplication.willTerminateNotification, object: nil, queue: .main) { _ in
                removeRegistration()
                close(lockFD)
        }
    }

    let win = NSWindow(
        contentRect: NSRect(x: 100, y: 100, width: 420, height: 280),
        styleMask: [.titled, .resizable],
        backing: .buffered, defer: false)
    win.title = "HYPR-SPIKE-TARGET"  // Controlled test window; title is a fixed marker, not user data.
    win.isReleasedWhenClosed = false
    win.makeKeyAndOrderFront(nil)
    app.activate(ignoringOtherApps: true)

    if register {
        print("fixture registered at ~/.hypr-rust/fixture-target.json (owner-only, token NOT printed — read from file)")
        print("HYPR-SPIKE-TARGET pid=\(getpid()) frame=\(win.frame)")
        print("fixture registered at ~/.hypr-rust/fixture-target.json (lock held for process lifetime)")
    } else {
        // Auxiliary batch target: self-owned window without fixture authority.
        // It is a valid geometry target, never a master and never a registration owner.
        print("HYPR-SPIKE-TARGET pid=\(getpid()) frame=\(win.frame)")
        print("unregistered auxiliary window: no fixture lock, no registration")
    }
    fflush(stdout)  // keep the PID readable when stdout is redirected
    print("keep this process running while the Rust probe executes…")
    app.run()
    exit(0)
}

// MARK: - Entry

let args = CommandLine.arguments
let mode = args.count > 1 ? args[1] : ""

func argValue(_ name: String) -> String? {
    guard let i = args.firstIndex(of: name), i + 1 < args.count else { return nil }
    return args[i + 1]
}

switch mode {
case "record":
    osRecord()
    discover()
case "spawn-target":
    spawnTarget(register: !args.contains("--no-register"))
case "space-observe":
    spaceObserve(seconds: Double(argValue("--seconds") ?? "30") ?? 30)
case "space-switch":
    spaceSwitch(dir: argValue("--dir") ?? "right", pid: (argValue("--pid")).flatMap(pid_t.init),
                execute: args.contains("--execute"))
default:
    print("modes: record | spawn-target [--no-register] | space-observe [--seconds n] | space-switch --dir right|left [--pid pid] [--execute]")
}
