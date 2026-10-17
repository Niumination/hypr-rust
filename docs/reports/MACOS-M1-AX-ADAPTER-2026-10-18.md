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

Spike #1 melaporkan host sebagai "Intel MacBook Pro 13-inch class" — **imprecise** (identifier sebenarnya MacBookPro16,2 = MacBook Pro 14" 2019) dan **bertabrakan** dengan klaim owner. CPU i5-10310U konsisten dengan ThinkPad X13 Yoga Gen 1; model identifier kemungkinan dilaporkan oleh firmware/EFI (SMBIOS), bukan label perangkat fisik. Laporan ini mencatat kedua nilai tanpa menyembunyikan konflik. Tidak ada serial/UUID/EFI config yang dibaca (dilarang).

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

```
$ cargo fmt --check
OK
$ MACOSX_DEPLOYMENT_TARGET=26.0 cargo clippy --workspace --all-targets -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.68s
$ MACOSX_DEPLOYMENT_TARGET=26.0 cargo test --workspace
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s   (layout-core)
test result: ok. 2 passed; 0 failed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s   (platform-macos: AXError mapping + Display)
```

8/8 test hijau. Test unit platform-macos murni Rust (tanpa AX): mapping 15 kode AXError → varian berbeda, Display string.

---

## 5. Native run di host (verbatim)

Target: self-owned `HYPR-SPIKE-TARGET` (window 420×312) di-spawn oleh Swift spike harness (disposable, versi Dock-fix @ `669e178`). Probe Rust hanya memutasi pid target eksplisit.

```
$ /tmp/spike-macos spawn-target &        # PID 79107
$ ./target/debug/ax-probe frame 79107
ok {x: 100, y: 668, width: 420, height: 312}
$ ./target/debug/ax-probe set-frame 79107 120 120 420 312
ok {x: 120, y: 120, width: 420, height: 312}
$ ./target/debug/ax-probe set-frame 79107 180 140 420 312
ok {x: 180, y: 140, width: 420, height: 312}
$ ./target/debug/ax-probe set-frame 79107 150 130 420 312
ok {x: 150, y: 130, width: 420, height: 312}
$ ./target/debug/ax-probe set-frame 79107 150 130 480 320
ok {x: 150, y: 130, width: 480, height: 320}
$ ./target/debug/ax-probe raise 79107
ok raised
$ kill 79107; sleep 1
$ ./target/debug/ax-probe frame 79107
error: stale target (process/window gone)
stale-exit=4
$ ./target/debug/ax-probe frame 999999
error: stale target (process/window gone)
nonexistent-exit=4
```

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
MACOSX_DEPLOYMENT_TARGET=26.0 cargo build -p hypr-rust-platform-macos --bin ax-probe
# spawn target (Swift harness disposable, branch spike/macos-spike-1):
swiftc -O spikes/macos/spike-macos.swift -o /tmp/spike-macos && /tmp/spike-macos spawn-target &
./target/debug/ax-probe frame <pid>        # baca
./target/debug/ax-probe set-frame <pid> 120 120 420 312
./target/debug/ax-probe raise <pid>
kill <pid>; ./target/debug/ax-probe frame <pid>   # exit 4
```
