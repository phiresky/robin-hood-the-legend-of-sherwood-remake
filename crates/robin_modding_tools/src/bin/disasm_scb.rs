//! Tiny CLI: disassemble `.scb` files to stdout or a directory.
//!
//! Single-file:
//!   cargo run -p robin_modding_tools --bin disasm_scb -- --decompile path/to/mission.scb
//!
//! Batch (writes <out>/<name>.ts per file plus a _duplicates.md summary):
//!   cargo run -p robin_modding_tools --bin disasm_scb -- --decompile --datadir <dd> \
//!     --out-dir /tmp/decompiled <dd>/Data/Levels/*.scb
//!
//! For named output, pass `--datadir <path>` so `GetActorScript(N)` /
//! `GetPatchScript(N)` get rewritten to `Actors.Name` / `Patches.Name`.
#![deny(clippy::print_stdout, clippy::print_stderr)]

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

use clap::Parser;

/// One class occurrence recorded during a batch run:
/// `(file_stem, class_name, body_hash, base_name)`.
type Occurrence = (String, String, u64, String);

#[derive(Parser, Debug, serde::Serialize, serde::Deserialize)]
#[command(about = "Disassemble (or decompile) Robin Hood .scb script files")]
struct Args {
    /// Decompile to high-level pseudo-source instead of raw disassembly.
    #[arg(short, long)]
    decompile: bool,

    /// Datadir to pull actor / patch names from.
    #[arg(long)]
    datadir: Option<String>,

    /// Bare mission name (single-file only). Defaults to each file's stem.
    #[arg(long)]
    mission: Option<String>,

    /// Write to `<out-dir>/<stem>.ts` per input instead of stdout.
    /// Also writes `_duplicates.md` summarizing classes shared across files.
    #[arg(long)]
    out_dir: Option<String>,

    /// One or more `.scb` paths. With `--out-dir` any number; without,
    /// exactly one (output goes to stdout).
    paths: Vec<String>,
}

fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    let args = Args::parse();

    if args.paths.is_empty() {
        tracing::error!("no input .scb files");
        return std::process::ExitCode::FAILURE;
    }

    match (args.out_dir.as_deref(), args.paths.len()) {
        (Some(dir), _) => run_batch(dir, &args),
        (None, 1) => run_single(&args.paths[0], &args),
        (None, _) => {
            tracing::error!("multiple inputs require --out-dir");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run_single(path: &str, args: &Args) -> std::process::ExitCode {
    let names = match load_actor_names(args.datadir.as_deref(), args.mission.as_deref(), path) {
        Ok(n) => n,
        Err(code) => return code,
    };
    match robin_assets::scb::parse_file(path) {
        Ok(scb) => {
            let output = render(&scb, args.decompile, names.as_ref());
            std::io::Write::write_all(&mut std::io::stdout(), output.as_bytes())
                .expect("write to stdout");
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            tracing::error!("{path}: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run_batch(out_dir: &str, args: &Args) -> std::process::ExitCode {
    let out_dir = Path::new(out_dir);
    if let Err(e) = std::fs::create_dir_all(out_dir) {
        tracing::error!("create {}: {e}", out_dir.display());
        return std::process::ExitCode::FAILURE;
    }

    let mut occurrences: Vec<Occurrence> = Vec::new();
    let mut ok = 0usize;
    let mut failed = false;

    for path in &args.paths {
        let stem = Path::new(path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());

        // Missing-mission errors from the profile lookup are non-fatal here —
        // fall through and decompile without names.
        let names =
            load_actor_names(args.datadir.as_deref(), Some(&stem), path).unwrap_or_default();
        let scb = match robin_assets::scb::parse_file(path) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("{path}: {e}");
                failed = true;
                continue;
            }
        };
        let text = render(&scb, args.decompile, names.as_ref());

        // Record class bodies for the duplicate summary.
        if args.decompile {
            for (cls, body) in extract_classes(&text) {
                let base = strip_hash_suffix(&cls);
                occurrences.push((stem.clone(), cls, hash_body(&body), base));
            }
        }

        let out_path = out_dir.join(format!("{stem}.ts"));
        if let Err(e) = std::fs::write(&out_path, text) {
            tracing::error!("write {}: {e}", out_path.display());
            failed = true;
            continue;
        }
        ok += 1;
    }

    if args.decompile && !occurrences.is_empty() {
        let summary_path = out_dir.join("_duplicates.md");
        let summary = build_duplicate_summary(&occurrences);
        if let Err(e) = std::fs::write(&summary_path, summary) {
            tracing::error!("write {}: {e}", summary_path.display());
            failed = true;
        }
    }

    tracing::info!(
        "wrote {ok}/{} files to {}",
        args.paths.len(),
        out_dir.display()
    );
    if failed {
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}

fn load_actor_names(
    datadir: Option<&str>,
    mission: Option<&str>,
    scb_path: &str,
) -> Result<Option<robin_assets::actor_names::ActorNames>, std::process::ExitCode> {
    let Some(dd) = datadir else {
        return Ok(None);
    };
    let m = mission.map(str::to_owned).unwrap_or_else(|| {
        Path::new(scb_path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    match robin_assets::actor_names::load_from_datadir(Path::new(dd), &m) {
        Ok(n) => Ok(Some(n)),
        Err(e) => {
            tracing::error!("{scb_path}: actor names: {e}");
            Err(std::process::ExitCode::FAILURE)
        }
    }
}

fn render(
    scb: &robin_assets::scb::ScbFile,
    decompile: bool,
    names: Option<&robin_assets::actor_names::ActorNames>,
) -> String {
    if decompile {
        robin_assets::decompile::decompile_with_names(scb, names)
    } else {
        robin_assets::disasm::dump(scb)
    }
}

// ── Duplicate-summary helpers ────────────────────────────────

/// Iterate `class Name … { … }` blocks in decompiled TS output.
/// Returns `(class_name, body)` pairs. Skips `abstract class` bases.
fn extract_classes(text: &str) -> impl Iterator<Item = (String, String)> {
    let mut out = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if let Some(rest) = line.strip_prefix("class ") {
            let name = rest.split([' ', '{']).next().unwrap_or("");
            let start = i;
            // Find the matching closing brace at column 0.
            let mut j = i + 1;
            while j < lines.len() && lines[j] != "}" {
                j += 1;
            }
            let body = lines[start..=j.min(lines.len() - 1)].join("\n");
            out.push((name.to_owned(), body));
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out.into_iter()
}

fn hash_body(s: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

fn strip_hash_suffix(name: &str) -> String {
    if let Some((head, tail)) = name.rsplit_once('_')
        && tail.len() == 8
        && tail.chars().all(|c| c.is_ascii_hexdigit())
    {
        head.to_owned()
    } else {
        name.to_owned()
    }
}

/// Markdown report: one section per class base that appears in ≥2 files,
/// listing the file-level instances and grouping by body hash so the
/// reader can see "these 8 files share a byte-identical `filet01`."
fn build_duplicate_summary(occ: &[Occurrence]) -> String {
    // Group: base_name -> Vec<(file, class_name, body_hash)>
    let mut by_base: HashMap<String, Vec<&Occurrence>> = HashMap::new();
    for o in occ {
        by_base.entry(o.3.clone()).or_default().push(o);
    }

    let mut bases: Vec<(&String, &Vec<&Occurrence>)> =
        by_base.iter().filter(|(_, v)| v.len() > 1).collect();
    bases.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(b.0)));

    let mut md = String::from("# Class duplicates across batch\n\n");
    let _ = writeln!(
        md,
        "Totals: {} classes in {} instances.\n",
        by_base.len(),
        occ.len()
    );

    for (base, insts) in bases {
        // Group instances by body hash.
        let mut by_hash: HashMap<u64, Vec<&Occurrence>> = HashMap::new();
        for o in insts {
            by_hash.entry(o.2).or_default().push(o);
        }
        let mut groups: Vec<_> = by_hash.iter().collect();
        // Equal-sized variants must not inherit randomized HashMap iteration.
        groups.sort_by(|(hash_a, a), (hash_b, b)| b.len().cmp(&a.len()).then(hash_a.cmp(hash_b)));

        let _ = writeln!(
            md,
            "## `{base}` — {} instances, {} distinct bodies",
            insts.len(),
            by_hash.len()
        );
        for (i, (_, group)) in groups.iter().enumerate() {
            let _ = writeln!(md, "\n**Variant {}** ({} files):", i + 1, group.len());
            let mut files: Vec<&str> = group.iter().map(|o| o.0.as_str()).collect();
            files.sort();
            for f in files {
                let _ = writeln!(md, "- `{f}`");
            }
        }
        let _ = writeln!(md);
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_bodies_are_owned_and_keep_normalized_lines_and_boundaries() {
        let classes = {
            let text = String::from(
                "abstract class Base {\r\n}\r\nclass First extends Base {\r\n  f() {\r\n  }\r\n}\r\n\r\nclass Second {\r\n}\r\n",
            );
            extract_classes(&text).collect::<Vec<_>>()
        };
        assert_eq!(
            classes,
            vec![
                (
                    "First".to_owned(),
                    "class First extends Base {\n  f() {\n  }\n}".to_owned()
                ),
                ("Second".to_owned(), "class Second {\n}".to_owned()),
            ]
        );
        assert_eq!(
            hash_body(&classes[0].1),
            hash_body("class First extends Base {\n  f() {\n  }\n}")
        );
    }

    #[test]
    fn incomplete_class_still_extends_to_end_of_input() {
        assert!(extract_classes("").next().is_none());
        for text in ["class Last {", "class Last {\n  unfinished();\n"] {
            assert_eq!(
                extract_classes(text).collect::<Vec<_>>(),
                vec![(
                    "Last".to_owned(),
                    text.lines().collect::<Vec<_>>().join("\n")
                ),]
            );
        }
    }

    #[test]
    fn variant_ties_use_body_hash_after_descending_frequency() {
        let mut occurrences = vec![
            ("z".into(), "Probe".into(), 99, "Probe".into()),
            ("b".into(), "Probe".into(), 17, "Probe".into()),
            ("a".into(), "Probe".into(), 17, "Probe".into()),
            ("y".into(), "Probe".into(), 99, "Probe".into()),
            ("single".into(), "Probe".into(), 1, "Probe".into()),
        ];
        let expected = build_duplicate_summary(&occurrences);
        assert!(expected.contains("**Variant 1** (2 files):\n- `a`\n- `b`"));
        assert!(expected.contains("**Variant 2** (2 files):\n- `y`\n- `z`"));
        assert!(expected.contains("**Variant 3** (1 files):\n- `single`"));
        for _ in 0..occurrences.len() {
            occurrences.rotate_left(1);
            assert_eq!(build_duplicate_summary(&occurrences), expected);
        }
        occurrences.reverse();
        assert_eq!(build_duplicate_summary(&occurrences), expected);
    }

    fn write_script(path: &Path) {
        // Real minimal SCB fixture: one class with no members/functions/quads.
        let mut bytes = robin_assets::scb::SCB_MAGIC.to_vec();
        bytes.extend_from_slice(&robin_assets::scb::SCB_VERSION.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        for value in ["fixture.scs", "Probe"] {
            bytes.extend_from_slice(&(value.len() as u32).to_le_bytes());
            bytes.extend_from_slice(value.as_bytes());
        }
        for _ in 0..4 {
            bytes.extend_from_slice(&0i32.to_le_bytes());
        }
        std::fs::write(path, bytes).unwrap();
    }

    fn batch_args(paths: &[&Path]) -> Args {
        Args {
            decompile: true,
            datadir: None,
            mission: None,
            out_dir: None,
            paths: paths
                .iter()
                .map(|path| path.to_str().unwrap().to_owned())
                .collect(),
        }
    }

    #[test]
    fn batch_reports_input_failure_but_still_writes_valid_outputs() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing.scb");
        let corrupt = directory.path().join("corrupt.scb");
        let valid = directory.path().join("valid.scb");
        std::fs::write(&corrupt, b"not an SCB").unwrap();
        write_script(&valid);
        let out = directory.path().join("out");
        assert_eq!(
            run_batch(
                out.to_str().unwrap(),
                &batch_args(&[&missing, &corrupt, &valid])
            ),
            std::process::ExitCode::FAILURE
        );
        assert!(
            std::fs::read_to_string(out.join("valid.ts"))
                .unwrap()
                .contains("class Probe {")
        );
        assert!(out.join("_duplicates.md").is_file());
        assert!(!out.join("missing.ts").exists());
        assert!(!out.join("corrupt.ts").exists());
    }

    #[test]
    fn batch_output_and_summary_write_failures_are_not_success() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first.scb");
        let second = directory.path().join("second.scb");
        write_script(&first);
        write_script(&second);
        let out = directory.path().join("out");
        std::fs::create_dir_all(out.join("first.ts")).unwrap();
        assert_eq!(
            run_batch(out.to_str().unwrap(), &batch_args(&[&first, &second])),
            std::process::ExitCode::FAILURE
        );
        assert!(out.join("second.ts").is_file());
        let summary_out = directory.path().join("summary-out");
        std::fs::create_dir_all(summary_out.join("_duplicates.md")).unwrap();
        assert_eq!(
            run_batch(summary_out.to_str().unwrap(), &batch_args(&[&first])),
            std::process::ExitCode::FAILURE
        );
        assert!(summary_out.join("first.ts").is_file());
    }

    #[test]
    fn batch_output_directory_creation_failure_is_not_success() {
        let directory = tempfile::tempdir().unwrap();
        let valid = directory.path().join("valid.scb");
        write_script(&valid);
        let out = directory.path().join("not-a-directory");
        std::fs::write(&out, b"existing file").unwrap();
        assert_eq!(
            run_batch(out.to_str().unwrap(), &batch_args(&[&valid])),
            std::process::ExitCode::FAILURE
        );
        assert_eq!(std::fs::read(&out).unwrap(), b"existing file");
    }

    #[test]
    fn optional_actor_names_remain_a_logged_fallback() {
        let directory = tempfile::tempdir().unwrap();
        let valid = directory.path().join("valid.scb");
        write_script(&valid);
        let mut args = batch_args(&[&valid]);
        args.datadir = Some(
            directory
                .path()
                .join("missing-datadir")
                .to_str()
                .unwrap()
                .to_owned(),
        );
        let out = directory.path().join("out");
        assert_eq!(
            run_batch(out.to_str().unwrap(), &args),
            std::process::ExitCode::SUCCESS
        );
        assert!(out.join("valid.ts").is_file());
    }
}
