// Runner for crates/robin_assets/examples/wasm_decode_bench.rs — see that
// file's header for the build steps. Decodes every mission of a converted
// shipping datadir under node and reports per-mission and total timings for
// the two decode phases (zstd+bitcode part decode; sprite_codec VQ chunk
// materialization). Shared part files are decoded once per mission that
// names them, mirroring per-mission install.
import { readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const [root, glueDir] = process.argv.slice(2);
if (!root || !glueDir) {
    console.error('usage: node scripts/wasm_decode_bench.mjs <converted-datadir-root> <wasm-bindgen-out-dir>');
    process.exit(2);
}

const glue = await import(pathToFileURL(resolve(glueDir, 'wasm_decode_bench.js')));
await glue.default({
    module_or_path: readFileSync(join(glueDir, 'wasm_decode_bench_bg.wasm')),
});

const dataDir = join(root, 'Data');
const t0 = performance.now();
const missions = JSON.parse(glue.bench_init(readFileSync(join(dataDir, 'datadir.bin'))));
const initMs = performance.now() - t0;
console.log(`datadir.bin decoded in ${initMs.toFixed(0)} ms; ${Object.keys(missions).length} missions`);

const totals = { part_bytes: 0, decode_ms: 0, materialize_ms: 0, vq_chunks: 0, vq_blob_bytes: 0, vq_sprites: 0 };
const mb = (n) => (n / 1e6).toFixed(1);
console.log('mission                     parts-MB  decode-ms  materialize-ms  chunks  sprites');
for (const [mission, files] of Object.entries(missions)) {
    const parts = files.map((f) => readFileSync(join(dataDir, f)));
    const r = JSON.parse(glue.bench_mission(parts));
    for (const k of Object.keys(totals)) totals[k] += r[k];
    console.log(
        `${mission.padEnd(28)}${mb(r.part_bytes).padStart(8)}${r.decode_ms.toFixed(0).padStart(11)}` +
        `${r.materialize_ms.toFixed(0).padStart(16)}${String(r.vq_chunks).padStart(8)}${String(r.vq_sprites).padStart(9)}`,
    );
}
console.log(
    `TOTAL: ${mb(totals.part_bytes)} MB parts, ${(totals.decode_ms / 1000).toFixed(1)} s zstd+bitcode, ` +
    `${(totals.materialize_ms / 1000).toFixed(1)} s sprite_codec materialize ` +
    `(${totals.vq_chunks} chunks, ${totals.vq_sprites} VQ sprites, ${mb(totals.vq_blob_bytes)} MB blobs)`,
);
