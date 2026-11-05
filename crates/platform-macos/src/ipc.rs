//! Read-only, versioned local IPC for native status-bar / extension consumers.
//!
//! Version 1 deliberately implements only `status.get`. The server has no path
//! to an action callback, AX handle, or AppKit object; requests are handled on
//! a worker thread and receive a small snapshot from the shell's Rust state.
//! Wire format is one bounded UTF-8 JSON object per newline-terminated request.

use std::fs::{self, DirBuilder};
use std::io::{self, BufRead, BufReader, Write};
use std::os::fd::AsRawFd;
use std::os::raw::{c_int, c_uint};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const PROTOCOL_VERSION: u64 = 1;
const MAX_REQUEST_BYTES: usize = 4096;
const MAX_REQUEST_ID_BYTES: usize = 64;
const SOCKET_NAME: &str = "ipc.sock";

// `getpeereid(3)` is the documented local-socket peer-credential API on the
// BSD-derived macOS socket layer (Apple man page:
// https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man3/getpeereid.3.html).
// The crate is macOS-only; keep this narrow platform boundary here rather
// than adding a general-purpose libc crate.
extern "C" {
    fn getpeereid(socket: c_int, effective_uid: *mut c_uint, effective_gid: *mut c_uint) -> c_int;
    fn geteuid() -> c_uint;
}

/// The intentionally low-sensitivity state exposed to local read-only clients.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StatusSnapshot {
    pub layout: &'static str,
    pub paused: bool,
    pub auto_tiling: bool,
    pub include_dialogs: bool,
}

struct IpcServer {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

static SERVER: OnceLock<Mutex<Option<IpcServer>>> = OnceLock::new();

fn server_slot() -> &'static Mutex<Option<IpcServer>> {
    SERVER.get_or_init(|| Mutex::new(None))
}

/// Start the owner-only Unix-domain socket. Failure is non-fatal to the UI.
pub(crate) fn start() -> Result<(), String> {
    let mut slot = server_slot().lock().unwrap();
    if slot.is_some() {
        return Ok(());
    }

    let socket_path = socket_path().map_err(|error| error.to_string())?;
    remove_stale_socket(&socket_path).map_err(|error| error.to_string())?;
    let listener = UnixListener::bind(&socket_path).map_err(|error| error.to_string())?;
    if let Err(error) = fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600)) {
        let _ = fs::remove_file(&socket_path);
        return Err(error.to_string());
    }
    let metadata = fs::symlink_metadata(&socket_path).map_err(|error| error.to_string())?;
    let parent = socket_path
        .parent()
        .ok_or_else(|| "socket parent is unavailable".to_string())?;
    let parent_uid = fs::metadata(parent)
        .map_err(|error| error.to_string())?
        .uid();
    if !metadata.file_type().is_socket()
        || metadata.mode() & 0o077 != 0
        || metadata.uid() != parent_uid
    {
        let _ = fs::remove_file(&socket_path);
        return Err("socket failed owner-only validation".to_string());
    }
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;

    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    let worker_path = socket_path.clone();
    let identity = (metadata.dev(), metadata.ino());
    let worker = thread::Builder::new()
        .name("hypr-rust-ipc-v1".to_string())
        .spawn(move || run_server(listener, worker_path, identity, worker_stop))
        .map_err(|error| {
            let _ = fs::remove_file(&socket_path);
            error.to_string()
        })?;
    *slot = Some(IpcServer {
        stop,
        worker: Some(worker),
    });
    Ok(())
}

/// Stop the listener and remove only the socket inode this process created.
pub(crate) fn shutdown() {
    let server = server_slot().lock().unwrap().take();
    drop(server);
}

fn socket_path() -> io::Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .ok_or_else(|| io::Error::new(io::ErrorKind::PermissionDenied, "HOME is unavailable"))?;
    let home = PathBuf::from(home).canonicalize()?;
    let home_meta = fs::metadata(&home)?;
    if !home_meta.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "HOME is not a directory",
        ));
    }

    let directory = home.join(".hypr-rust");
    match fs::symlink_metadata(&directory) {
        Ok(metadata) => validate_state_directory(&directory, &metadata, home_meta.uid())?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            match DirBuilder::new().mode(0o700).create(&directory) {
                Ok(()) => {}
                Err(create_error) if create_error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(create_error) => return Err(create_error),
            }
            let metadata = fs::symlink_metadata(&directory)?;
            validate_state_directory(&directory, &metadata, home_meta.uid())?;
        }
        Err(error) => return Err(error),
    }
    Ok(directory.join(SOCKET_NAME))
}

fn validate_state_directory(path: &Path, metadata: &fs::Metadata, home_uid: u32) -> io::Result<()> {
    if !metadata.file_type().is_dir() || metadata.uid() != home_uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "state directory is not a real directory owned by the home-directory owner",
        ));
    }
    if metadata.mode() & 0o077 != 0 {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    let verified = fs::symlink_metadata(path)?;
    if !verified.file_type().is_dir() || verified.uid() != home_uid || verified.mode() & 0o077 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "state directory could not be made owner-only",
        ));
    }
    Ok(())
}

fn remove_stale_socket(path: &Path) -> io::Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let parent_uid = fs::metadata(path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "socket parent is unavailable")
    })?)?
    .uid();
    if !metadata.file_type().is_socket()
        || metadata.uid() != parent_uid
        || metadata.mode() & 0o077 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "existing IPC path is not a private owner-owned socket; refusing to replace it",
        ));
    }

    match UnixStream::connect(path) {
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            "another Hypr-Rust IPC server is already listening",
        )),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
            ) =>
        {
            // Recheck the inode immediately before unlinking. Never remove a
            // path whose type/owner changed after the connect probe.
            let current = fs::symlink_metadata(path)?;
            if current.file_type().is_socket()
                && current.uid() == metadata.uid()
                && current.dev() == metadata.dev()
                && current.ino() == metadata.ino()
            {
                fs::remove_file(path)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "IPC path changed during stale-socket check",
                ))
            }
        }
        Err(error) => Err(error),
    }
}

fn run_server(
    listener: UnixListener,
    socket_path: PathBuf,
    socket_identity: (u64, u64),
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _address)) => serve_one(stream),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => {
                crate::shell::append_log(&format!("ipc: listener stopped: {error}"));
                break;
            }
        }
    }

    if let Ok(metadata) = fs::symlink_metadata(&socket_path) {
        if metadata.file_type().is_socket() && (metadata.dev(), metadata.ino()) == socket_identity {
            let _ = fs::remove_file(socket_path);
        }
    }
}

fn serve_one(stream: UnixStream) {
    if !peer_is_current_user(&stream) {
        return;
    }
    if stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .is_err()
        || stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .is_err()
    {
        return;
    }
    let mut reader = BufReader::new(stream);
    let line = read_bounded_line(&mut reader);
    let response = match line {
        Ok(Some(line)) => process_request(&line, crate::shell::ipc_status_snapshot()),
        Ok(None) => error_response(None, "bad_request", "empty request"),
        Err(ReadLineError::TooLarge) => {
            error_response(None, "message_too_large", "request exceeds 4096 bytes")
        }
        Err(ReadLineError::Io) => error_response(None, "bad_request", "request could not be read"),
    };
    let _ = reader.get_mut().write_all(response.as_bytes());
}

#[derive(Debug, PartialEq, Eq)]
enum ReadLineError {
    TooLarge,
    Io,
}

fn peer_is_current_user(stream: &UnixStream) -> bool {
    let mut peer_uid: c_uint = 0;
    let mut peer_gid: c_uint = 0;
    // SAFETY: the descriptor is a live connected UnixStream, and both output
    // pointers refer to initialized writable c_uint values for the duration
    // of the documented getpeereid(2) call.
    let result = unsafe { getpeereid(stream.as_raw_fd(), &mut peer_uid, &mut peer_gid) };
    if result != 0 {
        return false;
    }
    // SAFETY: geteuid has no preconditions and returns this process' effective
    // uid; the comparison makes a failed credential lookup deny the peer.
    let current_uid = unsafe { geteuid() };
    peer_uid == current_uid
}

fn read_bounded_line(reader: &mut BufReader<UnixStream>) -> Result<Option<Vec<u8>>, ReadLineError> {
    let mut line = Vec::with_capacity(256);
    loop {
        let available = reader.fill_buf().map_err(|_| ReadLineError::Io)?;
        if available.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Err(ReadLineError::Io) // v1 requires a newline terminator
            };
        }
        let until_newline = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| index + 1);
        let count = until_newline.unwrap_or(available.len());
        if line.len() + count > MAX_REQUEST_BYTES {
            return Err(ReadLineError::TooLarge);
        }
        line.extend_from_slice(&available[..count]);
        reader.consume(count);
        if until_newline.is_some() {
            return Ok(Some(line));
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Request {
    version: u64,
    id: String,
    method: String,
}

#[derive(Debug, PartialEq, Eq)]
enum RequestParseError {
    Malformed,
}

fn process_request(line: &[u8], status: StatusSnapshot) -> String {
    if line.len() > MAX_REQUEST_BYTES {
        return error_response(None, "message_too_large", "request exceeds 4096 bytes");
    }
    let request = match parse_request(line) {
        Ok(request) => request,
        Err(_) => return error_response(None, "bad_request", "invalid request"),
    };
    if request.version != PROTOCOL_VERSION {
        return error_response(
            Some(&request.id),
            "unsupported_version",
            "supported protocol version is 1",
        );
    }
    if request.method != "status.get" {
        return error_response(
            Some(&request.id),
            "method_not_found",
            "method is not available in protocol version 1",
        );
    }
    success_response(&request.id, status)
}

/// Parse the deliberately narrow v1 request object. Strings are unescaped
/// printable ASCII; IDs are limited separately. This keeps the parser small,
/// deterministic, dependency-free, and fail-closed rather than accepting a
/// partially parsed general JSON document.
fn parse_request(line: &[u8]) -> Result<Request, RequestParseError> {
    if line.len() > MAX_REQUEST_BYTES || !line.ends_with(b"\n") {
        return Err(RequestParseError::Malformed);
    }
    let mut parser = JsonCursor {
        bytes: &line[..line.len() - 1],
        offset: 0,
    };
    parser.ws();
    parser.byte(b'{')?;
    let mut version = None;
    let mut id = None;
    let mut method = None;
    let mut params_seen = false;
    parser.ws();
    if parser.peek() == Some(b'}') {
        return Err(RequestParseError::Malformed);
    }
    loop {
        parser.ws();
        let key = parser.string()?;
        parser.ws();
        parser.byte(b':')?;
        parser.ws();
        match key.as_str() {
            "version" if version.is_none() => version = Some(parser.unsigned_integer()?),
            "id" if id.is_none() => {
                let value = parser.string()?;
                if value.is_empty()
                    || value.len() > MAX_REQUEST_ID_BYTES
                    || !value
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
                {
                    return Err(RequestParseError::Malformed);
                }
                id = Some(value);
            }
            "method" if method.is_none() => method = Some(parser.string()?),
            "params" if !params_seen => {
                parser.byte(b'{')?;
                parser.ws();
                parser.byte(b'}')?;
                params_seen = true;
            }
            _ => return Err(RequestParseError::Malformed),
        }
        parser.ws();
        match parser.peek() {
            Some(b',') => {
                parser.offset += 1;
            }
            Some(b'}') => {
                parser.offset += 1;
                break;
            }
            _ => return Err(RequestParseError::Malformed),
        }
    }
    parser.ws();
    if parser.offset != parser.bytes.len() || !params_seen {
        return Err(RequestParseError::Malformed);
    }
    Ok(Request {
        version: version.ok_or(RequestParseError::Malformed)?,
        id: id.ok_or(RequestParseError::Malformed)?,
        method: method.ok_or(RequestParseError::Malformed)?,
    })
}

struct JsonCursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl JsonCursor<'_> {
    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r')) {
            self.offset += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.offset).copied()
    }

    fn byte(&mut self, expected: u8) -> Result<(), RequestParseError> {
        if self.peek() == Some(expected) {
            self.offset += 1;
            Ok(())
        } else {
            Err(RequestParseError::Malformed)
        }
    }

    fn string(&mut self) -> Result<String, RequestParseError> {
        self.byte(b'"')?;
        let start = self.offset;
        while let Some(byte) = self.peek() {
            if byte == b'"' {
                let value = &self.bytes[start..self.offset];
                if !value
                    .iter()
                    .all(|byte| (0x20..=0x7e).contains(byte) && *byte != b'\\')
                {
                    return Err(RequestParseError::Malformed);
                }
                self.offset += 1;
                return String::from_utf8(value.to_vec()).map_err(|_| RequestParseError::Malformed);
            }
            if !(0x20..=0x7e).contains(&byte) || byte == b'\\' {
                return Err(RequestParseError::Malformed);
            }
            self.offset += 1;
        }
        Err(RequestParseError::Malformed)
    }

    fn unsigned_integer(&mut self) -> Result<u64, RequestParseError> {
        let start = self.offset;
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.offset += 1;
        }
        let digits = &self.bytes[start..self.offset];
        if digits.is_empty() || (digits.len() > 1 && digits[0] == b'0') {
            return Err(RequestParseError::Malformed);
        }
        std::str::from_utf8(digits)
            .ok()
            .and_then(|text| text.parse().ok())
            .ok_or(RequestParseError::Malformed)
    }
}

fn success_response(id: &str, status: StatusSnapshot) -> String {
    format!(
        "{{\"version\":1,\"id\":\"{id}\",\"ok\":true,\"result\":{{\"layout\":\"{}\",\"paused\":{},\"auto_tiling\":{},\"include_dialogs\":{},\"app_version\":\"{}\"}}}}\n",
        status.layout,
        status.paused,
        status.auto_tiling,
        status.include_dialogs,
        env!("CARGO_PKG_VERSION"),
    )
}

fn error_response(id: Option<&str>, code: &str, message: &str) -> String {
    let id = id
        .map(|id| format!("\"{id}\""))
        .unwrap_or_else(|| "null".to_string());
    format!(
        "{{\"version\":1,\"id\":{id},\"ok\":false,\"error\":{{\"code\":\"{code}\",\"message\":\"{message}\"}}}}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::{parse_request, process_request, RequestParseError, StatusSnapshot};

    fn status() -> StatusSnapshot {
        StatusSnapshot {
            layout: "master-stack",
            paused: true,
            auto_tiling: false,
            include_dialogs: false,
        }
    }

    #[test]
    fn v1_status_query_returns_only_safe_snapshot_fields() {
        let line = br#"{"version":1,"id":"test-01","method":"status.get","params":{}}"#;
        let mut terminated = line.to_vec();
        terminated.push(b'\n');
        let response = process_request(&terminated, status());
        assert!(response.contains("\"id\":\"test-01\""));
        assert!(response.contains("\"layout\":\"master-stack\""));
        assert!(response.contains("\"paused\":true"));
        assert!(response.contains("\"auto_tiling\":false"));
        assert!(response.contains("\"include_dialogs\":false"));
        assert!(response.contains("\"app_version\":"));
        assert!(!response.contains("title"));
        assert!(!response.contains("window"));
    }

    #[test]
    fn request_key_order_and_whitespace_are_not_significant() {
        let line = concat!(
            " { \"params\" : { }, ",
            "\"method\" : \"status.get\", ",
            "\"id\" : \"x_2\", \"version\" : 1 } \n",
        )
        .as_bytes();
        assert_eq!(parse_request(line).unwrap().id, "x_2");
    }

    #[test]
    fn unknown_version_and_method_fail_closed() {
        let wrong_version = br#"{"version":2,"id":"a","method":"status.get","params":{}}"#;
        let mut wrong_version_line = wrong_version.to_vec();
        wrong_version_line.push(b'\n');
        let response = process_request(&wrong_version_line, status());
        assert!(response.contains("unsupported_version"));

        let write = br#"{"version":1,"id":"b","method":"reflow.request","params":{}}"#;
        let mut write_line = write.to_vec();
        write_line.push(b'\n');
        let response = process_request(&write_line, status());
        assert!(response.contains("method_not_found"));
    }

    #[test]
    fn malformed_duplicate_and_nonempty_params_are_rejected() {
        let malformed: &[&[u8]] = &[
            br#"{"version":1,"version":1,"id":"a","method":"status.get","params":{}}"#,
            br#"{"version":1,"id":"a","method":"status.get","params":{"x":1}}"#,
            br#"{"version":1,"id":"bad id","method":"status.get","params":{}}"#,
            br#"{"version":1,"id":"bad\\u0020id","method":"status.get","params":{}}"#,
            br#"{"version":1,"id":"a","method":"status.get","params":{},"extra":true}"#,
        ];
        for line in malformed {
            let mut terminated = line.to_vec();
            terminated.push(b'\n');
            assert_eq!(
                parse_request(&terminated),
                Err(RequestParseError::Malformed)
            );
        }
    }

    #[test]
    fn newline_is_required_and_oversize_is_rejected() {
        let no_newline = br#"{"version":1,"id":"a","method":"status.get","params":{}}"#;
        assert_eq!(parse_request(no_newline), Err(RequestParseError::Malformed));
        let too_large = vec![b'x'; super::MAX_REQUEST_BYTES + 1];
        let response = process_request(&too_large, status());
        assert!(response.contains("message_too_large"));
    }
}
