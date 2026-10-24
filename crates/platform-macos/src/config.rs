//! Versioned, owner-only configuration for the macOS shell.
//!
//! The config is a small flat JSON object at `~/.hypr-rust/config.json`, a
//! regular non-symlink file owned by the current user with perm 0600. It backs
//! the shell until a settings UI exists. `auto_tiling` defaults to `false`:
//! auto-tiling is off until the user explicitly opts in.
//!
//! The parser is deliberately tiny (flat object of numbers, booleans and
//! strings) so the crate keeps no new dependencies.

use std::path::PathBuf;

use layout_core::{LayoutConfig, LayoutKind};

use crate::ffi;

/// Current config schema version.
pub const CONFIG_VERSION: u32 = 1;

/// One value in the flat config object.
#[derive(Debug, Clone, PartialEq)]
enum Value {
    Number(f64),
    Bool(bool),
    Text(String),
}

/// Shell configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub version: u32,
    pub kind: LayoutKind,
    pub layout: LayoutConfig,
    /// Explicit user opt-in to automatic tiling. Off by default.
    pub auto_tiling: bool,
    /// Include dialogs in the plan (ordinary app dialogs are excluded by
    /// default). This is a per-plan policy, never a mandatory exclusion.
    pub include_dialogs: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            kind: LayoutKind::MasterStack,
            layout: LayoutConfig::default(),
            auto_tiling: false,
            include_dialogs: false,
        }
    }
}

/// Config failure. Never carries file contents or other sensitive data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// No config file exists yet; the caller should fall back to defaults.
    Missing,
    /// The file is not a regular owner-only 0600 file.
    NotOwnerOnly,
    /// The body could not be parsed.
    Parse,
    /// A value was outside its valid range.
    Invalid(&'static str),
    /// The file could not be written.
    Write,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => write!(f, "no config file (using defaults)"),
            Self::NotOwnerOnly => write!(f, "config must be a regular owner-only 0600 file"),
            Self::Parse => write!(f, "config is not valid JSON"),
            Self::Invalid(field) => write!(f, "config value out of range: {field}"),
            Self::Write => write!(f, "config could not be written"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Path of the config file: `~/.hypr-rust/config.json`.
pub fn config_path() -> Result<PathBuf, ConfigError> {
    let home = std::env::var("HOME").map_err(|_| ConfigError::NotOwnerOnly)?;
    Ok(PathBuf::from(home).join(".hypr-rust").join("config.json"))
}

impl Config {
    /// Load and validate the config. `Missing` means "use defaults"; the other
    /// errors are surfaced to the user rather than silently ignored.
    pub fn load() -> Result<Config, ConfigError> {
        load_from(&config_path()?)
    }

    /// Load the config, falling back to defaults when the file is absent.
    pub fn load_or_default() -> Config {
        Config::load().unwrap_or_default()
    }

    /// Serialize and write the config atomically (temp file + rename, 0600).
    pub fn save(&self) -> Result<(), ConfigError> {
        save_to(&config_path()?, self)
    }

    /// Render the config as the flat JSON object.
    pub fn to_json(&self) -> String {
        format!(
            "{{\n  \"version\": {},\n  \"layout\": \"{}\",\n  \"outer_gap\": {},\n  \"inner_gap\": {},\n  \"master_ratio\": {},\n  \"auto_tiling\": {},\n  \"include_dialogs\": {}\n}}\n",
            self.version,
            layout_kind_name(self.kind),
            fmt(self.layout.outer_gap),
            fmt(self.layout.inner_gap),
            fmt(self.layout.master_ratio),
            self.auto_tiling,
            self.include_dialogs,
        )
    }

    /// The layout config the shell passes to the planner.
    pub fn plan_config(&self) -> (LayoutKind, LayoutConfig) {
        (self.kind, self.layout)
    }

    /// The configured layout name, for the menu title.
    pub fn kind_name(&self) -> &'static str {
        layout_kind_name(self.kind)
    }
}

/// Load and validate the config at `path`.
fn load_from(path: &std::path::Path) -> Result<Config, ConfigError> {
    let meta = std::fs::symlink_metadata(path).map_err(|_| ConfigError::Missing)?;
    verify_owner_only(&meta)?;
    let body = std::fs::read_to_string(path).map_err(|_| ConfigError::Parse)?;
    parse_config(&body)
}

/// Write the config to `path` atomically (temp file + rename, 0600).
fn save_to(path: &std::path::Path, config: &Config) -> Result<(), ConfigError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|_| ConfigError::Write)?;
    }
    let tmp = path.with_extension("json.tmp");
    {
        use std::io::Write;
        let mut file = std::fs::File::create(&tmp).map_err(|_| ConfigError::Write)?;
        file.write_all(config.to_json().as_bytes())
            .map_err(|_| ConfigError::Write)?;
        file.sync_all().map_err(|_| ConfigError::Write)?;
    }
    set_owner_only(&tmp)?;
    std::fs::rename(&tmp, path).map_err(|_| ConfigError::Write)?;
    Ok(())
}

fn fmt(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// Owner-only check shared with the fixture registration guard.
fn verify_owner_only(meta: &std::fs::Metadata) -> Result<(), ConfigError> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    if meta.file_type().is_symlink() || !meta.file_type().is_file() {
        return Err(ConfigError::NotOwnerOnly);
    }
    if meta.uid() != unsafe { ffi::getuid() } {
        return Err(ConfigError::NotOwnerOnly);
    }
    if meta.permissions().mode() & 0o777 != 0o600 {
        return Err(ConfigError::NotOwnerOnly);
    }
    Ok(())
}

fn set_owner_only(path: &std::path::Path) -> Result<(), ConfigError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|_| ConfigError::Write)
}

fn layout_kind_name(kind: LayoutKind) -> &'static str {
    match kind {
        LayoutKind::Bsp => "bsp",
        LayoutKind::MasterStack => "master-stack",
        LayoutKind::Monocle => "monocle",
        LayoutKind::Columns => "columns",
        LayoutKind::Rows => "rows",
    }
}

fn parse_kind(text: &str) -> Option<LayoutKind> {
    match text {
        "bsp" => Some(LayoutKind::Bsp),
        "master-stack" => Some(LayoutKind::MasterStack),
        "monocle" => Some(LayoutKind::Monocle),
        "columns" => Some(LayoutKind::Columns),
        "rows" => Some(LayoutKind::Rows),
        _ => None,
    }
}

/// Parse the flat config object. Unknown keys are ignored so a newer config can
/// be read by an older build; a known key with the wrong value type is an error.
fn parse_config(body: &str) -> Result<Config, ConfigError> {
    let mut config = Config::default();
    let mut saw_version = false;
    for (key, value) in parse_object(body)? {
        match key.as_str() {
            "version" => {
                let number = as_number(&value, "version")?;
                let version = number as u32;
                if version != CONFIG_VERSION {
                    return Err(ConfigError::Invalid("version"));
                }
                saw_version = true;
            }
            "layout" => {
                let text = as_text(&value, "layout")?;
                config.kind = parse_kind(text).ok_or(ConfigError::Invalid("layout"))?;
            }
            "outer_gap" => {
                config.layout.outer_gap = finite_non_negative(as_number(&value, "outer_gap")?)?;
            }
            "inner_gap" => {
                config.layout.inner_gap = finite_non_negative(as_number(&value, "inner_gap")?)?;
            }
            "master_ratio" => {
                config.layout.master_ratio =
                    finite_in_unit_interval(as_number(&value, "master_ratio")?)?;
            }
            "auto_tiling" => config.auto_tiling = as_bool(&value, "auto_tiling")?,
            "include_dialogs" => config.include_dialogs = as_bool(&value, "include_dialogs")?,
            _ => {}
        }
    }
    if !saw_version {
        return Err(ConfigError::Invalid("version"));
    }
    Ok(config)
}

fn as_number(value: &Value, field: &'static str) -> Result<f64, ConfigError> {
    match value {
        Value::Number(number) => Ok(*number),
        _ => Err(ConfigError::Invalid(field)),
    }
}

fn as_bool(value: &Value, field: &'static str) -> Result<bool, ConfigError> {
    match value {
        Value::Bool(boolean) => Ok(*boolean),
        _ => Err(ConfigError::Invalid(field)),
    }
}

fn as_text<'a>(value: &'a Value, field: &'static str) -> Result<&'a str, ConfigError> {
    match value {
        Value::Text(text) => Ok(text),
        _ => Err(ConfigError::Invalid(field)),
    }
}

fn finite_non_negative(value: f64) -> Result<f64, ConfigError> {
    if value.is_finite() && value >= 0.0 {
        Ok(value)
    } else {
        Err(ConfigError::Invalid("gap"))
    }
}

fn finite_in_unit_interval(value: f64) -> Result<f64, ConfigError> {
    if value.is_finite() && (0.05..=0.95).contains(&value) {
        Ok(value)
    } else {
        Err(ConfigError::Invalid("master_ratio"))
    }
}

/// Parse a flat JSON object: `{ "key": value, ... }` where a value is a number,
/// a boolean or a string. No nesting, no escapes.
fn parse_object(body: &str) -> Result<Vec<(String, Value)>, ConfigError> {
    let bytes = body.as_bytes();
    let mut pos = 0usize;
    let mut items = Vec::new();
    skip_ws(bytes, &mut pos);
    expect(bytes, &mut pos, b'{')?;
    skip_ws(bytes, &mut pos);
    if peek(bytes, pos) == Some(b'}') {
        return Ok(items);
    }
    loop {
        skip_ws(bytes, &mut pos);
        let key = parse_string(bytes, &mut pos)?;
        skip_ws(bytes, &mut pos);
        expect(bytes, &mut pos, b':')?;
        skip_ws(bytes, &mut pos);
        let value = parse_value(bytes, &mut pos)?;
        items.push((key, value));
        skip_ws(bytes, &mut pos);
        match next(bytes, &mut pos)? {
            b',' => continue,
            b'}' => return Ok(items),
            _ => return Err(ConfigError::Parse),
        }
    }
}

fn parse_value(bytes: &[u8], pos: &mut usize) -> Result<Value, ConfigError> {
    match peek(bytes, *pos) {
        Some(b'"') => Ok(Value::Text(parse_string(bytes, pos)?)),
        Some(b't') => {
            expect_word(bytes, pos, b"true")?;
            Ok(Value::Bool(true))
        }
        Some(b'f') => {
            expect_word(bytes, pos, b"false")?;
            Ok(Value::Bool(false))
        }
        Some(_) => Ok(Value::Number(parse_number(bytes, pos)?)),
        None => Err(ConfigError::Parse),
    }
}

fn parse_string(bytes: &[u8], pos: &mut usize) -> Result<String, ConfigError> {
    expect(bytes, pos, b'"')?;
    let start = *pos;
    while *pos < bytes.len() && bytes[*pos] != b'"' {
        *pos += 1;
    }
    if *pos >= bytes.len() {
        return Err(ConfigError::Parse);
    }
    let text = std::str::from_utf8(&bytes[start..*pos]).map_err(|_| ConfigError::Parse)?;
    *pos += 1;
    Ok(text.to_string())
}

fn parse_number(bytes: &[u8], pos: &mut usize) -> Result<f64, ConfigError> {
    let start = *pos;
    while *pos < bytes.len()
        && (bytes[*pos].is_ascii_digit() || matches!(bytes[*pos], b'-' | b'+' | b'.' | b'e' | b'E'))
    {
        *pos += 1;
    }
    let text = std::str::from_utf8(&bytes[start..*pos]).map_err(|_| ConfigError::Parse)?;
    text.parse::<f64>().map_err(|_| ConfigError::Parse)
}

fn skip_ws(bytes: &[u8], pos: &mut usize) {
    while *pos < bytes.len() && bytes[*pos].is_ascii_whitespace() {
        *pos += 1;
    }
}

fn peek(bytes: &[u8], pos: usize) -> Option<u8> {
    bytes.get(pos).copied()
}

fn next(bytes: &[u8], pos: &mut usize) -> Result<u8, ConfigError> {
    if *pos >= bytes.len() {
        return Err(ConfigError::Parse);
    }
    let byte = bytes[*pos];
    *pos += 1;
    Ok(byte)
}

fn expect(bytes: &[u8], pos: &mut usize, byte: u8) -> Result<(), ConfigError> {
    if peek(bytes, *pos) == Some(byte) {
        *pos += 1;
        Ok(())
    } else {
        Err(ConfigError::Parse)
    }
}

fn expect_word(bytes: &[u8], pos: &mut usize, word: &[u8]) -> Result<(), ConfigError> {
    if bytes.len() >= *pos + word.len() && &bytes[*pos..*pos + word.len()] == word {
        *pos += word.len();
        Ok(())
    } else {
        Err(ConfigError::Parse)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid_and_disables_auto_tiling() {
        let config = Config::default();
        assert_eq!(config.version, CONFIG_VERSION);
        assert_eq!(config.kind, LayoutKind::MasterStack);
        assert!(!config.auto_tiling);
        assert!(!config.include_dialogs);
    }

    #[test]
    fn round_trips_through_json() {
        let config = Config::default();
        let parsed = parse_config(&config.to_json()).unwrap();
        assert_eq!(parsed, config);
    }

    #[test]
    fn parses_a_custom_config() {
        let body = r#"{
            "version": 1,
            "layout": "columns",
            "outer_gap": 20,
            "inner_gap": 4,
            "master_ratio": 0.6,
            "auto_tiling": true,
            "include_dialogs": true
        }"#;
        let config = parse_config(body).unwrap();
        assert_eq!(config.kind, LayoutKind::Columns);
        assert_eq!(config.layout.outer_gap, 20.0);
        assert_eq!(config.layout.inner_gap, 4.0);
        assert_eq!(config.layout.master_ratio, 0.6);
        assert!(config.auto_tiling);
        assert!(config.include_dialogs);
    }

    #[test]
    fn rejects_bad_version_and_values() {
        assert_eq!(
            parse_config(r#"{"version": 2}"#),
            Err(ConfigError::Invalid("version"))
        );
        assert_eq!(
            parse_config(r#"{"version": 1, "layout": "hex"}"#),
            Err(ConfigError::Invalid("layout"))
        );
        assert_eq!(
            parse_config(r#"{"version": 1, "master_ratio": 5}"#),
            Err(ConfigError::Invalid("master_ratio"))
        );
        assert_eq!(
            parse_config(r#"{"version": 1, "outer_gap": -1}"#),
            Err(ConfigError::Invalid("gap"))
        );
        assert_eq!(parse_config("{"), Err(ConfigError::Parse));
        assert_eq!(parse_config(r#"{"version": 1"#), Err(ConfigError::Parse));
        assert_eq!(
            parse_config(r#"{"version": 1, "auto_tiling": "yes"}"#),
            Err(ConfigError::Invalid("auto_tiling"))
        );
        assert_eq!(
            parse_config(r#"{"version": 1, "outer_gap": true}"#),
            Err(ConfigError::Invalid("outer_gap"))
        );
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let config =
            parse_config(r#"{"version": 1, "future_field": 42, "layout": "rows"}"#).unwrap();
        assert_eq!(config.kind, LayoutKind::Rows);
    }

    #[test]
    fn save_then_load_round_trips_and_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir();
        let path = dir.join(format!("hypr-config-{}.json", std::process::id()));
        let config = Config {
            kind: LayoutKind::Columns,
            layout: LayoutConfig {
                outer_gap: 20.0,
                inner_gap: 4.0,
                master_ratio: 0.6,
            },
            auto_tiling: true,
            include_dialogs: true,
            ..Config::default()
        };
        save_to(&path, &config).unwrap();

        // The file is owner-only, regular and non-symlink.
        let meta = std::fs::symlink_metadata(&path).unwrap();
        assert!(!meta.file_type().is_symlink());
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        // It loads back identically.
        assert_eq!(load_from(&path).unwrap(), config);
        // A world-readable config is refused.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(load_from(&path), Err(ConfigError::NotOwnerOnly));
        std::fs::remove_file(&path).unwrap();
        let _ = std::fs::remove_file(path.with_extension("json.tmp"));
    }
}
