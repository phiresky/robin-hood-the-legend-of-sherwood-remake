//! Tiny CLI: disassemble `.scb` files to stdout or a directory.
//!
//! Single-file:
//!   cargo run --example disasm_scb -- --decompile path/to/mission.scb
//!
//! Batch (writes <out>/<name>.ts per file plus a _duplicates.md summary):
//!   cargo run --example disasm_scb -- --decompile --datadir <dd> \
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

#[derive(Parser, Debug)]
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
    tracing_subscriber::fmt::init();
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
                continue;
            }
        };
        // Existing pre-batch bug: `structure_range_d` panics on certain
        // control-flow shapes (sherwood hub). Catch so one bad script
        // doesn't kill the whole batch.
        // Suppress the default panic hook for the duration of this call
        // so a caught panic doesn't spam stderr with a backtrace.
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            render(&scb, args.decompile, names.as_ref())
        }));
        std::panic::set_hook(prev_hook);
        let text = match result {
            Ok(t) => t,
            #[allow(clippy::print_stderr)]
            Err(_) => {
                eprintln!("WARN: {path}: decompiler panicked; skipping");
                continue;
            }
        };

        // Record class bodies for the duplicate summary.
        if args.decompile {
            for (cls, body) in extract_classes(&text) {
                let base = strip_hash_suffix(&cls);
                occurrences.push((stem.clone(), cls, hash_body(body), base));
            }
        }

        let out_path = out_dir.join(format!("{stem}.ts"));
        if let Err(e) = std::fs::write(&out_path, text) {
            tracing::error!("write {}: {e}", out_path.display());
            continue;
        }
        ok += 1;
    }

    if args.decompile && !occurrences.is_empty() {
        let summary_path = out_dir.join("_duplicates.md");
        let summary = build_duplicate_summary(&occurrences);
        if let Err(e) = std::fs::write(&summary_path, summary) {
            tracing::error!("write {}: {e}", summary_path.display());
        }
    }

    tracing::info!(
        "wrote {ok}/{} files to {}",
        args.paths.len(),
        out_dir.display()
    );
    std::process::ExitCode::SUCCESS
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
fn extract_classes(text: &str) -> impl Iterator<Item = (String, &str)> {
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
            // Stash: because we can't return a &str easily with ownership
            // in this closure-less loop, collect owned and return Vec.
            out.push((name.to_owned(), body));
            i = j + 1;
        } else {
            i += 1;
        }
    }
    // Convert owned bodies back to static-lifetime &str via leaking — OK
    // here because batch mode is a one-shot process and Strings are small
    // relative to the overall run.
    out.into_iter().map(|(n, b)| {
        let leaked: &'static str = Box::leak(b.into_boxed_str());
        (n, leaked as &str)
    })
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
        let mut groups: Vec<&Vec<&Occurrence>> = by_hash.values().collect();
        groups.sort_by_key(|v| std::cmp::Reverse(v.len()));

        let _ = writeln!(
            md,
            "## `{base}` — {} instances, {} distinct bodies",
            insts.len(),
            by_hash.len()
        );
        for (i, group) in groups.iter().enumerate() {
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
