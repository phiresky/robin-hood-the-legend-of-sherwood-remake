# Editor workflow and ownership refactor

Implemented on `audit2-editor`. Tested production source checkpoint:
`20ff4c59d24c655f5fd1a025fbfb503f06f3b386`.

## Boundaries

- `document-commands.ts` owns immutable part/group patch, duplicate and delete operations. Commands return revisions; `MapSession` still owns history. Group duplication now also checks the copied object IDs against existing objects, preserving historical names unless a collision requires a suffix. Stale explicit command targets throw rather than silently producing an unchanged document.
- `session-publication.ts` adapts `MapSession` into one reactive snapshot carrying document identity, dirty state and copied history arrays. Load, revision and save acknowledgments can no longer update those UI fields independently. Replaced-session saves and stale loads emit nothing; disposal invalidates pending loads and suppresses late save publications.
- `map-candidate.ts` performs filesystem reads, GLB preparation, saved-document validation, provenance checks and default document construction before the viewport is touched. It disposes parsed assets on failure. Successful candidates transfer asset ownership to the caller, which either rejects/disposes a stale load or installs it in the viewport.
- `editor-viewport.ts` owns RAF, resize observation, event cancellation, control teardown, source assets, detached ground and the renderer/context lifetime. Editable mesh clones remain non-owners. Installing another asset without retiring the existing owner is rejected.

`Editor3D.tsx` retains UI, picking, camera mathematics and synchronous publication ordering. Generation, library and datadir identity checks still precede resource replacement. Selection is cleared before retiring its highlighted resources, and revision synchronization still invalidates deleted selections. Save capture retains the exact immutable document and original directory across asynchronous writes. No wire formats, dependencies or lockfiles changed.

## Validation

From `level-editor/`, Node `v26.7.0`, workspace pnpm `12.3.4`:

- `pnpm verify`: **passed** — 26 shared/app/runner tests, 11 pipeline tests, app and pipeline TypeScript checks, production Vite build.
- Ten added Node tests cover immutable command behavior, copied-ID collision, stale command errors, coherent history/dirty publication, stale and post-unmount completion, validated candidate ownership, malformed saved document cleanup, duplicate GLB cleanup and detached ground retirement.
- `pnpm build:browser`: **passed**.
- Actual Chrome lifecycle fixture, served on isolated loopback port 5297: **passed**, four mounts and 32 alternating map loads with duplicate/delete and overlay toggling. GPU counts were stable after warmup; every unmount released all tracked buffers, textures, programs, framebuffers, vertex arrays, DOM listeners, resize observers and animation frames. Four viewport contexts were explicitly lost as expected. No WebGL errors occurred after deleting shared-resource duplicates.
- `git diff --check`: **passed**. Scoped TypeScript files formatted with Prettier 3.6.2.

Browser command (build and serve were separate):

```sh
timeout --signal=TERM --kill-after=10s 90s node app/tests/run-lifecycle.mjs http://127.0.0.1:5297
```

Tested lifecycle bundle: `lifecycle-B_EaReDN.js`, SHA-256
`7c6dd94bf6538c354483f49a9c60b8870db04e2da03314936c49d295071a57d5`.
Runner allocated and removed its own temporary Chrome profile. No game data or real directory writes were used. Rust builds were not run for this TypeScript-only track.

## Limits and remaining work

- Browser acceptance covers real GLB/WebGL rendering and teardown; reverse async load completion and save-during-edit are covered by session/adapter Node tests, not new browser timing scenarios. Candidate failure tests substitute GLTF decoding while exercising the real validation/disposal path.
- TODO: split camera/picking presentation from the remaining UI closure if those features grow. This pass establishes independently testable document, publication, loading and resource boundaries without rewriting camera interactions.
- The existing Vite large-chunk warning remains; this refactor is not a bundle-size optimization.
