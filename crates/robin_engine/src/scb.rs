//! Sim-side `.scb` script-bytecode types.
//!
//! The parser (`parse_bytes`, `parse_file`) and the `Error` type live in
//! `robin_assets::scb`. Only the data types the VM needs at runtime live
//! here so engine can be compiled without robin_assets. See Decision 2.

pub use crate::vm::Quad;

/// `.scb` format version the loader accepts.
pub const SCB_VERSION: f32 = 1.5;

/// 8-byte magic at the start of every `.scb` file.
pub const SCB_MAGIC: &[u8; 8] = b"SBSCRIPT";

/// Primitive type tag for a script variable or function.
#[repr(u8)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum TypeTag {
    NotDefined = 0,
    Bool = 1,
    Int = 2,
    Float = 3,
    Void = 4,
    Event = 5,
    Function = 6,
    NativeType = 7,
    NativeFunction = 8,
}

impl TypeTag {
    pub fn from_u8(b: u8) -> Option<Self> {
        Some(match b {
            0 => Self::NotDefined,
            1 => Self::Bool,
            2 => Self::Int,
            3 => Self::Float,
            4 => Self::Void,
            5 => Self::Event,
            6 => Self::Function,
            7 => Self::NativeType,
            8 => Self::NativeFunction,
            _ => return None,
        })
    }
}

/// The subset of `SCType` that lands on disk.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ScType {
    pub tag: TypeTag,
    pub native_type_name: String,
}

/// One member variable of a script class.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct MemberVariable {
    pub ty: ScType,
    pub name: String,
    pub address: i32,
}

/// One function of a script class.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct Function {
    pub name: String,
    pub address: i32,
    pub num_parameters: i32,
    pub size_of_return_value: i32,
    pub size_of_parameters: i32,
    pub size_of_volatile: i32,
    pub size_of_temporary: i32,
}

/// Fully parsed contents of one class in a `.scb` file.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ClassEntry {
    pub source_file: String,
    pub class_name: String,
    pub size_of_member_variables: i32,
    pub member_variables: Vec<MemberVariable>,
    pub functions: Vec<Function>,
    pub quads: Vec<Quad>,
}

/// A parsed `.scb` file.
#[derive(
    Debug,
    Clone,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ScbFile {
    pub version: f32,
    pub classes: Vec<ClassEntry>,
}

/// Sim-side error surface. The parser-specific `Error` lives in
/// `robin_assets::scb` and converts into `ScriptError::Load`.
#[derive(Debug)]
pub enum Error {
    /// Stringified parser error from `robin_assets::scb::Error`.
    Parse(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Parse(s) => write!(f, "scb parse error: {s}"),
        }
    }
}

impl std::error::Error for Error {}
