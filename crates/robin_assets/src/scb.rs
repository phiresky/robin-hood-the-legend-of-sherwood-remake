//! `.scb` script-bytecode loader.
//!
//! Each mission ships a compiled script in `Data/Levels/<mission>.scb`.
//! The file holds one or more "classes" (the VM's unit of scoping), each
//! with member variables, functions, and a stream of quads (the VM's
//! 4-tuple instructions).
//!
//! Fully implemented: parses header (magic, version, class count), each
//! class's source filename / class name, member variables with type tags,
//! functions with frame layout, and raw quad streams. Tested against all
//! 39 shipped full-game scripts + the demo script.

use std::fmt;
use std::fs;
use std::path::Path;

use crate::binary_reader::{self, Reader};

// Sim-side data types live in `robin_engine::scb` so the engine can
// consume parsed scripts without depending on robin_assets. The parser
// in this module produces the engine type directly.
pub use robin_engine::scb::{
    ClassEntry, Function, MemberVariable, SCB_MAGIC, SCB_VERSION, ScType, ScbFile, TypeTag,
};
pub use robin_engine::vm::Quad;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Reader(binary_reader::Error),
    BadMagic { found: [u8; 8] },
    BadVersion { found: f32, expected: f32 },
    BadUtf8 { field: String, offset: usize },
    UnknownTypeTag(u8),
    TrailingBytes { left: usize },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Reader(e) => write!(f, "malformed .scb: {e}"),
            Error::BadMagic { found } => {
                write!(f, "not a .scb file (magic = {:?})", found)
            }
            Error::BadVersion { found, expected } => {
                write!(f, ".scb version {found} != expected {expected}")
            }
            Error::BadUtf8 { field, offset } => {
                write!(f, "invalid UTF-8 in {field} at byte {offset}")
            }
            Error::UnknownTypeTag(b) => write!(f, "unknown type tag 0x{b:02x}"),
            Error::TrailingBytes { left } => {
                write!(f, "{left} bytes left unparsed after last class")
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<binary_reader::Error> for Error {
    fn from(error: binary_reader::Error) -> Self {
        Self::Reader(error)
    }
}

/// Parses a `.scb` file: header, all classes, each class's members,
/// functions, and quad stream. Opcode semantics are not decoded — quads
/// are kept as `{u8 op, [u8;8] operands}`.
pub fn parse_bytes(bytes: &[u8]) -> Result<ScbFile, Error> {
    let mut r = Reader::new(bytes);

    let magic = r.take_array::<8>("SCB magic")?;
    if &magic != SCB_MAGIC {
        return Err(Error::BadMagic { found: magic });
    }

    let version = r.f32("SCB version")?;
    if version != SCB_VERSION {
        return Err(Error::BadVersion {
            found: version,
            expected: SCB_VERSION,
        });
    }

    // Original provenance: `original-code/virtualmachine/SCSerialize.cpp`,
    // `SCSerialize::Serialize`, writes an LE ULONG class count followed by
    // exactly that many `ClassInformations` records.
    let num_classes = r.count_u32("SCB class count", 24)?;
    let mut classes = Vec::with_capacity(num_classes);
    for class_index in 0..num_classes {
        classes.push(parse_class(&mut r, class_index)?);
    }

    // Every byte in the file should now be accounted for. Anything left
    // over means either my understanding of the format is off or the
    // file has an unknown trailing section.
    let left = r.remaining();
    if left != 0 {
        return Err(Error::TrailingBytes { left });
    }

    Ok(ScbFile { version, classes })
}

fn parse_class(r: &mut Reader<'_>, class_index: usize) -> Result<ClassEntry, Error> {
    let source_file = take_len_prefixed_string(r, format!("class {class_index} source filename"))?;
    let class_name = take_len_prefixed_string(r, format!("class {class_index} name"))?;

    // Member variables: i32 count, i32 total heap size, then N records.
    // Original provenance: `ClassInformations::Serialize` in the same file
    // reads both SLONGs before iterating `slMaxCount` member records.
    let mv_count_offset = r.position();
    let mv_count = r.count_i32(format!("class {class_index} member count"), 0)?;
    let size_of_member_variables = r.i32(format!("class {class_index} member storage size"))?;
    r.validate_count(
        mv_count,
        10,
        format!("class {class_index} member count"),
        mv_count_offset,
    )?;
    let mut member_variables = Vec::with_capacity(mv_count);
    for member_index in 0..mv_count {
        member_variables.push(parse_member_variable(r, class_index, member_index)?);
    }

    // Functions: i32 count, then N records.
    let fn_count = r.count_i32(format!("class {class_index} function count"), 28)?;
    let mut functions = Vec::with_capacity(fn_count);
    for function_index in 0..fn_count {
        functions.push(parse_function(r, class_index, function_index)?);
    }

    // Quads: i32 count, then 9 bytes per quad. Operands are opcode-
    // dependent; we keep the raw bytes. On disk the layout is little-
    // endian; BE hosts would need per-opcode byte-swapping.
    let quad_count = r.count_i32(format!("class {class_index} quad count"), 9)?;
    let mut quads = Vec::with_capacity(quad_count);
    for quad_index in 0..quad_count {
        let operation = r.u8(format!("class {class_index} quad {quad_index} opcode"))?;
        let operands =
            r.take_array::<8>(format!("class {class_index} quad {quad_index} operands"))?;
        quads.push(Quad {
            operation,
            operands,
        });
    }

    Ok(ClassEntry {
        source_file,
        class_name,
        size_of_member_variables,
        member_variables,
        functions,
        quads,
    })
}

fn parse_type(r: &mut Reader<'_>, context: &str) -> Result<ScType, Error> {
    let tag_byte = r.u8(format!("{context} type tag"))?;
    let tag = TypeTag::from_u8(tag_byte).ok_or(Error::UnknownTypeTag(tag_byte))?;
    // Native type name is length-prefixed with a single byte (not 4).
    let name_len = r.u8(format!("{context} native type name length"))? as usize;
    let name_offset = r.position();
    let name_bytes = r.take(name_len, format!("{context} native type name"))?;
    let native_type_name = std::str::from_utf8(name_bytes)
        .map(|s| s.to_owned())
        .map_err(|_| Error::BadUtf8 {
            field: format!("{context} native type name"),
            offset: name_offset,
        })?;
    Ok(ScType {
        tag,
        native_type_name,
    })
}

fn parse_member_variable(
    r: &mut Reader<'_>,
    class_index: usize,
    member_index: usize,
) -> Result<MemberVariable, Error> {
    let context = format!("class {class_index} member {member_index}");
    let ty = parse_type(r, &context)?;
    let name = take_len_prefixed_string(r, format!("{context} name"))?;
    let address = r.i32(format!("{context} address"))?;
    Ok(MemberVariable { ty, name, address })
}

fn parse_function(
    r: &mut Reader<'_>,
    class_index: usize,
    function_index: usize,
) -> Result<Function, Error> {
    let context = format!("class {class_index} function {function_index}");
    let name = take_len_prefixed_string(r, format!("{context} name"))?;
    let address = r.i32(format!("{context} address"))?;
    let num_parameters = r.i32(format!("{context} parameter count"))?;
    let size_of_return_value = r.i32(format!("{context} return size"))?;
    let size_of_parameters = r.i32(format!("{context} parameter size"))?;
    let size_of_volatile = r.i32(format!("{context} volatile size"))?;
    let size_of_temporary = r.i32(format!("{context} temporary size"))?;
    Ok(Function {
        name,
        address,
        num_parameters,
        size_of_return_value,
        size_of_parameters,
        size_of_volatile,
        size_of_temporary,
    })
}

/// Convenience: read+parse a `.scb` file from disk.
pub fn parse_file<P: AsRef<Path>>(path: P) -> Result<ScbFile, Error> {
    let p = path.as_ref();
    let resolved =
        robin_engine::sbfile::resolve_case_insensitive(p).unwrap_or_else(|| p.to_path_buf());
    let bytes = fs::read(resolved)?;
    parse_bytes(&bytes)
}

fn take_len_prefixed_string(r: &mut Reader<'_>, context: String) -> Result<String, Error> {
    let len = r.u32(format!("{context} length"))? as usize;
    let offset = r.position();
    let bytes = r.take(len, context.clone())?;
    // TODO(parity): shipped SCBs are ASCII/UTF-8 in practice. If an Original
    // asset contains an 8-bit codepage string, identify that codepage instead
    // of substituting replacement characters.
    std::str::from_utf8(bytes)
        .map(|s| s.to_owned())
        .map_err(|_| Error::BadUtf8 {
            field: context,
            offset,
        })
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;

    /// Path to the demo mission script, resolved relative to this crate.
    /// Returns None if the datadir isn't checked out (e.g. in CI without
    /// assets).
    fn demo_scb_path() -> Option<std::path::PathBuf> {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let p = std::path::PathBuf::from(manifest_dir)
            .join("../../datadirs/demo/Data/Levels/Dem_Lei_MP.scb");
        p.canonicalize().ok()
    }

    #[test]
    fn rejects_non_scb_magic() {
        let bytes = b"not a scb file, not even close......";
        let err = parse_bytes(bytes).unwrap_err();
        assert!(matches!(err, Error::BadMagic { .. }));
    }

    #[test]
    fn rejects_truncated_header() {
        let bytes = b"SBSCRI";
        let err = parse_bytes(bytes).unwrap_err();
        assert!(matches!(err, Error::Reader(_)));
        assert!(err.to_string().contains("SCB magic at byte 0"));
    }

    #[test]
    fn rejects_class_count_that_cannot_fit_remaining_bytes() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(SCB_MAGIC);
        bytes.extend_from_slice(&SCB_VERSION.to_le_bytes());
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());

        let err = parse_bytes(&bytes).unwrap_err();
        assert!(err.to_string().contains("SCB class count"));
        assert!(err.to_string().contains("count 4294967295"));
    }

    #[test]
    fn rejects_negative_member_count_instead_of_treating_it_as_empty() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(SCB_MAGIC);
        bytes.extend_from_slice(&SCB_VERSION.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes()); // source filename
        bytes.extend_from_slice(&0u32.to_le_bytes()); // class name
        bytes.extend_from_slice(&(-1i32).to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());

        let err = parse_bytes(&bytes).unwrap_err();
        assert!(err.to_string().contains("class 0 member count"));
        assert!(err.to_string().contains("negative count -1"));
    }

    #[test]
    fn rejects_quad_count_before_allocating_or_advancing_past_input() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(SCB_MAGIC);
        bytes.extend_from_slice(&SCB_VERSION.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes()); // source filename
        bytes.extend_from_slice(&0u32.to_le_bytes()); // class name
        bytes.extend_from_slice(&0i32.to_le_bytes()); // member count
        bytes.extend_from_slice(&0i32.to_le_bytes()); // member storage
        bytes.extend_from_slice(&0i32.to_le_bytes()); // function count
        bytes.extend_from_slice(&i32::MAX.to_le_bytes()); // quad count

        let err = parse_bytes(&bytes).unwrap_err();
        assert!(err.to_string().contains("class 0 quad count"));
        assert!(err.to_string().contains("only 0 remain"));
    }

    #[test]
    fn rejects_wrong_version() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(SCB_MAGIC);
        bytes.extend_from_slice(&2.0f32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        let err = parse_bytes(&bytes).unwrap_err();
        match err {
            Error::BadVersion { found, expected } => {
                assert_eq!(expected, 1.5);
                assert_eq!(found, 2.0);
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn empty_class_list_ok() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(SCB_MAGIC);
        bytes.extend_from_slice(&1.5f32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        let scb = parse_bytes(&bytes).unwrap();
        assert_eq!(scb.version, 1.5);
        assert!(scb.classes.is_empty());
    }

    #[test]
    fn rejects_trailing_garbage() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(SCB_MAGIC);
        bytes.extend_from_slice(&1.5f32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&[0xEE; 8]); // unexpected tail
        let err = parse_bytes(&bytes).unwrap_err();
        match err {
            Error::TrailingBytes { left } => assert_eq!(left, 8),
            other => panic!("wrong error: {other:?}"),
        }
    }

    /// Builds a minimal one-class, one-member, one-function, no-quads
    /// .scb in-memory and verifies every field round-trips.
    #[test]
    fn parses_synthetic_single_class() {
        let mut b = Vec::new();
        b.extend_from_slice(SCB_MAGIC);
        b.extend_from_slice(&1.5f32.to_le_bytes());
        b.extend_from_slice(&1u32.to_le_bytes()); // num_classes

        // --- class: source filename + class name
        let src = b"C:\\Temp\\script.scs";
        b.extend_from_slice(&(src.len() as u32).to_le_bytes());
        b.extend_from_slice(src);
        let name = b"StartUp";
        b.extend_from_slice(&(name.len() as u32).to_le_bytes());
        b.extend_from_slice(name);

        // --- member variables: 1 int "iCount" @ address 0, heap size 4
        b.extend_from_slice(&1i32.to_le_bytes()); // mv_count
        b.extend_from_slice(&4i32.to_le_bytes()); // size_of_member_variables
        b.push(TypeTag::Int as u8); // type tag
        b.push(0u8); // native type name length
        let mv_name = b"iCount";
        b.extend_from_slice(&(mv_name.len() as u32).to_le_bytes());
        b.extend_from_slice(mv_name);
        b.extend_from_slice(&0i32.to_le_bytes()); // address

        // --- functions: 1 function "Main" @ address 0
        b.extend_from_slice(&1i32.to_le_bytes()); // fn_count
        let fn_name = b"Main";
        b.extend_from_slice(&(fn_name.len() as u32).to_le_bytes());
        b.extend_from_slice(fn_name);
        b.extend_from_slice(&0i32.to_le_bytes()); // address
        b.extend_from_slice(&0i32.to_le_bytes()); // num_parameters
        b.extend_from_slice(&0i32.to_le_bytes()); // size_of_return_value
        b.extend_from_slice(&0i32.to_le_bytes()); // size_of_parameters
        b.extend_from_slice(&4i32.to_le_bytes()); // size_of_volatile
        b.extend_from_slice(&0i32.to_le_bytes()); // size_of_temporary

        // --- quads: zero of them
        b.extend_from_slice(&0i32.to_le_bytes());

        let scb = parse_bytes(&b).unwrap();
        assert_eq!(scb.classes.len(), 1);
        let c = &scb.classes[0];
        assert_eq!(c.source_file, "C:\\Temp\\script.scs");
        assert_eq!(c.class_name, "StartUp");
        assert_eq!(c.size_of_member_variables, 4);
        assert_eq!(c.member_variables.len(), 1);
        assert_eq!(c.member_variables[0].ty.tag, TypeTag::Int);
        assert_eq!(c.member_variables[0].name, "iCount");
        assert_eq!(c.member_variables[0].address, 0);
        assert_eq!(c.functions.len(), 1);
        assert_eq!(c.functions[0].name, "Main");
        assert_eq!(c.functions[0].size_of_volatile, 4);
        assert!(c.quads.is_empty());
    }

    /// Exercises the parser against the actual shipped demo mission
    /// script. This is the key differential-test hook: if the on-disk
    /// format diverges from my understanding, this fails with an error
    /// pointing at the offset.
    #[test]
    fn parses_shipped_demo_script() {
        let Some(path) = demo_scb_path() else {
            tracing::warn!("skipping: demo .scb not present");
            return;
        };
        let scb = parse_file(&path).expect("demo .scb should parse");
        assert_eq!(scb.version, 1.5);
        assert!(!scb.classes.is_empty());

        // First class is the mission script: "StartUp" authored by
        // Spellbound dev "ECoste" in Windows Temp.
        let first = &scb.classes[0];
        assert_eq!(first.class_name, "StartUp");
        assert!(
            first.source_file.contains("script.scs"),
            "source_file = {:?}",
            first.source_file
        );
        assert!(
            !first.member_variables.is_empty(),
            "StartUp should declare state for iOldSeconds* timers"
        );
        // From the hex dump: seven i32 member variables named iOldSeconds*.
        // Don't over-constrain the count here — the point is non-zero.
        assert!(!first.quads.is_empty(), "StartUp must have bytecode");

        // Sanity-check every class parsed cleanly.
        for c in &scb.classes {
            assert!(!c.class_name.is_empty());
        }
    }

    /// Asserts the demo .scb has non-trivial content across all four
    /// record types. A regression in offset arithmetic typically shows
    /// up as one of these counts going to zero (or the file not fully
    /// consuming).
    #[test]
    fn demo_script_has_content() {
        let Some(path) = demo_scb_path() else { return };
        let scb = parse_file(&path).unwrap();
        let mvars: usize = scb.classes.iter().map(|c| c.member_variables.len()).sum();
        let fns: usize = scb.classes.iter().map(|c| c.functions.len()).sum();
        let quads: usize = scb.classes.iter().map(|c| c.quads.len()).sum();
        assert!(scb.classes.len() > 1, "demo .scb has multiple classes");
        assert!(mvars > 0, "script declares member variables");
        assert!(fns > 0, "script declares functions");
        assert!(quads > 0, "script has bytecode");
    }

    /// Parse every .scb in the full-game datadirs. If the directory
    /// isn't present, the test is silently skipped. Catches format
    /// regressions across all 39 mission scripts.
    #[test]
    fn parses_all_fullgame_scripts() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let levels_dir =
            std::path::PathBuf::from(manifest_dir).join("../../datadirs/fullgame/Data/Levels");
        let Ok(levels_dir) = levels_dir.canonicalize() else {
            tracing::warn!("skipping: fullgame datadirs not present");
            return;
        };

        let mut parsed = 0;
        let mut total_classes = 0;
        let mut total_quads = 0;

        for entry in std::fs::read_dir(&levels_dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("scb") {
                continue;
            }
            let scb = parse_file(&path).unwrap_or_else(|e| {
                panic!("{}: {e}", path.display());
            });
            assert!(scb.version == SCB_VERSION);
            for c in &scb.classes {
                assert!(!c.class_name.is_empty());
                total_quads += c.quads.len();
            }
            total_classes += scb.classes.len();
            parsed += 1;
        }

        assert!(parsed > 0, "should have found .scb files");
        tracing::info!("fullgame: {parsed} scripts, {total_classes} classes, {total_quads} quads");
    }
}
