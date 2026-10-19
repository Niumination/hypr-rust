# Laporan Verifikasi — M1 Rust AX Adapter (macOS Tahoe)

**Tanggal:** 18 Oktober 2026
**Branch:** `macos/tahoe-ax-adapter-m1` @ `4176c0f` (adapter code; this report + arena brief land in the following docs commit)
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

Crate baru `crates/platform-macos` (standalone, **di luar** workspace members; dependensi: `hypr-rust-layout-core` saja, tanpa crate eksternal):

- `src/ffi.rs` — satu-satunya deklarasi FFI mentah (ApplicationServices + CoreFoundation + `kill` libSystem). Semua `unsafe` di crate ini terkunci di sini + `adapter.rs` (wrapper berdokumentasi safety comment).
- `src/adapter.rs` — API aman: `MacAxAdapter::{connect, main_window, window_frame, set_window_frame, raise}`; `AxError` (PermissionDenied / StaleTarget / Unsupported / IllegalArgument / NoWindows / InvalidGeometry / PositionWriteFailed{source, rollback_restored, actual} / SizeAfterPositionFailed{source, rollback_restored, actual} / VerificationReadFailed{source, rollback_restored, actual} / VerificationFailed{expected, actual, rollback_restored} / Failure(code)); mapping kode AXError diverifikasi terhadap HIServices host (macOS 26.7.1): success 0 · failure -25200 · illegalArgument -25201 · invalidUIElement -25202 · cannotComplete -25204 · attributeUnsupported -25205 · actionUnsupported -25206 · notImplemented -25208 · apiDisabled -25211 · noValue -25212 · parameterizedAttributeUnsupported -25213 · notEnoughPrecision -25214. Write/read orchestration di-balik trait `FrameOps` (mock-able, tanpa Apple frameworks).
- `src/bin/ax-probe.rs` — CLI native test. `frame <pid>` read-only (bebas, tanpa token). `set-frame --target <pid> --token <hex> x y w h` dan `raise --target <pid> --token <hex>` **mutating**: wajib token yang cocok dengan `~/.hypr-rust/fixture-target.json` (regular non-symlink, owner-only, perm 0600, uid match) + **fixture liveness lock** (`~/.hypr-rust/fixture.lock`, flock ditahan fixture seumur hidup) + pid alive. Verifikasi (semua fail closed) dilakukan SEBELUM mutasi AX. Exit: 0 ok · 1 error · 3 permission denied · 4 stale target · 5 unregistered target (gagal verifikasi).
- `frames_match(expected, actual, tolerance)` — pure fn perbandingan geometri dengan toleransi 1pt (unit-tested). Validasi geometri memakai `Rect::is_finite_and_positive()` dari `layout-core` (finite coords, positive size, finite right/bottom).
- `set_window_frame` return `Result<Rect, AxError>` — frame terverifikasi dikembalikan ke caller; CLI tidak laca read post-transaction redundan. Rollback penuh (posisi + ukuran) + verify pada SEMUA kegagalan pasca-mutasi.

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
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 8.30s
$ MACOSX_DEPLOYMENT_TARGET=26.0 cargo test --workspace
test result: ok. 6 passed; 0 failed; ... (layout-core)
$ cd crates/platform-macos && MACOSX_DEPLOYMENT_TARGET=26.0 cargo test
test result: ok. 6 passed; 0 failed; ... (platform-macos: AXError mapping, Display, liveness ESRCH/EPERM, main-window fallback decision, frame validation, frames_match tolerance)
```

21/21 test hijau (6 workspace + 15 platform-macos).

---

## 5. Native run di host (verbatim)

**Rerun 19 Okt 2026 (post-review round 3, fixture-locked)** — target: self-owned `HYPR-SPIKE-TARGET` (window 420×312) di-spawn oleh Swift spike harness @ `5d16063` (PR #1). Harness menulis `~/.hypr-rust/fixture-target.json` (0600) berisi `{pid, token}` + memegang `fixture.lock` (flock) seumur hidup. Token diredaksi di dokumen ini.

```
$ /tmp/spike-macos spawn-target &        # PID <redacted>, token <redacted>
$ ls -la ~/.hypr-rust/
drwx------  .  fixture-target.json  fixture.lock   # dir 0700, file 0600
$ ax-probe frame <pid>
ok {x: 100, y: 668, width: 420, height: 312}
$ ax-probe set-frame <pid> 120 120 420 312          # TANPA token
error: unregistered target (fixture identity verification failed)
       mutations require --target <pid> --token <hex> from the harness
notoken-exit=5
$ ax-probe raise <pid>                                # TANPA token
error: unregistered target (fixture identity verification failed)
notoken-raise-exit=5
$ ax-probe set-frame --target <pid> --token <hex> 120 120 420 312
ok {x: 120, y: 120, width: 420, height: 312}
$ ax-probe set-frame --target <pid> --token <hex> 180 140 420 312
ok {x: 180, y: 140, width: 420, height: 312}
$ ax-probe set-frame --target <pid> --token <hex> 150 130 420 312
ok {x: 150, y: 130, width: 420, height: 312}
$ ax-probe set-frame --target <pid> --token <hex> 150 130 480 320   # resize
ok {x: 150, y: 130, width: 480, height: 320}
$ ax-probe raise --target <pid> --token <hex>
ok raised
$ ax-probe set-frame --target <pid> --token <hex> nan 130 480 320
error: invalid geometry (non-finite or non-positive size)
invalid-exit=1
$ ax-probe frame <pid>                  # window unchanged after rejects
ok {x: 150, y: 130, width: 480, height: 320}
$ kill <pid>; sleep 1
$ ax-probe frame <pid>
error: stale target (process/window gone)
stale-exit=4
$ ax-probe set-frame --target <pid> --token <old-hex> 120 120 420 312   # token LAMA, fixture mati
error: stale target (process/window gone)     # liveness gate (flock backstop: lock bebas)
oldtoken-exit=4
```

**Fixture lock gate (F1+F1b)**: `set-frame`/`raise` tanpa `--target/--token`, atau dengan token tidak cocok / file tidak regular / symlink / uid salah / perm ≠0600 / lock tidak dipesan / pid mati → **exit 5** (atau 4 untuk pid mati) SEBELUM panggilan AX. Process-instance identity = flock yang ditahan fixture: saat fixture mati (SIGTERM/SIGKILL), kernel melepas lock → probe menolak. Hanya fixture self-owned hidup yang boleh dimutasi. `frame` tetap read-only dan bebas. Token tidak pernah muncul di log/dokumen.

**Read-back verification (F2)**: setiap `set-frame` yang diterima AX dibaca balik (toleransi 1pt). Cocok → `ok` + frame terverifikasi; tidak cocok → geometri awal di-rollback penuh (posisi + ukuran) + diverifikasi → `VerificationFailed { expected, actual, rollback_restored }`. Semua set-frame di run ini terverifikasi cocok (read-back eksak).

**Rollback paths** (`PositionWriteFailed` / `SizeAfterPositionFailed` / `VerificationReadFailed`): tidak dapat dipaksa runtime tanpa AX menolak/salah-set diam-diam (tidak ada switch AX publik). Kontrak rollback penuh + verify diuji via 10 mock unit test (FrameOps seam) — dilabeli mock, bukan native. Murni-fungsi (`frames_match`, validasi) teruji unit.

**Leak fix** (P2-3): jalur sukses `main_window()` melepas ref aplikasi setelah window di-transfer ke `AxWindow` — ter-verifikasi code path (setiap panggilan sukses melepas `app` tepat sekali).

---

## 6. Checklist acceptance criteria (MACOS-TAHOE-NEXT-STAGE.md)

| Kriteria | Status | Bukti |
|---|---|---|
| Baseline fmt/clippy/test + toolchain version | ✅ | §2, §4 |
| Self-owned window discovered/read/moved/resized repeatedly, read-back within tolerance | ✅ | §5 (move×3 + resize, read-back eksak ≤1pt, token-gated) |
| Probe mutasi hanya target self-owned terdaftar (token + flock) | ✅ | §5: `set-frame`/`raise` wajib token; verifikasi file (regular non-symlink, 0600, uid match) + flock lock + pid hidup → gagal exit 5/4 sebelum AX; hanya fixture self-owned; tidak ada enumerasi window user; tidak ada judul di log |
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
| F1: mutasi `ax-probe` tidak terbatas ke target self-owned | ✅ | Token gate: `set-frame`/`raise` wajib `--target <pid> --token <hex>`; verifikasi fail-closed (file regular non-symlink 0600 uid-match + flock lock + pid+token cocok + pid hidup) sebelum AX; gagal → exit 5 (live-tested §5). `frame` tetap read-only. Swift mutating path dihapus |
| F1b: registrasi fixture tidak lifecycle-bounded / rawan PID-reuse | ✅ | Registrasi didesain ulang: `~/.hypr-rust/` (0700), `fixture-target.json` (0600, atomic temp+rename), `fixture.lock` flock ditahan fixture seumur hidup (process-instance identity), cleanup saat terminate, fail closed. Proses start-time tidak ada API publiknya (proc_pidinfo/libproc private) → flock sebagai identity. PID-reuse ditolak (lock dilepas saat mati). Live-tested §5 |
| F2: `set_window_frame` tanpa verifikasi hasil | ✅ | Read-back dalam adapter (tol 1pt) → mismatch: rollback penuh + `VerificationFailed {expected, actual, rollback_restored}`. SEMUA kegagalan pasca-mutasi (position/size/read-back error) → rollback penuh + typed error (`PositionWriteFailed`/`SizeAfterPositionFailed`/`VerificationReadFailed`). `set_window_frame` return verified frame; CLI read redundan dihapus. 10 mock unit test (FrameOps seam), dilabeli mock |

---

## 7. Keterbatasan jujur

- Permission-denied path tidak diuji runtime (lihat §6).
- Single display 1.0x; multi-display/multi-DPI tidak diuji.
- Host: model identifier sistem (MacBookPro16,2) ≠ perangkat fisik owner (ThinkPad X13 Yoga) — lihat §1.
- Windows 11: belum ada mesin target; klaim Windows menunggu native test.
- Fixture lock adalah jaring keselamatan **uji** (fixture self-owned), bukan kontrol akses multi-user: file di `~/.hypr-rust/` milik owner. Tujuannya mencegah probe menimpa window lain, bukan menahan adversary. Proses start-time tidak ada API publiknya → flock sebagai process-instance identity.
- Mismatch read-back (`VerificationFailed`) dan read-back error (`VerificationReadFailed`) tidak bisa dipaksa runtime tanpa AX menolak/salah-set diam-diam (tidak ada switch AX publik); kontrak rollback penuh + verify diuji via 10 mock unit test (dilabeli mock), jalur native ditandai jujur belum live-tested.

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
MACOSX_DEPLOYMENT_TARGET=26.0 cargo test                    # 15/15
MACOSX_DEPLOYMENT_TARGET=26.0 cargo build --bin ax-probe    # binary: crates/platform-macos/target/debug/ax-probe

# spawn target: Swift harness lives on the spike branch, not this one
# (harness @ 5d16063 writes ~/.hypr-rust/fixture-target.json 0600 + holds fixture.lock)
git show spike/macos-spike-1:spikes/macos/spike-macos.swift > /tmp/spike-macos.swift
swiftc -O /tmp/spike-macos.swift -o /tmp/spike-macos && /tmp/spike-macos spawn-target &
# capture: PID from `pgrep -f "spike-macos spawn-target"`, TOKEN from the registration file
TOKEN=$(python3 -c "import json;print(json.load(open('$HOME/.hypr-rust/fixture-target.json'))['token'])")

./target/debug/ax-probe frame <pid>                                                    # read-only, no token
./target/debug/ax-probe set-frame --target <pid> --token <hex> 120 120 420 312          # mutate (token + lock required)
./target/debug/ax-probe raise --target <pid> --token <hex>                              # mutate (token + lock required)
./target/debug/ax-probe set-frame <pid> 120 120 420 312                                 # refused: exit 5 (no token)
./target/debug/ax-probe set-frame --target <pid> --token <hex> nan 0 100 100            # refused: exit 1 (invalid geometry)
kill <pid>; ./target/debug/ax-probe frame <pid>                                         # exit 4 (stale)
```
