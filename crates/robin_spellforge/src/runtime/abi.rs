//! Exact executable identity. Keep source fingerprints conservative.
use super::*;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

const RILUA_CRATE_VERSION: &str = "0.1.24";
const RILUA_CRATES_IO_SHA256: &str =
    "ff1250d13cf4516fbc7ca77c2cca6ff8090e03245f7e7fb6d85875d87efddc3f";
const RILUA_SOURCE_SHA256: &str = env!("SPELLFORGE_RILUA_SOURCE_SHA256");
const RUNTIME_SOURCE_SHA256: &str = env!("SPELLFORGE_RUNTIME_SOURCE_SHA256");
const ENGINE_CONTRACT_SOURCE_SHA256: &str = env!("SPELLFORGE_ENGINE_CONTRACT_SOURCE_SHA256");
const CONTRACT_SEMANTICS: &str = "replace-or-augment-before-or-after;missing-filter-ai-event=1;missing-other-event=0;result=nil-zero,bool-word,exact-i32;native-bool=true-false-or-numeric-zero-one-v1;f32-words=ieee754-le;native-yield=depth-first-flat-v1;random=engine-tape-v1;module-resolution=canonical-lowercase-exact-root-precedence-v2;journal=persistent-sha256-chain-v1";
const SANDBOX_STDLIB: &str = "base,table,string,math,coroutine,debug-internal;hide=coroutine,debug,io,os,package,dofile,load,loadfile,loadstring,getfenv,setfenv,collectgarbage,_G;private-require-v1";

/// Exact executable ABI digest used in package, save, replay, and peer
/// identity. Any behavioral surface change must change an input below rather
/// than hand-editing a human-readable version label.
pub fn spellforge_vm_abi() -> &'static str {
    static ABI: OnceLock<String> = OnceLock::new();
    ABI.get_or_init(|| {
        format!(
            "{SPELLFORGE_VM_ABI_SCHEME}{}",
            robin_engine::spellforge::hex_hash(&spellforge_vm_abi_digest())
        )
    })
}

pub fn spellforge_vm_abi_digest() -> [u8; 32] {
    let mut digest = Sha256::new();
    hash_abi_component(&mut digest, b"spellforge-executable-abi-v1");
    hash_abi_component(&mut digest, &SPELLFORGE_CONTRACT_VERSION.to_le_bytes());
    hash_abi_component(&mut digest, RILUA_CRATE_VERSION.as_bytes());
    hash_abi_component(&mut digest, RILUA_CRATES_IO_SHA256.as_bytes());
    hash_abi_component(&mut digest, RILUA_SOURCE_SHA256.as_bytes());
    hash_abi_component(&mut digest, RUNTIME_SOURCE_SHA256.as_bytes());
    hash_abi_component(&mut digest, ENGINE_CONTRACT_SOURCE_SHA256.as_bytes());
    hash_abi_component(
        &mut digest,
        b"rilua-features=send;libm=0.2.16,default-features=false,force-soft-floats",
    );
    hash_abi_component(&mut digest, BOOTSTRAP.as_bytes());
    hash_abi_component(&mut digest, SANDBOX_STDLIB.as_bytes());
    hash_abi_component(&mut digest, CONTRACT_SEMANTICS.as_bytes());
    for limit in [
        PACKAGE_SOURCE_LIMIT as u64,
        MEMORY_LIMIT as u64,
        package::PACKAGE_FILE_LIMIT as u64,
        package::PACKAGE_PATH_LIMIT as u64,
        package::PACKAGE_METADATA_LIMIT as u64,
        u64::from(INSTRUCTION_LIMIT),
        u64::from(SPELLFORGE_TAPE_EVENT_LIMIT),
        u64::from(SPELLFORGE_TAPE_NATIVE_CALL_LIMIT),
        u64::from(SPELLFORGE_TAPE_TRANSCRIPT_ENTRY_LIMIT),
        SPELLFORGE_TAPE_BYTE_LIMIT,
        SPELLFORGE_SNAPSHOT_BYTE_LIMIT,
        SPELLFORGE_EVENT_NATIVE_CALL_LIMIT as u64,
        SPELLFORGE_EVENT_TRANSCRIPT_ENTRY_LIMIT as u64,
        SPELLFORGE_ARGUMENT_WORD_LIMIT as u64,
        SPELLFORGE_TAPE_STRING_BYTE_LIMIT as u64,
    ] {
        hash_abi_component(&mut digest, &limit.to_le_bytes());
    }
    hash_abi_component(&mut digest, BRIDGE_MARKER);
    hash_abi_component(&mut digest, &PSEUDO_NATIVE_RANDOM.to_le_bytes());
    for definition in NATIVE_REGISTRY {
        hash_abi_component(&mut digest, &(definition.native as u32).to_le_bytes());
        hash_abi_component(
            &mut digest,
            &[match definition.namespace {
                NativeNamespace::Original => 0,
                NativeNamespace::RustExtension => 1,
            }],
        );
        hash_abi_component(&mut digest, &[u8::from(definition.expose_to_lua)]);
        hash_abi_component(&mut digest, definition.signature.name.as_bytes());
        hash_abi_component(&mut digest, definition.signature.return_type.as_bytes());
        for parameter in definition.signature.params {
            hash_abi_component(&mut digest, parameter.ty.as_bytes());
            hash_abi_component(&mut digest, parameter.name.as_bytes());
        }
    }
    for (alias, native) in SPELLFORGE_NATIVE_ALIASES {
        hash_abi_component(&mut digest, alias.as_bytes());
        hash_abi_component(&mut digest, &(*native as u32).to_le_bytes());
    }
    digest.finalize().into()
}

fn hash_abi_component(digest: &mut Sha256, bytes: &[u8]) {
    digest.update((bytes.len() as u64).to_le_bytes());
    digest.update(bytes);
}
