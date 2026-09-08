//! CI guardrail for the "method-lvalue footgun" introduced by the
//! PI-into-Sprite refactor.
//!
//! Background: getters like `ElementData::position()` return `Point3D`
//! by value.  Writing `elem.position().x = v` (or `+= v`) compiles
//! cleanly but is a silent no-op — the assignment lands on a temporary
//! that is dropped at the end of the expression.  A mechanical script
//! introduced ~15 instances of this pattern during the refactor and
//! they were only caught by integration tests ("arrow never moves").
//!
//! We tried `#[must_use]` on the getters first; it does NOT catch the
//! `.field = v` form (Rust treats the field projection as "the value
//! was used"), only the bare-statement `f.position();` form.  The
//! getters are still annotated as defense-in-depth, but this scanner
//! is the actual guardrail for the dangerous pattern.
//!
//! If this test fires on a legitimate construct, either widen the
//! `EXEMPT_PATTERNS` list or rework the call site (typically: take an
//! `&mut PositionInterface` and call `set_position` / `set_direction`
//! / etc., or read the value into a local, mutate the local, write it
//! back via the setter).

use std::fs;
use std::path::{Path, PathBuf};

/// Methods known to return a value type whose fields are mutable
/// place expressions — i.e. exactly the methods where
/// `<expr>.<method>(...).field = v` silently modifies a temporary.
///
/// Keep in sync with the `#[must_use]` annotations in `element.rs`,
/// `position_interface.rs`, and `sprite.rs`.
const DANGEROUS_METHODS: &[&str] = &[
    // ElementData / Element trait forwarders
    "position",
    "position_map",
    // PositionInterface getters
    "get_position",
    "get_position_map",
    "get_position_sprite",
    "get_old_position",
    "get_increment",
    "get_increment_map",
    "get_position_goal",
    "get_half_diagonal",
    // Sprite frame getters
    "current_offset",
    "hotspot_for_row",
    "current_hotspot",
];

/// Field projections that, applied to a temporary returned by one of
/// the above methods, would produce the silent-no-op assignment.
const FIELDS: &[&str] = &["x", "y", "z"];

/// True if `idx` falls inside a `"..."` string literal on this line.
/// Counts unescaped double-quotes before `idx`; odd → inside a string.
/// Cheap and fooled by quotes inside `//` comments, but the caller
/// already filters those.
fn is_inside_string_literal(line: &str, idx: usize) -> bool {
    let bytes = line.as_bytes();
    let mut in_str = false;
    let mut k = 0;
    while k < idx && k < bytes.len() {
        if bytes[k] == b'"' {
            // Count preceding unescaped backslashes.
            let mut bs = 0;
            let mut m = k;
            while m > 0 && bytes[m - 1] == b'\\' {
                bs += 1;
                m -= 1;
            }
            if bs % 2 == 0 {
                in_str = !in_str;
            }
        }
        k += 1;
    }
    in_str
}

/// Assignment operators we care about: `=`, `+=`, `-=`, `*=`, `/=`,
/// `%=`, `&=`, `|=`, `^=`, `<<=`, `>>=`.  We match on `<op>=` with `=`
/// not preceded by `=`/`!`/`<`/`>` to avoid `==`, `!=`, `<=`, `>=`.
fn is_assignment_op_at(line: &str, eq_idx: usize) -> bool {
    if line.as_bytes().get(eq_idx) != Some(&b'=') {
        return false;
    }
    // Skip `==`, `!=`, `<=`, `>=`.
    if let Some(prev) = eq_idx.checked_sub(1).and_then(|i| line.as_bytes().get(i))
        && matches!(prev, b'=' | b'!' | b'<' | b'>')
    {
        return false;
    }
    // Skip `=>` and `=` followed by `=` (guards the `==` case where
    // we are at the first `=`).
    line.as_bytes().get(eq_idx + 1) != Some(&b'=')
}

/// True if the byte slice immediately preceding `at` matches
/// `.<method>(...).<field>` for some `method` in `DANGEROUS_METHODS`
/// and `field` in `FIELDS`, with optional compound-assignment prefix
/// and arbitrary whitespace before `=`.
fn line_matches_footgun(line: &str, eq_idx: usize) -> Option<(&'static str, &'static str)> {
    let bytes = line.as_bytes();
    let mut i = eq_idx;
    // Step back over a compound-assignment operator character
    // (`+=`, `-=`, `*=`, `/=`, `%=`, `&=`, `|=`, `^=`).
    if i > 0
        && matches!(
            bytes[i - 1],
            b'+' | b'-' | b'*' | b'/' | b'%' | b'&' | b'|' | b'^'
        )
    {
        i -= 1;
    }
    // Walk back over whitespace.
    while i > 0 && bytes[i - 1].is_ascii_whitespace() {
        i -= 1;
    }
    // Now `line[..i]` should end in `.<field>` for one of FIELDS.
    for &field in FIELDS {
        let needle = format!(".{field}");
        if i < needle.len() {
            continue;
        }
        if &bytes[i - needle.len()..i] != needle.as_bytes() {
            continue;
        }
        let before_dot = i - needle.len();
        if before_dot == 0 {
            continue;
        }
        // Char immediately before the `.field` must be `)` — i.e.
        // `<...method>(...).field`.  An ident char would mean the
        // `.x` is a struct-literal field write or a local-variable
        // projection (both safe).
        if bytes[before_dot - 1] != b')' {
            continue;
        }
        let close_paren = before_dot - 1;
        let Some(open_paren) = find_matching_open_paren(line, close_paren) else {
            continue;
        };
        // Walk back over the method name.
        let mut j = open_paren;
        while j > 0 {
            let c = bytes[j - 1];
            if c.is_ascii_alphanumeric() || c == b'_' {
                j -= 1;
            } else {
                break;
            }
        }
        let method_bytes = &bytes[j..open_paren];
        // The character before the method name must be `.` for this
        // to be a method call (not a free function).
        if j == 0 || bytes[j - 1] != b'.' {
            continue;
        }
        for &dm in DANGEROUS_METHODS {
            if method_bytes == dm.as_bytes() {
                return Some((dm, field));
            }
        }
    }
    None
}

/// Find the byte index of the `(` matching the `)` at `close`.
/// Returns `None` on imbalance or strings/comments inside the line.
fn find_matching_open_paren(line: &str, close: usize) -> Option<usize> {
    let bytes = line.as_bytes();
    if bytes[close] != b')' {
        return None;
    }
    let mut depth: i32 = 1;
    let mut i = close;
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b')' => depth += 1,
            b'(' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Recursively collect `*.rs` files under `dir`, skipping `target/`.
fn collect_rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name == "target" || name == ".git" {
            continue;
        }
        if path.is_dir() {
            collect_rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn no_method_lvalue_footgun_in_workspace() {
    // CARGO_MANIFEST_DIR -> crates/robin_engine; workspace root is two up.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest.parent().and_then(Path::parent).unwrap().to_owned();
    let crates_dir = workspace.join("crates");

    let mut files = Vec::new();
    collect_rust_files(&crates_dir, &mut files);
    assert!(!files.is_empty(), "found no .rs files under {crates_dir:?}");

    // Skip this test file itself — it contains the patterns as data
    // strings, and even though they aren't real call sites, we want
    // to avoid any ambiguity.
    let self_path = PathBuf::from(file!());
    let self_name = self_path.file_name().unwrap();

    let mut hits: Vec<String> = Vec::new();
    for path in &files {
        if path.file_name() == Some(self_name) {
            continue;
        }
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        for (lineno, line) in text.lines().enumerate() {
            // Skip comments quickly.
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.starts_with("///") {
                continue;
            }
            // Look for every `=` in the line and test the context.
            for (idx, _) in line.match_indices('=') {
                if !is_assignment_op_at(line, idx) {
                    continue;
                }
                if is_inside_string_literal(line, idx) {
                    continue;
                }
                if let Some((method, field)) = line_matches_footgun(line, idx) {
                    hits.push(format!(
                        "{}:{}: `.{method}(...).{field} = ...` modifies a \
                         temporary returned by `{method}` and is silently \
                         dropped.\n  | {}",
                        path.strip_prefix(&workspace).unwrap_or(path).display(),
                        lineno + 1,
                        line.trim_end(),
                    ));
                }
            }
        }
    }

    if !hits.is_empty() {
        panic!(
            "Detected {} method-lvalue-footgun call site(s).\n\n\
             Each line below assigns to a field of a value returned by one of \
             {DANGEROUS_METHODS:?}, which Rust silently treats as a write to a \
             temporary that is dropped at the end of the expression.  Use the \
             matching `set_*` setter, or read into a local, mutate, then write \
             back.\n\n{}\n",
            hits.len(),
            hits.join("\n"),
        );
    }
}

/// Sanity check: the scanner itself fires on the canonical bad form.
#[test]
fn scanner_detects_canonical_bad_form() {
    let line = "        elem.position().x = 1.0;";
    let eq_idx = line.find('=').unwrap();
    assert!(is_assignment_op_at(line, eq_idx));
    let hit = line_matches_footgun(line, eq_idx);
    assert_eq!(hit, Some(("position", "x")));

    let line2 = "    self.element.get_position_map().y += 0.5;";
    // `+=` — find the `=` after the `+`.
    let eq_idx2 = line2.find("+=").unwrap() + 1;
    assert!(is_assignment_op_at(line2, eq_idx2));
    assert_eq!(
        line_matches_footgun(line2, eq_idx2),
        Some(("get_position_map", "y")),
    );
}

/// Sanity check: comparison and other innocuous uses do not fire.
#[test]
fn scanner_ignores_comparisons_and_setters() {
    // Comparison — must not fire.
    let line = "if elem.position().x == 1.0 {";
    for (idx, _) in line.match_indices('=') {
        if is_assignment_op_at(line, idx) {
            assert_eq!(line_matches_footgun(line, idx), None);
        }
    }

    // Setter call — must not fire (no `.field` projection).
    let line = "elem.set_position(p);";
    for (idx, _) in line.match_indices('=') {
        assert!(!is_assignment_op_at(line, idx) || line_matches_footgun(line, idx).is_none());
    }

    // Reading into a local, then mutating the local — fine.
    let line = "let mut p = elem.position(); p.x = 1.0;";
    for (idx, _) in line.match_indices('=') {
        if is_assignment_op_at(line, idx) {
            assert_eq!(line_matches_footgun(line, idx), None);
        }
    }
}
