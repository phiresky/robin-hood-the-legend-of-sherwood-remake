import { existsSync, readdirSync, renameSync, statSync } from 'node:fs';
import { basename, join } from 'node:path';
import { execFileSync } from 'node:child_process';

const paths = process.argv.slice(2);

if (paths.length === 0) {
    console.error('usage: node scripts/optimize-wasm.mjs <wasm-or-dir> [wasm-or-dir...]');
    process.exit(2);
}

let optimized = 0;

for (const inputPath of paths) {
    if (!existsSync(inputPath)) {
        continue;
    }

    const stat = statSync(inputPath);
    const wasmPaths = stat.isDirectory()
        ? readdirSync(inputPath).map((entry) => join(inputPath, entry))
        : [inputPath];

    for (const wasmPath of wasmPaths) {
        if (!wasmPath.endsWith('.wasm') || wasmPath.endsWith('.debug.wasm')) {
            continue;
        }

        if (!statSync(wasmPath).isFile()) {
            continue;
        }

        const tmpPath = `${wasmPath}.opt`;
        execFileSync(
            'wasm-opt',
            // --enable-simd: modules are built with +simd128 (.cargo/config.toml).
            ['-Oz', '--enable-simd', '--strip-debug', '--strip-dwarf', '-o', tmpPath, wasmPath],
            { stdio: 'inherit' },
        );
        renameSync(tmpPath, wasmPath);
        execFileSync(
            'wasm-strip',
            ['--keep-section=__wasm_bindgen_unstable', '-o', wasmPath, wasmPath],
            { stdio: 'inherit' },
        );
        console.log(`optimized ${basename(wasmPath)}`);
        optimized += 1;
    }
}

if (optimized === 0) {
    console.warn(`no wasm files found in: ${paths.join(', ')}`);
}
