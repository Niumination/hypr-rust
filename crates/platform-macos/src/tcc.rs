//! TCC grant-drift detection: distinguish a **stale** Accessibility grant
//! from a **never-granted** denial.
//!
//! TCC records the signature (cdhash) of the granted code at add time, and
//! ad-hoc signing produces a new cdhash on every build. After a rebuild the
//! recorded entry no longer matches the running code, `AXIsProcessTrusted`
//! reports denied, and the generic "grant it" instruction is misleading —
//! the entry already exists and must be **removed and re-added** (owner-
//! confirmed ritual, 2026-10-26; see docs/USING-AX-SHELL.md).
//!
//! The running executable's cdhash is read by spawning the system tool
//! `/usr/bin/codesign -dv --verbose=4 <path>` with a fixed argv (no shell,
//! no input from window metadata or config — the path is `current_exe()`).
//! The cdhash of a running executable cannot change during the process
//! lifetime, so it is cached once.
//!
//! Design owner decision 2026-11-09: state file + denied-only message.
//! This module never grants, revokes or modifies any TCC entry; it only
//! reads signatures and compares them with a local record.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Verdict of the recorded-grant comparison shown only on `denied`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Staleness {
    /// Both cdhashes known and different: the grant predates this build.
    Stale,
    /// Both known and equal: the denial has another cause (e.g. removed
    /// entry, or AX toggled off) — the generic instruction applies.
    Match,
    /// A cdhash is unavailable (unsigned binary, `codesign` failure, no
    /// record yet): never claim drift without evidence.
    Unknown,
}

/// Parse `CDHash=<hex>` out of `codesign -dv --verbose=4` output.
///
/// Both streams must be passed in: on this build `codesign` writes its
/// verbose report to **stderr** and nothing to stdout (probed 2026-11-09).
/// An unsigned input produces `code object is not signed at all` and no
/// `CDHash=` line → `None`. The plural `CDHashes=` (fat binaries) does not
/// match the `CDHash=` prefix and also yields `None` (fail to "unknown",
/// never to a fabricated hash).
pub fn parse_cdhash(output: &str) -> Option<String> {
    output
        .lines()
        .find_map(|line| line.trim().strip_prefix("CDHash=").map(str::trim))
        .filter(|hash| !hash.is_empty())
        .map(str::to_string)
}

/// Read the cdhash of the code signed at `exe` (best-effort).
///
/// Spawns `/usr/bin/codesign` with a fixed argv; any failure (missing tool,
/// unreadable path, unsigned code) yields `None`, which callers surface as
/// [`Staleness::Unknown`] — a denial is then reported with the generic
/// instruction, never with a false stale-grant claim.
pub fn cdhash_of(exe: &Path) -> Option<String> {
    let output = std::process::Command::new("/usr/bin/codesign")
        .arg("-dv")
        .arg("--verbose=4")
        .arg(exe)
        .output()
        .ok()?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    parse_cdhash(&text)
}

/// The running executable's cdhash, cached for the process lifetime.
///
/// The code object of a running process cannot change, so one spawn (at
/// most) per process; a `None` is cached too and re-attempted never.
pub fn current_cdhash() -> Option<&'static str> {
    static CDHASH: OnceLock<Option<String>> = OnceLock::new();
    CDHASH
        .get_or_init(|| {
            std::env::current_exe()
                .ok()
                .and_then(|path| cdhash_of(&path))
        })
        .as_deref()
}

/// Path of the grant record: `~/.hypr-rust/ax-grant.state`.
pub fn state_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(
        PathBuf::from(home)
            .join(".hypr-rust")
            .join("ax-grant.state"),
    )
}

/// Read the recorded cdhash. Missing, empty, symlink or non-regular files
/// all yield `None` → [`Staleness::Unknown`] (the record is an app-owned
/// hint, never a security boundary).
pub fn read_recorded(path: &Path) -> Option<String> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if meta.file_type().is_symlink() || !meta.file_type().is_file() {
        return None;
    }
    let body = std::fs::read_to_string(path).ok()?;
    let trimmed = body.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Record the cdhash that Accessibility currently trusts.
///
/// Writes only when the value changed (temp file + rename, owner-only 0600,
/// same pattern as the config save). Returns whether a write happened;
/// every failure is `false` — callers treat recording as best-effort and a
/// missing record only ever downgrades the message to the generic one.
pub fn record_granted(path: &Path, cdhash: &str) -> bool {
    if cdhash.is_empty() {
        return false;
    }
    if read_recorded(path).as_deref() == Some(cdhash) {
        return false;
    }
    if let Some(dir) = path.parent() {
        if std::fs::create_dir_all(dir).is_err() {
            return false;
        }
    }
    let tmp = path.with_extension("state.tmp");
    {
        use std::io::Write;
        let Ok(mut file) = std::fs::File::create(&tmp) else {
            return false;
        };
        if file.write_all(cdhash.as_bytes()).is_err() || file.sync_all().is_err() {
            return false;
        }
    }
    use std::os::unix::fs::PermissionsExt;
    if std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)).is_err() {
        return false;
    }
    std::fs::rename(&tmp, path).is_ok()
}

/// Compare the running cdhash with the recorded one (pure).
///
/// `Stale` requires **both** values present and different; anything less is
/// `Unknown`, so an unsigned development build or a first run without a
/// record can never be mislabeled as a stale grant. Both sides are compared
/// trimmed so trailing newlines cannot fake a difference.
pub fn staleness(current: Option<&str>, recorded: Option<&str>) -> Staleness {
    match (current, recorded) {
        (Some(current), Some(recorded)) if current.trim() != recorded.trim() => Staleness::Stale,
        (Some(_), Some(_)) => Staleness::Match,
        _ => Staleness::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    fn scratch_dir() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "hypr-rust-tcc-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).expect("create isolated test dir");
        path
    }

    // Verbatim stderr of `codesign -dv --verbose=4` on the dist bundle
    // (probe 2026-11-09; stdout was empty).
    const SIGNED_OUTPUT: &str = "Executable=/Users/zaryu/Desktop/Niumination/labs/hypr-rust/crates/platform-macos/target/dist/ax-shell.app/Contents/MacOS/ax-shell\n\
Identifier=io.github.niumination.ax-shell\n\
Format=app bundle with Mach-O thin (x86_64)\n\
CandidateCDHash sha256=b087e2be1ffa0d1c1c1738a27e03a75f790a7a4d\n\
CandidateCDHashFull sha256=b087e2be1ffa0d1c1c1738a27e03a75f790a7a4d2b6bc3f6b6d7d0bed32dcf43\n\
CDHash=b087e2be1ffa0d1c1c1738a27e03a75f790a7a4d\n\
Signature=adhoc\n\
TeamIdentifier=not set\n";

    const UNSIGNED_OUTPUT: &str =
        "crates/platform-macos/target/debug/ax-shell: code object is not signed at all\n";

    #[test]
    fn parses_the_cdhhash_line_from_signed_output() {
        assert_eq!(
            parse_cdhash(SIGNED_OUTPUT),
            Some("b087e2be1ffa0d1c1c1738a27e03a75f790a7a4d".to_string())
        );
    }

    #[test]
    fn unsigned_output_yields_none() {
        assert_eq!(parse_cdhash(UNSIGNED_OUTPUT), None);
        assert_eq!(parse_cdhash(""), None);
    }

    #[test]
    fn plural_cdhshes_line_is_not_mistaken_for_a_hash() {
        let fat = "CDHashes=(\"abc123\", \"def456\")\nSignature=adhoc\n";
        assert_eq!(parse_cdhash(fat), None);
    }

    #[test]
    fn cdhash_of_an_unsigned_temp_file_is_none() {
        let dir = scratch_dir();
        let exe = dir.join("unsigned-bin");
        std::fs::write(&exe, b"not a real binary").unwrap();
        assert_eq!(cdhash_of(&exe), None);
        assert_eq!(cdhash_of(&dir.join("does-not-exist")), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn staleness_needs_both_hashes_and_a_difference() {
        let a = "aaaa";
        let b = "bbbb";
        assert_eq!(staleness(Some(a), Some(b)), Staleness::Stale);
        assert_eq!(staleness(Some(a), Some(a)), Staleness::Match);
        assert_eq!(staleness(Some(a), None), Staleness::Unknown);
        assert_eq!(staleness(None, Some(a)), Staleness::Unknown);
        assert_eq!(staleness(None, None), Staleness::Unknown);
        // Whitespace in the record must not fake a drift.
        assert_eq!(staleness(Some(a), Some(" aaaa ")), Staleness::Match);
    }

    #[test]
    fn record_then_read_roundtrips_owner_only_and_skips_unchanged_writes() {
        use std::os::unix::fs::PermissionsExt;

        let dir = scratch_dir();
        let path = dir.join("ax-grant.state");
        assert!(record_granted(&path, "aaaa"));
        assert!(record_granted(&path, "bbbb"));
        assert_eq!(read_recorded(&path).as_deref(), Some("bbbb"));
        // Unchanged value: no rewrite.
        assert!(!record_granted(&path, "bbbb"));

        let meta = std::fs::symlink_metadata(&path).unwrap();
        assert!(meta.file_type().is_file());
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);

        // Empty hash is never recorded; missing file reads as None.
        assert!(!record_granted(&dir.join("other.state"), ""));
        assert_eq!(read_recorded(&dir.join("missing")), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn symlinked_record_reads_as_none() {
        let dir = scratch_dir();
        let target = dir.join("target.txt");
        std::fs::write(&target, b"bbbb").unwrap();
        let link = dir.join("ax-grant.state");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert_eq!(read_recorded(&link), None);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
