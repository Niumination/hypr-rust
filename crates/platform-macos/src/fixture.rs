//! Fixture registration verification shared by the probe and keyboard
//! binaries. Fail closed on any problem: the registration file must exist,
//! be a regular non-symlink file owned by the current user with perm 0600,
//! contain a matching pid + token, the pid must be alive, and the fixture
//! liveness lock must be held by a live process (only EWOULDBLOCK/EAGAIN
//! counts as "held"; any other flock error rejects).

use std::path::PathBuf;

use crate::ffi;
use crate::process_alive;

/// Fixture verification failure.
#[derive(Debug)]
pub enum FixtureError {
    /// Registration/lock verification failed (unregistered target).
    Unregistered,
    /// Target process is gone.
    Stale,
}

fn fixture_dir() -> Result<PathBuf, ()> {
    let home = std::env::var("HOME").map_err(|_| ())?;
    Ok(PathBuf::from(home).join(".hypr-rust"))
}

fn registration_path() -> Result<PathBuf, ()> {
    Ok(fixture_dir()?.join("fixture-target.json"))
}

fn lock_path() -> Result<PathBuf, ()> {
    Ok(fixture_dir()?.join("fixture.lock"))
}

/// The registered fixture's pid, only when the registration is valid, the pid
/// is alive and the liveness lock is held.
///
/// Unlike [`verify_registration`] this does not need the token: the shell is the
/// product and never learns the fixture token. It still fails closed on a
/// missing/stale registration or a free lock.
pub fn registered_fixture_pid() -> Option<u32> {
    let path = registration_path().ok()?;
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if meta.file_type().is_symlink() || !meta.file_type().is_file() {
        return None;
    }
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    if meta.uid() != unsafe { ffi::getuid() } {
        return None;
    }
    if meta.permissions().mode() & 0o777 != 0o600 {
        return None;
    }
    let body = std::fs::read_to_string(&path).ok()?;
    let (pid, _token) = parse_registration(&body)?;
    if !process_alive(pid) {
        return None;
    }
    verify_lock().ok()?;
    Some(pid)
}

/// Verify the fixture registration end-to-end (fail closed).
pub fn verify_registration(pid: u32, token: &str) -> Result<(), FixtureError> {
    let path = registration_path().map_err(|_| FixtureError::Unregistered)?;
    let meta = std::fs::symlink_metadata(&path).map_err(|_| FixtureError::Unregistered)?;
    // Must be a regular file, NOT a symlink (lstat, no symlink following).
    if meta.file_type().is_symlink() || !meta.file_type().is_file() {
        return Err(FixtureError::Unregistered);
    }
    // Owner-only: uid must match the current user, perms exactly 0600.
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    if meta.uid() != unsafe { ffi::getuid() } {
        return Err(FixtureError::Unregistered);
    }
    if meta.permissions().mode() & 0o777 != 0o600 {
        return Err(FixtureError::Unregistered);
    }
    let body = std::fs::read_to_string(&path).map_err(|_| FixtureError::Unregistered)?;
    let (file_pid, file_token) = parse_registration(&body).ok_or(FixtureError::Unregistered)?;
    if file_pid != pid || file_token != token {
        return Err(FixtureError::Unregistered);
    }
    if !process_alive(pid) {
        return Err(FixtureError::Stale);
    }
    verify_lock()?;
    Ok(())
}

/// Verify the fixture liveness lock is held by a live process. If the caller
/// can acquire the exclusive lock, no live fixture holds it (stale record or
/// reused PID) -> reject. Only lock contention (EWOULDBLOCK/EAGAIN) counts as
/// "held by a live fixture"; any other flock error is rejected (fail closed).
fn verify_lock() -> Result<(), FixtureError> {
    use ffi::{__error, close, flock, open, K_ERRNO_EWOULDBLOCK, LOCK_EX, LOCK_NB, O_RDWR};
    let lock = lock_path().map_err(|_| FixtureError::Unregistered)?;
    let cstr = std::ffi::CString::new(lock.as_os_str().as_encoded_bytes())
        .map_err(|_| FixtureError::Unregistered)?;
    // SAFETY: cstr is a valid NUL-terminated C string.
    let fd = unsafe { open(cstr.as_ptr(), O_RDWR, 0o600) };
    if fd < 0 {
        return Err(FixtureError::Unregistered);
    }
    // SAFETY: fd is a valid open file descriptor.
    let ret = unsafe { flock(fd, LOCK_EX | LOCK_NB) };
    // SAFETY: __error is the thread-local errno pointer (libSystem).
    let errno = unsafe { *__error() };
    // SAFETY: fd is a valid open file descriptor; closed once here.
    unsafe { close(fd) };
    if ret == 0 {
        // Lock was free -> no live fixture -> reject.
        return Err(FixtureError::Unregistered);
    }
    // Only contention (EWOULDBLOCK/EAGAIN) means a live fixture holds the lock.
    if errno == K_ERRNO_EWOULDBLOCK {
        Ok(())
    } else {
        Err(FixtureError::Unregistered)
    }
}

/// Minimal parser for the harness-written registration JSON
/// (`{"pid":<u32>,"token":"<hex>"}`). No dependencies.
fn parse_registration(body: &str) -> Option<(u32, String)> {
    let pid_start = body.find("\"pid\":")? + "\"pid\":".len();
    let pid_end = body[pid_start..].find(|c: char| !c.is_ascii_digit())? + pid_start;
    let pid: u32 = body[pid_start..pid_end].parse().ok()?;
    let token_start = body.find("\"token\":\"")? + "\"token\":\"".len();
    let token_end = body[token_start..].find('"')? + token_start;
    let token = body[token_start..token_end].to_string();
    if token.is_empty() {
        return None;
    }
    Some((pid, token))
}
