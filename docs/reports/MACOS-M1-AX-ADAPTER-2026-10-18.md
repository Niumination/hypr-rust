# Laporan Verifikasi — M1 Rust AX Adapter (macOS Tahoe)

**Tanggal:** 18 Oktober 2026
**Branch:** `macos/tahoe-ax-adapter-m1` @ `91e57e2`
**Scope:** `docs/MACOS-TAHOE-NEXT-STAGE.md` — Stage M1, macOS-only vertical slice
**Metode:** implementasi + native run di host user (self-owned target window)

---

## 1. Environment (verbatim `scripts/collect-macos-env.sh` + tambahan manual)

```
== OS ==
ProductName:		macOS
ProductVersion:		26.7.1
BuildVersion:		25G241
architecture: x86_64
kernel: 25.6.0
cpu_brand: Intel(R) Core(TM) i5-10310U CPU @ 1.70GHz

== System-reported model (serial/UUID intentionally excluded) ==
      Model Name: MacBook Pro
      Model Identifier: MacBookPro16,2
      Processor Name: Quad-Core Intel Core i5
      Processor Speed: 2,2 GHz
      Total Number of Cores: 4
      Memory: 16 GB
```

Tambahan manual (per SYSTEM-INFO-COLLECTION.md langkah 4):
- Display: internal, tunggal (1920×1080 @1.0x, visible 1920×1050). Tidak ada display kedua.
- AX trust: `AXIsProcessTrusted=true` untuk proses Swift/cargo dari terminal host.

### Rekonsiliasi identitas host (wajib, konflik diketahui)

| Sumber | Nilai |
|---|---|
| Owner-reported (USER-DEVICE-PROFILE.md) | Lenovo ThinkPad X13 Yoga Gen 1 |
| System-reported (`system_profiler`) | Model Identifier **MacBookPro16,2** ("MacBook Pro") |
| CPU (owner + system) | Intel i5-10310U (konsisten ThinkPad X13 Yoga Gen 1) |

Spike #1 melaporkan host sebagai "Intel MacBook Pro 13-inch class" — **imprecise** (identifier sebenarnya MacBookPro16,2 = MacBook Pro 14" 2019) dan **bertabrakan** dengan klaim owner. CPU i5-10310U konsisten dengan ThinkPad X13 Yoga Gen 1; model identifier ternyata **spoof SMBIOS OpenCore** (`SystemProductName = MacBookPro16,2` di `EFI/oc/config.plist`, dibaca read-only 2026-10-18; serial/UUID tidak dibaca). Host = **Hackintosh**: OpenCore EFI + kext stack Acidanthera/Voodoo + SSDT laptop di perangkat fisik ThinkPad X13 Yoga Gen 1. Bukti lengkap: `docs/USER-DEVICE-PROFILE.md` §"Host identity — confirmed Hackintosh".

---

## 2. Toolchain

```
rustc 1.98.1 (48a229cea 2026-09-01) (Homebrew)
cargo 1.98.1 (797e8a9bc 2026-08-05) (Homebrew)
swift-driver version: 1.148.6 Apple Swift version 6.3.3 (swiftlang-6.3.3.1.3 clang-2100.1.1.101)
Target: x86_64-apple-macosx26.0
xcode-select -p: /Library/Developer/CommandLineTools
```

Catatan build: binary butuh `MACOSX_DEPLOYMENT_TARGET=26.0` (std Homebrew rustc dibangun untuk macOS 26; default rustc x86_64 = 10.12 → link error). Semua perintah cargo di bawah memakai env var ini.

---

## 3. Yang diimplementasi

Crate baru `crates/platform-macos` (workspace member; dependensi: `hypr-rust-layout-core` saja, tanpa crate eksternal):

- `src/ffi.rs` — satu-satunya deklarasi FFI mentah (ApplicationServices + CoreFoundation + `kill` libSystem). Semua `unsafe` di crate ini terkunci di sini + `adapter.rs` (wrapper berdokumentasi safety comment).
- `src/adapter.rs` — API aman: `MacAxAdapter::{connect, main_window, window_frame, set_window_frame, raise}`; `AxError` (PermissionDenied / StaleTarget / Unsupported / IllegalArgument / NoWindows / Failure(code)); mapping kode AXError diverifikasi terhadap HIServices host (macOS 26.7.1): success 0 · failure -25200 · illegalArgument -25201 · invalidUIElement -25202 · cannotComplete -25204 · attributeUnsupported -25205 · actionUnsupported -25206 · notImplemented -25208 · apiDisabled -25211 · noValue -25212 · parameterizedAttributeUnsupported -25213 · notEnoughPrecision -25214.
- `src/bin/ax-probe.rs` — CLI native test: `frame <pid>`, `set-frame <pid> x y w h` (dengan read-back verification ≤1pt), `raise <pid>`. Exit: 0 ok · 1 error · 3 permission denied · 4 stale target.

Temuan implementasi (jujur):
- Konstanta SDK `kAXWindowsAttribute` dll adalah **macro `CFSTR("...")`**, bukan symbol ekspor — key dibuat sekali via `OnceLock` + `CFStringCreateWithCString`.
- Dead pid di build ini mengembalikan `kAXErrorCannotComplete` (-25204), bukan `invalidUIElement` — stale dideteksi via pre-check `kill(pid, 0)`.

---

## 4. Verifikasi toolchain (verbatim)

`platform-macos` sengaja **bukan** workspace member (lihat §6 P1-2): command Windows-safe untuk workspace + command eksplisit untuk crate macOS.

```
$ cargo fmt --check                                  # workspace (layout-core)
OK
$ cargo fmt --manifest-path crates/platform-macos/Cargo.toml --check
OK
$ MACOSX_DEPLOYMENT_TARGET=26.0 cargo clippy --workspace --all-targets -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 6.69s
$ MACOSX_DEPLOYMENT_TARGET=26.0 cargo clippy --manifest-path crates/platform-macos/Cargo.toml --all-targets -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.03s
$ MACOSX_DEPLOYMENT_TARGET=26.0 cargo test --workspace
test result: ok. 6 passed; 0 failed; ... (layout-core)
$ cd crates/platform-macos && MACOSX_DEPLOYMENT_TARGET=26.0 cargo test
test result: ok. 5 passed; 0 failed; ... (platform-macos: AXError mapping, Display, liveness ESRCH/EPERM, main-window fallback decision, frame validation)
```

11/11 test hijau (6 workspace + 5 platform-macos).

---

## 5. Native run di host (verbatim)

**Rerun 18 Okt 2026 (post-review, adapter fixed)** — target: self-owned `HYPR-SPIKE-TARGET` (window 420×312) di-spawn oleh Swift spike harness (disposable, versi Dock-fix @ `669e178`). Probe Rust hanya memutasi pid target eksplisit.

```
$ /tmp/spike-macos spawn-target &        # PID 10042
$ ax-probe frame 10042
ok {x: 100, y: 668, width: 420, height: 312}
$ ax-probe set-frame 10042 120 120 420 312
ok {x: 120, y: 120, width: 420, height: 312}
$ ax-probe set-frame 10042 180 140 420 312
ok {x: 180, y: 140, width: 420, height: 312}
$ ax-probe set-frame 10042 150 130 420 312
ok {x: 150, y: 130, width: 420, height: 312}
$ ax-probe set-frame 10042 150 130 480 320
ok {x: 150, y: 130, width: 480, height: 320}
$ ax-probe raise 10042
ok raised
$ ax-probe set-frame 10042 nan 130 480 320
error: invalid geometry (non-finite or non-positive size)
invalid-exit=1
$ ax-probe set-frame 10042 150 130 0 320
error: invalid geometry (non-finite or non-positive size)
zero-exit=1
$ ax-probe frame 10042                  # window unchanged after rejects
ok {x: 150, y: 130, width: 480, height: 320}
$ kill 10042; sleep 1
$ ax-probe frame 10042
error: stale target (process/window gone)
stale-exit=4
```

Rerun identik dengan run pertama (17 Okt) untuk semua operasi inti; validasi geometry kini live-terverifikasi (NaN dan zero-size ditolak SEBELUM write apa pun, window tetap di 150,130 480×320).

**Rollback path** (`SizeAfterPositionFailed`): tidak dapat diuji runtime tanpa memaksa AX menolak size-write setelah position-write diterima (tidak ada switch AX publik untuk itu). Kontrak yang diterapkan: preflight finite/positive → save geometri awal → tulis posisi → tulis ukuran → jika ukuran ditolak: restore posisi awal + verifikasi read-back → error menyatakan eksplisit bila rollback gagal. Murni-fungsi yang mendasarinya (validasi, keputusan fallback, mapping errno) teruji unit.

**Leak fix** (P2-3): jalur sukses `main_window()` melepas ref aplikasi setelah window di-transfer ke `AxWindow` — kini ter-verifikasi code path (setiap panggilan sukses melepas `app` tepat sekali).

---

## 6. Checklist acceptance criteria (MACOS-TAHOE-NEXT-STAGE.md)

| Kriteria | Status | Bukti |
|---|---|---|
| Baseline fmt/clippy/test + toolchain version | ✅ | §2, §4 |
| Self-owned window discovered/read/moved/resized repeatedly, read-back within tolerance | ✅ | §5 (move×3 + resize, read-back eksak ≤1pt) |
| Probe mutasi hanya target sendiri | ✅ | ax-probe operasi pada pid eksplisit; tidak ada enumerasi window user; tidak ada judul di log |
| AX denied path exercised atau ditandai "not runtime tested" | ⚠️ **NOT RUNTIME TESTED** | Code path ada: `connect()` gate `AXIsProcessTrusted()` → `PermissionDenied` (exit 3). Host memegang AX trust; revoke butuh TCC reset + System Settings (merusak environment test). Ditandai jujur. |
| Stale target: prompt, tanpa crash/hang/retry/false success | ✅ | §5: exit 4, error typed, <1s |
| Log tanpa judul/serial/UUID/kredensial | ✅ | Output hanya angka geometry + pid |
| Host/OS exact + workspaces disabled tercatat | ✅ | §1; tidak ada kode Spaces di crate |
| Tidak ada private API / dependensi baru / global loop / startup service | ✅ | FFI hanya ApplicationServices + CoreFoundation public; dependensi cuma layout-core |

### Review upstream (arena.ai) — semua temuan ditangani

| Temuan | Status | Perbaikan |
|---|---|---|
| P1: `set_window_frame` partial-mutation | ✅ | Preflight finite/positive (`InvalidGeometry`, live-tested §5) + save-before-write + rollback posisi dengan verifikasi read-back + `SizeAfterPositionFailed` eksplisit |
| P1: crate macOS tanpa gating di workspace | ✅ | `platform-macos` dikeluarkan dari `members` (§4); command workspace tetap Windows-safe; crate memakai `[workspace]` kosong (standalone) |
| P2: leak CF ref di jalur sukses `main_window()` | ✅ | `CFRelease(app)` setelah window transfer; semua path keluar kini melepas `app` tepat sekali |
| P2: fallback AXWindows tidak untuk error AX | ✅ | `is_missing_main_window`: noValue/attributeUnsupported → fallback; permission/stale/cannotComplete/illegal tetap propagasi |
| P2: langkah reproduksi merujuk file yang tidak ada di branch | ✅ | §8 diperbaiki (ambil harness dari branch spike via `git show`) |
| P2: `kill(pid,0)` menyamakan EPERM dengan mati | ✅ | Cek errno: hanya ESRCH → stale; EPERM/EFOTHER → hidup, AX surfacing error asli |

---

## 7. Keterbatasan jujur

- Permission-denied path tidak diuji runtime (lihat §6).
- Single display 1.0x; multi-display/multi-DPI tidak diuji.
- `kill(pid,0)` existence check tidak kebal pid-reuse (probe short-lived; diterima).
- Host: model identifier sistem (MacBookPro16,2) ≠ perangkat fisik owner (ThinkPad X13 Yoga) — lihat §1.
- Windows 11: belum ada mesin target; klaim Windows menunggu native test.

## 8. Reproduksi

```
git clone https://github.com/Niumination/hypr-rust && cd hypr-rust
git checkout macos/tahoe-ax-adapter-m1

# workspace commands (Windows-safe; layout-core only)
cargo fmt --check
MACOSX_DEPLOYMENT_TARGET=26.0 cargo clippy --workspace --all-targets -- -D warnings
MACOSX_DEPLOYMENT_TARGET=26.0 cargo test --workspace

# macOS-only crate (standalone, outside workspace members)
cargo fmt --manifest-path crates/platform-macos/Cargo.toml --check
MACOSX_DEPLOYMENT_TARGET=26.0 cargo clippy --manifest-path crates/platform-macos/Cargo.toml --all-targets -- -D warnings
cd crates/platform-macos
MACOSX_DEPLOYMENT_TARGET=26.0 cargo test                    # 5/5
MACOSX_DEPLOYMENT_TARGET=26.0 cargo build --bin ax-probe    # binary: crates/platform-macos/target/debug/ax-probe

# spawn target: Swift harness lives on the spike branch, not this one
git show spike/macos-spike-1:spikes/macos/spike-macos.swift > /tmp/spike-macos.swift
swiftc -O /tmp/spike-macos.swift -o /tmp/spike-macos && /tmp/spike-macos spawn-target &

./target/debug/ax-probe frame <pid>            # baca
./target/debug/ax-probe set-frame <pid> 120 120 420 312
./target/debug/ax-probe set-frame <pid> nan 0 100 100   # ditolak: invalid geometry
./target/debug/ax-probe raise <pid>
kill <pid>; ./target/debug/ax-probe frame <pid>          # exit 4
```
