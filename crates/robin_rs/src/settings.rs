//! Persistent key-value config store.
//!
//! The game stores options in `.cfg` files using a simple `key = value`
//! text format, one entry per line. A `BTreeMap` keeps entries in sorted
//! iteration order.
//!
//! Legacy compatibility surface: the live application persists typed player
//! profiles and key configurations instead. Retain the original numeric and
//! boolean parsing here for callers importing original-game configuration.
//! New persistence code should use the typed stores, not this string map.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;

pub struct Settings {
    map: BTreeMap<String, String>,
    initialized: bool,
    cached_write: bool,
    resource: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self::new()
    }
}

impl Settings {
    pub fn new() -> Self {
        Settings {
            map: BTreeMap::new(),
            initialized: false,
            cached_write: false,
            resource: String::new(),
        }
    }

    pub fn open(&mut self, repository: &str) -> bool {
        self.try_open(repository)
            .map_err(|error| {
                tracing::warn!(repository, %error, "could not open legacy settings");
            })
            .is_ok()
    }

    pub fn try_open(&mut self, repository: &str) -> std::io::Result<()> {
        let resource = format!("{}.cfg", repository);
        if self.initialized && resource == self.resource {
            self.try_close()?;
        }
        let bytes = match fs::read(&resource) {
            Ok(b) => b,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error),
        };
        // Preserve an existing dirty store if either read or flush fails.
        self.try_close()?;
        self.resource = resource;
        self.initialized = true;

        for line in bytes.split(|&b| b == b'\n') {
            if let Some(eq_pos) = line.iter().position(|&b| b == b'=') {
                let key = trim_field(&line[..eq_pos]);
                let value = trim_field(&line[eq_pos + 1..]);
                if !key.is_empty() {
                    let key_s = String::from_utf8_lossy(key).into_owned();
                    let val_s = String::from_utf8_lossy(value).into_owned();
                    // std::map::insert doesn't overwrite — first key wins
                    self.map.entry(key_s).or_insert(val_s);
                }
            }
        }

        Ok(())
    }

    fn write_map_to_file(&self) -> std::io::Result<()> {
        let max_key_len = self.map.keys().map(|k| k.len()).max().unwrap_or(0);
        let path = std::path::Path::new(&self.resource);
        let directory = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."));
        let mut file = tempfile::NamedTempFile::new_in(directory)?;

        for (key, value) in &self.map {
            let padding = max_key_len.saturating_sub(key.len());
            writeln!(file, "{}{} = {}", key, " ".repeat(padding), value)?;
        }
        file.as_file().sync_all()?;
        file.persist(path).map_err(|error| error.error)?;
        Ok(())
    }

    pub fn close(&mut self) -> bool {
        self.try_close()
            .map_err(|error| {
                tracing::warn!(path = %self.resource, %error, "could not flush legacy settings");
            })
            .is_ok()
    }

    pub fn try_close(&mut self) -> std::io::Result<()> {
        if self.cached_write {
            self.write_map_to_file()?;
        }
        self.cached_write = false;
        self.initialized = false;
        self.map.clear();
        Ok(())
    }

    pub fn read_long(&self, key: &str) -> Option<i32> {
        if !self.initialized {
            return None;
        }
        let v = self.map.get(key)?;
        Some(parse_atoi(v))
    }

    pub fn read_string(&self, key: &str) -> Option<&str> {
        if !self.initialized {
            return None;
        }
        self.map.get(key).map(|s| s.as_str())
    }

    pub fn read_float(&self, key: &str) -> Option<f32> {
        if !self.initialized {
            return None;
        }
        let v = self.map.get(key)?;
        Some(parse_atof(v))
    }

    pub fn read_bool(&self, key: &str) -> Option<bool> {
        if !self.initialized {
            return None;
        }
        let v = self.map.get(key)?;
        // Original-game bug: the string comparisons ("true"/"false") are unreachable;
        // the final `atoi(str) > 0` always overwrites. We replicate that.
        Some(parse_atoi(v) > 0)
    }

    pub fn write_long(&mut self, key: &str, value: i32) -> bool {
        if !self.initialized {
            return false;
        }
        // Match sprintf("%li", value)
        self.map.insert(key.to_owned(), format!("{}", value));
        self.cached_write = true;
        true
    }

    pub fn write_string(&mut self, key: &str, value: &str) -> bool {
        if !self.initialized {
            return false;
        }
        self.map.insert(key.to_owned(), value.to_owned());
        self.cached_write = true;
        true
    }

    pub fn write_float(&mut self, key: &str, value: f32) -> bool {
        if !self.initialized {
            return false;
        }
        // Match sprintf("%f", value) — default 6 decimal places.
        // C promotes float to double in varargs; cast to f64 for same output.
        self.map
            .insert(key.to_owned(), format!("{:.6}", value as f64));
        self.cached_write = true;
        true
    }

    pub fn write_bool(&mut self, key: &str, value: bool) -> bool {
        if !self.initialized {
            return false;
        }
        self.map.insert(
            key.to_owned(),
            if value { "true" } else { "false" }.to_owned(),
        );
        self.cached_write = true;
        true
    }
}

impl Drop for Settings {
    fn drop(&mut self) {
        // Flush on destroy if dirty
        if self.initialized && self.cached_write {
            self.close();
        }
    }
}

/// Trim leading/trailing whitespace and trailing `=` from a config field.
/// Also strips `\r` for files with Windows line endings.
fn trim_field(s: &[u8]) -> &[u8] {
    let start = s
        .iter()
        .position(|&b| b != b' ' && b != b'\t')
        .unwrap_or(s.len());
    let end = s
        .iter()
        .rposition(|&b| b != b' ' && b != b'\t' && b != b'=' && b != b'\r')
        .map_or(start, |p| p + 1);
    if end > start { &s[start..end] } else { &[] }
}

/// Match C `atoi()` semantics: skip whitespace, parse optional sign + digits.
fn parse_atoi(s: &str) -> i32 {
    let s = s.trim_start();
    let (s, neg) = if let Some(rest) = s.strip_prefix('-') {
        (rest, true)
    } else if let Some(rest) = s.strip_prefix('+') {
        (rest, false)
    } else {
        (s, false)
    };
    let mut result: i32 = 0;
    for b in s.bytes() {
        if b.is_ascii_digit() {
            result = result.wrapping_mul(10).wrapping_add((b - b'0') as i32);
        } else {
            break;
        }
    }
    if neg { result.wrapping_neg() } else { result }
}

/// Match C `atof()` semantics for well-formed float strings.
fn parse_atof(s: &str) -> f32 {
    s.trim().parse::<f64>().unwrap_or(0.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_field_strips_whitespace() {
        assert_eq!(trim_field(b"  hello  "), b"hello");
        assert_eq!(trim_field(b"\thello\t"), b"hello");
        assert_eq!(trim_field(b"hello"), b"hello");
        assert_eq!(trim_field(b"  "), &[] as &[u8]);
    }

    #[test]
    fn trim_field_strips_trailing_equals() {
        assert_eq!(trim_field(b"key ="), b"key");
        assert_eq!(trim_field(b"key  = "), b"key");
    }

    #[test]
    fn trim_field_strips_cr() {
        assert_eq!(trim_field(b"value\r"), b"value");
    }

    #[test]
    fn parse_atoi_basic() {
        assert_eq!(parse_atoi("42"), 42);
        assert_eq!(parse_atoi("-7"), -7);
        assert_eq!(parse_atoi("+3"), 3);
        assert_eq!(parse_atoi("  123"), 123);
        assert_eq!(parse_atoi("abc"), 0);
        assert_eq!(parse_atoi(""), 0);
    }

    #[test]
    fn parse_atof_basic() {
        assert!((parse_atof("1.5") - 1.5).abs() < f32::EPSILON);
        assert!((parse_atof("abc") - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn open_missing_file_succeeds() {
        let mut s = Settings::new();
        let directory = tempfile::tempdir().unwrap();
        assert!(s.open(directory.path().join("missing").to_str().unwrap()));
        assert!(s.initialized);
        assert_eq!(s.map.len(), 0);
    }

    #[test]
    fn roundtrip_values() {
        let mut s = Settings::new();
        s.initialized = true;

        assert!(s.write_long("count", 42));
        assert!(s.write_string("name", "Robin"));
        assert!(s.write_float("volume", 0.75));
        assert!(s.write_bool("fullscreen", true));

        assert_eq!(s.read_long("count"), Some(42));
        assert_eq!(s.read_string("name"), Some("Robin"));
        assert!((s.read_float("volume").unwrap() - 0.75).abs() < 0.001);
        // Note: read_bool matches original atoi("true") > 0 = false (bug preserved)
        assert_eq!(s.read_bool("fullscreen"), Some(false));
    }

    #[test]
    fn read_uninitialized_returns_none() {
        let s = Settings::new();
        assert_eq!(s.read_long("key"), None);
        assert_eq!(s.read_string("key"), None);
        assert_eq!(s.read_float("key"), None);
        assert_eq!(s.read_bool("key"), None);
    }

    #[test]
    fn read_missing_key_returns_none() {
        let mut s = Settings::new();
        s.initialized = true;
        assert_eq!(s.read_long("nope"), None);
    }

    #[test]
    fn write_uninitialized_returns_false() {
        let mut s = Settings::new();
        assert!(!s.write_long("key", 1));
    }

    #[test]
    fn open_parse_and_read() {
        let directory = tempfile::tempdir().unwrap();
        let dir = directory.path();
        let cfg_path = dir.join("test.cfg");
        {
            let mut f = fs::File::create(&cfg_path).unwrap();
            write!(f, "volume = 80\nname   = Tuck\n").unwrap();
        }
        let mut s = Settings::new();
        let repo = dir.join("test");
        assert!(s.open(repo.to_str().unwrap()));
        assert_eq!(s.read_long("volume"), Some(80));
        assert_eq!(s.read_string("name"), Some("Tuck"));
    }

    #[test]
    fn close_writes_file() {
        let directory = tempfile::tempdir().unwrap();
        let dir = directory.path();
        let repo = dir.join("out");
        let cfg_path = dir.join("out.cfg");

        let mut s = Settings::new();
        assert!(s.open(repo.to_str().unwrap()));
        s.write_long("a_key", 10);
        s.write_string("b_key", "hello");
        assert!(s.close());

        let contents = fs::read_to_string(&cfg_path).unwrap();
        assert!(contents.contains("a_key = 10"));
        assert!(contents.contains("b_key = hello"));
    }

    #[test]
    fn first_key_wins_on_duplicate() {
        let directory = tempfile::tempdir().unwrap();
        let dir = directory.path();
        let cfg_path = dir.join("dup.cfg");
        {
            let mut f = fs::File::create(&cfg_path).unwrap();
            write!(f, "key = first\nkey = second\n").unwrap();
        }
        let mut s = Settings::new();
        assert!(s.open(dir.join("dup").to_str().unwrap()));
        assert_eq!(s.read_string("key"), Some("first"));
    }

    #[test]
    fn unreadable_input_is_not_an_empty_success() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("bad.cfg")).unwrap();
        let mut settings = Settings::new();
        assert!(
            settings
                .try_open(dir.path().join("bad").to_str().unwrap())
                .is_err()
        );
        assert!(!settings.initialized);
    }

    #[test]
    fn failed_replace_retains_dirty_state_and_can_retry() {
        let dir = tempfile::tempdir().unwrap();
        let mut settings = Settings::new();
        settings
            .try_open(dir.path().join("retry").to_str().unwrap())
            .unwrap();
        settings.write_long("count", 7);
        fs::create_dir(dir.path().join("retry.cfg")).unwrap();
        assert!(settings.try_close().is_err());
        assert!(settings.cached_write);
        assert_eq!(settings.read_long("count"), Some(7));
        fs::remove_dir(dir.path().join("retry.cfg")).unwrap();
        settings.try_close().unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("retry.cfg")).unwrap(),
            "count = 7\n"
        );
    }

    #[test]
    fn close_without_changes_releases_open_state() {
        let dir = tempfile::tempdir().unwrap();
        let mut settings = Settings::new();
        settings
            .try_open(dir.path().join("empty").to_str().unwrap())
            .unwrap();
        settings.try_close().unwrap();
        assert!(!settings.initialized);
        assert!(!dir.path().join("empty.cfg").exists());
    }
}
