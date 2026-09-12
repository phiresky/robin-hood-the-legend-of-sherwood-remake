# Robin Hood rilua patch record

Base: crates.io `rilua 0.1.24`, upstream commit
`891a5ab8cebb05ace0336ba396c2d1803049bd10` (the packaged `.cargo_vcs_info.json`).
The upstream `.crate` archive has SHA-256
`ff1250d13cf4516fbc7ca77c2cca6ff8090e03245f7e7fb6d85875d87efddc3f`.
The workspace pins this version and patches it to this directory.

Local changes, recorded in [../rilua.patch](../rilua.patch):

- `Cargo.toml`: add `libm` with `force-soft-floats` and no default features.
- `compiler/codegen.rs`, `vm/execute.rs`, `vm/value.rs`, and
  `stdlib/{base,math,string}.rs`: deterministic math in constant folding,
  runtime arithmetic, formatting, and standard-library functions. Host libm
  differences must not change simulation results across native and WASM.
- `platform.rs`: share locale-independent decimal parsing, bytewise string
  comparison, and ASCII character classification between native and WASM;
  preserve UTF-8 character boundaries when parsing numeric prefixes. Also
  expose Android standard-stream symbols and use the Windows UCRT time
  symbols on both MSVC and MinGW.
- `docs/src/api.md`: remove one trailing blank line (nonsemantic).

## Upgrading

1. Unpack and verify the upstream crate archive in a scratch directory. Do
   not apply patches directly to the shared Cargo registry cache.
2. Apply `vendor/rilua.patch` with `git apply --directory=<scratch-rilua>`
   from the repository root. For a newer upstream release, review each hunk
   and remove only changes that upstream now implements equivalently.
3. Update this record, the exact workspace version pin, and the lockfile;
   regenerate the patch against the new pristine archive.
4. Run `cargo test -p robin_spellforge` and its explicit WASM tests from
   `docs/TESTING.md`, plus affected native/browser replay acceptance.
   Include Windows/Android build checks when platform bindings change.

`crates/robin_spellforge/build.rs` hashes this manifest and every file under
`src/` into `SPELLFORGE_RILUA_SOURCE_SHA256`. Editing runtime source therefore
changes replay trust identity. This documentation and the patch record are
outside that hashed input set; documenting existing changes does not alter it.
