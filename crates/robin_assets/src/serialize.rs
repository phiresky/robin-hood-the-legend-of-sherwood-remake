//! Serialization traits for ported game classes.
//!
//! Game objects use serde for JSON serialization. The `BinarySerialize`
//! trait is for legacy CPF binary format reading only.

use robin_engine::sbfile::SbFile;

/// Trait for reading legacy CPF binary data.
pub trait BinarySerialize {
    fn load_legacy(&mut self, file: &mut SbFile) -> Result<(), i32>;
}

/// Trait for primitive types readable from binary.
pub trait SerializeVar {
    fn read_from(&mut self, file: &mut SbFile) -> Result<(), i32>;
}

impl SerializeVar for u8 {
    fn read_from(&mut self, f: &mut SbFile) -> Result<(), i32> {
        f.serialize_u8(self)
    }
}
impl SerializeVar for u16 {
    fn read_from(&mut self, f: &mut SbFile) -> Result<(), i32> {
        f.serialize_u16(self)
    }
}
impl SerializeVar for i16 {
    fn read_from(&mut self, f: &mut SbFile) -> Result<(), i32> {
        f.serialize_i16(self)
    }
}
impl SerializeVar for u32 {
    fn read_from(&mut self, f: &mut SbFile) -> Result<(), i32> {
        f.serialize_u32(self)
    }
}
impl SerializeVar for i32 {
    fn read_from(&mut self, f: &mut SbFile) -> Result<(), i32> {
        f.serialize_i32(self)
    }
}
impl SerializeVar for u64 {
    fn read_from(&mut self, f: &mut SbFile) -> Result<(), i32> {
        f.serialize_u64(self)
    }
}
impl SerializeVar for i64 {
    fn read_from(&mut self, f: &mut SbFile) -> Result<(), i32> {
        f.serialize_i64(self)
    }
}
impl SerializeVar for f32 {
    fn read_from(&mut self, f: &mut SbFile) -> Result<(), i32> {
        f.serialize_f32(self)
    }
}
impl SerializeVar for bool {
    fn read_from(&mut self, f: &mut SbFile) -> Result<(), i32> {
        f.serialize_bool(self)
    }
}
impl SerializeVar for String {
    fn read_from(&mut self, f: &mut SbFile) -> Result<(), i32> {
        f.serialize_string(self)
    }
}
