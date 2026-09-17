# Local bitcode changes

Source: SoftbearStudios/bitcode revision 7ae88076943b74151e99713f33635ac5a5d4f4bf.
The derive macro stays pinned to that revision. Encoding is unchanged.

`decode_with_limit` scopes a cumulative per-thread budget to one synchronous
native-derived decode. Primitive scratch buffers, collection entries, owned
strings, and smart pointers are charged before population allocates them.
An unwind-safe guard restores the prior state. Ordinary decoding has no limit
unless it runs inside that scope. Custom decoder allocations are outside the
contract; authored sprites use the audited built-in derive paths.

The charge includes conservative collection overhead and scratch growth. It
bounds admitted storage/work rather than exact allocator capacity or process RSS.
Zero-sized entries still consume budget. The sprite loader separately limits
compressed input, Zstd window size, expanded bytes, and materialized frame memory.

Regression coverage lives in robin_assets::custom_sprites::admission tests.
