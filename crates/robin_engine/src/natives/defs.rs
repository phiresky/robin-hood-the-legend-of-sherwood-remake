//! Native function IDs generated from the declarative native registry.
//!
//! The original game registers exactly 265 functions. Their IDs are an ABI:
//! shipped SCB bytecode encodes them directly in `NativeCall` instructions.

use super::signatures::native_registry;

/// Number of natives in the original game's fixed SCB namespace.
pub const ORIGINAL_NATIVE_COUNT: u32 = 265;

/// First ID reserved for Rust/Lua extensions.
pub const RUST_EXTENSION_NATIVE_START: u32 = ORIGINAL_NATIVE_COUNT;

macro_rules! define_native_fn {
    (
        original { $( $original:ident => $original_metadata:tt; )* }
        rust_extensions {
            $first_extension:ident => $first_extension_metadata:tt;
            $( $extension:ident => $extension_metadata:tt; )*
        }
    ) => {
        /// Native function ID. Original discriminants match the registration
        /// order in `VMCoreCustom::InitializeStaticExtensions`; extensions use
        /// the separate range beginning at [`RUST_EXTENSION_NATIVE_START`].
        #[repr(u32)]
        #[derive(
            Debug,
            Clone,
            Copy,
            PartialEq,
            Eq,
            Hash,
            num_enum::TryFromPrimitive,
            strum_macros::Display,
            strum_macros::IntoStaticStr,
        )]
        #[allow(missing_docs)]
        pub enum NativeFn {
            $( $original, )*
            $first_extension = RUST_EXTENSION_NATIVE_START,
            $( $extension, )*
        }
    };
}

// Original provenance: `original-code/GVMCoreCustom.cpp`,
// `VMCoreCustom::InitializeStaticExtensions`, assigns functions 0..=264 in
// this order. Signature provenance: `original-code/RHScriptAPI.scs`, the
// corresponding `NativeFunction` declarations. Rust extensions are declared
// separately below the fixed original namespace by the registry macro.
native_registry!(define_native_fn);

/// Resolves a native function index to its name, or `"unknown"`.
pub fn native_name(index: u32) -> &'static str {
    NativeFn::try_from(index).map_or("unknown", |native| native.into())
}
