# Laporan Verifikasi — macOS Spike #1 (Hypr-Rust)

**Tanggal:** 17 Oktober 2026
**Proyek:** `labs/hypr-rust` (repo `Niumination/hypr-rust`, public)
**Cabang:** `spike/macos-spike-1` @ `a2d899b` + rerun `fb120d9` · base `d2fc8ed`
**PR:** https://github.com/Niumination/hypr-rust/pull/1
**Metode:** harness Swift CLI `spikes/macos/spike-macos.swift` (ad-hoc, `swiftc -O`, Swift 6.3.3) — API publik saja: CGWindowList / AXUIElement / NSWorkspace / CGEvent. Tanpa CGS/SkyLight, tanpa API private. Target uji: window sendiri `HYPR-SPIKE-TARGET`; window user hanya dibaca discovery, judul tidak dicetak.

---

## 1. Tujuan

Fase 0 roadmap: buktikan kontrol window inti (Spike 1) dan kontrak workspace native (Spike 2) pada build macOS Intel nyata, sebelum core tiling dilanjutkan.

## 2. Environment (Spike 0 record)

```
date: 2026-10-17T15:41:18Z
os_version: Version 26.7.1 (Build 25G241)
arch: x86_64
host_cpu: Intel(R) Core(TM) i5-10310U CPU @ 1.70GHz
display[0]: frame=1920x1080 origin=(0,0) scale=1.0x [main]
accessibility_permission: GRANTED (AXIsProcessTrusted)
signing: ad-hoc CLI built with swiftc, run from terminal host
```

Discovery (layer-0 on-screen, read-only): 4 window (Firefox, Nicegram Desktop, kitty, Finder) — owner name + bounds saja.

## 3. Toolchain verification (Rust)

Dijalankan di cabang spike sebelum eksekusi native:

```
$ cargo fmt --check        → OK (no diff)
$ cargo clippy --workspace --all-targets -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 3.97s
$ cargo test --workspace
running 6 tests
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

## 4. Spike 1 — ordinary window management: **PASS**

Perintah: `./spike-macos spawn-target` (background) → `./spike-macos run-tests --pid <pid>`:

```
target AX tree: ACCESSIBLE (windows: 1)
read frame: PASS position=(100,668) size=420x312
move[0] set=(120,120) -> read=((120,120)) PASS
move[1] set=(180,140) -> read=((180,140)) PASS
move[2] set=(150,130) -> read=((150,130)) PASS
resize -> read=480x320 PASS
focus request (kAXRaiseAction): PASS
summary: move PASS resize PASS — ordinary window management via AX PASS
```

Stale handle: target di-kill, lalu probe ulang:

```
target AX tree: NOT ACCESSIBLE or no window (windows found: 0)
exit=0   (bersih, tidak hang)
```

## 5. Spike 2 — native workspace contract: **DISABLED**

```
$ ./spike-macos space-observe --seconds 40
listening 40s …
observe: NO EVENT in 40s (not proof of unsupported — see protocol §Evidence; rerun with a real space switch)

$ ./spike-macos space-switch --dir right --execute --pid <pid>
switch posted (Ctrl+→). … detect space identity: NOT AVAILABLE via public API (no CGS/SkyLight).
move focused/target window to another Space: NOT AVAILABLE via reviewed public API.
```

| Operation | Status |
|---|---|
| Detect active/target Space | NOT AVAILABLE (tidak ada API publik untuk identity/enumeration Space) |
| Switch to selected target Space | FAIL — chord synthesis Ctrl+→/← relatif saja, bukan switch terbukti; tidak memenuhi required set |
| Move focused window across Spaces | NOT AVAILABLE (drag gesture/CGS only) |
| Observe changes | PARTIAL — `activeSpaceDidChangeNotification` (publik) terdaftar, tidak terbukti live |

**Keputusan:** workspace native `DISABLED` di macOS. Core tiling (Spike 1) berlanjut. Tidak ada simulate minimize/hide, tidak ada API private.

## 6. Rerun (sesi 2, 17 Okt 2026)

Mesin/build sama, harness tidak berubah. Toolchain re-verify: fmt OK, clippy clean, 6/6 test PASS @ `d2fc8ed`. Discovery 4 window; `run-tests` identik PASS (frame (100,668) 420×312, move ×3 eksak, resize 480×320, raise PASS); stale probe bersih; `space-observe` 40s NO EVENT; chord right/left di-posting, tidak ada perubahan Space. **Hasil spike #1 reproducible persis; tidak ada perubahan keputusan.**

## 7. Bukti command SHA

| Commit | Isi |
|---|---|
| `d2fc8ed` | Fix toolchain: cargo fmt + 2 clippy errors (base) |
| `a2d899b` | macOS spike #1: AX window control PASS, native workspaces DISABLED |
| `fb120d9` | Spike rerun: reproducible PASS, workspace unchanged |

## 8. Keterbatasan (jujur)

- 1 display 1.0x; multi-display/mixed-DPI NOT TESTED.
- Dock ineligible-probe inconclusive (tidak ada elemen AX via kAXWindowsAttribute di build ini).
- Mesin kemungkinan single-Space (chord test inconclusive) — keputusan tidak bergantung pada ini.
- Windows 11: belum diuji (tidak ada mesin target di sesi ini).

## 9. Checklist reproduksi (reviewer)

1. `git clone git@github.com:Niumination/hypr-rust.git && git checkout spike/macos-spike-1`
2. `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` → semua hijau
3. `swiftc -O spikes/macos/spike-macos.swift -o spikes/macos/spike-macos`
4. Pastikan AX permission granted untuk terminal host
5. `./spike-macos record && ./spike-macos spawn-target` (background) → catat PID
6. `./spike-macos run-tests --pid <pid>` → harapannya PASS semua
7. `./spike-macos space-observe --seconds 40` (terminal A) + `./spike-macos space-switch --dir right --execute --pid <pid>` (terminal B) → catat tidak ada perubahan Space
8. Kill target, ulangi `run-tests --pid <pid>` → clean error

**Verifikasi eksternal:** semua commit + PR ter-push ke `github.com:Niumination/hypr-rust` (public) — dapat direview dari arena.ai.
