// spikes/macos/spike-macos.swift
//
// Hypr-Rust native platform spike — macOS (Intel), public APIs only.
// Implements the protocol in docs/PLATFORM-SPIKES.md:
//   Spike 0 (record), Spike 1 (ordinary window management), Spike 2 (Spaces contract).
//
// Modes:
//   record                 Spike 0 environment record + window discovery (read-only)
//   spawn-target           Start a controlled target window; prints its PID and exits the printing, stays alive
//   run-tests --pid <n>    Spike 1 mutations (move/resize/raise) against the given PID only
//   space-observe          Listen for NSWorkspace.activeSpaceDidChangeNotification (30 s)
//   space-switch --dir right|left [--pid <n>]  Post public CGEvent Ctrl+Arrow, observe space change
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

func axSize(_ el: AXUIElement, _ name: String) -> CGSize? {
    guard let v = axAttr(el, name), CFGetTypeID(v) == AXValueGetTypeID() else { return nil }
    var s = CGSize.zero
    guard AXValueGetValue(v as! AXValue, .cgSize, &s) else { return nil }
    return s
}

func axSetPoint(_ el: AXUIElement, _ name: String, _ p: CGPoint) -> Bool {
    var pp = p
    guard let v = AXValueCreate(.cgPoint, &pp) else { return false }
    return AXUIElementSetAttributeValue(el, name as CFString, v) == .success
}

func axSetSize(_ el: AXUIElement, _ name: String, _ s: CGSize) -> Bool {
    var ss = s
    guard let v = AXValueCreate(.cgSize, &ss) else { return false }
    return AXUIElementSetAttributeValue(el, name as CFString, v) == .success
}

// MARK: - Spike 1 mutations (controlled target only)

func runTests(pid: pid_t) {
    print("== Spike 1 — mutations on controlled target pid=\(pid) ==")
    guard AXIsProcessTrusted() else {
        print("mutations: DENIED — Accessibility permission not granted to host process (AXIsProcessTrusted=false). This is the protocol's permission-denied case; move/resize/focus NOT VERIFIED on this machine until granted.")
        exit(3)
    }
    let wins = axWindows(pid)
    guard let win = wins.first else {
        print("target AX tree: NOT ACCESSIBLE or no window (windows found: \(wins.count))"); exit(4)
    }
    print("target AX tree: ACCESSIBLE (windows: \(wins.count))")

    guard let p0 = axPoint(win, kAXPositionAttribute), let s0 = axSize(win, kAXSizeAttribute) else {
        print("read frame: FAIL"); exit(5)
    }
    print("read frame: PASS position=(\(Int(p0.x)),\(Int(p0.y))) size=\(Int(s0.width))x\(Int(s0.height))")

    // Move x3 (repeatable, no feedback loop): deterministic offsets.
    var allMoved = true
    for (i, target) in [(120.0, 120.0), (180.0, 140.0), (150.0, 130.0)].enumerated() {
        let ok = axSetPoint(win, kAXPositionAttribute, CGPoint(x: target.0, y: target.1))
        Thread.sleep(forTimeInterval: 0.25)
        let p = axPoint(win, kAXPositionAttribute)
        let converged = ok && p != nil && abs(p!.x - target.0) < 2 && abs(p!.y - target.1) < 2
        print("move[\(i)] set=(\(Int(target.0)),\(Int(target.1))) -> read=(\(p.map { "(\(Int($0.x)),\(Int($0.y)))" } ?? "nil")) \(converged ? "PASS" : "FAIL")")
        allMoved = allMoved && converged
    }

    // Resize: set an explicit size, verify.
    let newSize = CGSize(width: 480, height: 320)
    let resized = axSetSize(win, kAXSizeAttribute, newSize)
    Thread.sleep(forTimeInterval: 0.25)
    let s = axSize(win, kAXSizeAttribute)
    let resizeOk = resized && s != nil && abs(s!.width - newSize.width) < 4 && abs(s!.height - newSize.height) < 4
    print("resize -> read=\(s.map { "\(Int($0.width))x\(Int($0.height))" } ?? "nil") \(resizeOk ? "PASS" : "FAIL")")

    // Focus request: AXRaise (public action). Focus theft check = we only requested OUR target.
    let raiseErr = AXUIElementPerformAction(win, kAXRaiseAction as CFString)
    print("focus request (kAXRaiseAction): \(raiseErr == .success ? "PASS" : "FAIL (\(raiseErr.rawValue))")")

    // No Dock/system-UI mutation: probes mutate only the self-owned target.
    // (Dock AX-move probe removed per M1 stage instruction — system apps must
    // never be mutated by a probe.)

    print("summary: move\(allMoved ? " PASS" : " FAIL") resize\(resizeOk ? " PASS" : " FAIL") — ordinary window management via AX \(allMoved && resizeOk ? "PASS" : "PARTIAL/FAIL")")
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

// MARK: - Controlled target window (our own app; never a user window)

/// Register this process as the self-owned mutation target: write
/// /tmp/hypr-rust-target.json (perm 0600) with {pid, token} and return the
/// token. The Rust probe requires this token for mutation commands, so a
/// probe can never mutate an arbitrary pid — only the registered fixture.
func registerTargetFile() -> String? {
    let token = UUID().uuidString.replacingOccurrences(of: "-", with: "")
    let path = "/tmp/hypr-rust-target.json"
    let body = "{\"pid\":\(getpid()),\"token\":\"\(token)\"}"
    let ok = FileManager.default.createFile(
        atPath: path,
        contents: body.data(using: .utf8),
        attributes: [.posixPermissions: 0o600])
    if !ok {
        FileManager.default.createFile(atPath: path, contents: nil, attributes: nil)
        try? body.data(using: .utf8)!.write(to: URL(fileURLWithPath: path))
        try? FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: path)
    }
    return token
}

func spawnTarget() {
    let app = NSApplication.shared
    app.setActivationPolicy(.regular)
    let win = NSWindow(
        contentRect: NSRect(x: 100, y: 100, width: 420, height: 280),
        styleMask: [.titled, .resizable],
        backing: .buffered, defer: false)
    win.title = "HYPR-SPIKE-TARGET"  // Controlled test window; title is a fixed marker, not user data.
    win.isReleasedWhenClosed = false
    win.makeKeyAndOrderFront(nil)
    app.activate(ignoringOtherApps: true)
    if let token = registerTargetFile() {
        print("target-token=\(token)")
    } else {
        FileHandle.standardError.write("warning: target token registration failed\n".data(using: .utf8)!)
    }
    print("HYPR-SPIKE-TARGET pid=\(getpid()) frame=\(win.frame)")
    print("keep this process running while 'run-tests --pid \(getpid())' executes…")
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
    spawnTarget()
case "run-tests":
    guard let p = argValue("--pid"), let pid = pid_t(p) else { print("usage: run-tests --pid <pid>"); exit(1) }
    runTests(pid: pid)
case "space-observe":
    spaceObserve(seconds: Double(argValue("--seconds") ?? "30") ?? 30)
case "space-switch":
    spaceSwitch(dir: argValue("--dir") ?? "right", pid: (argValue("--pid")).flatMap(pid_t.init),
                execute: args.contains("--execute"))
default:
    print("modes: record | spawn-target | run-tests --pid <pid> | space-observe [--seconds n] | space-switch --dir right|left [--pid pid] [--execute]")
}
