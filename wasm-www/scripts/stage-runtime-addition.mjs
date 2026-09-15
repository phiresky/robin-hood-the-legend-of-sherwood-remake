import { writeBrotliWasm } from './compress-runtime-wasm.mjs';
import { createHash } from 'node:crypto';
import { mkdir, readFile, writeFile, rm, copyFile } from 'node:fs/promises';
import { resolve, join, dirname } from 'node:path';
import { pathToFileURL } from 'node:url';
import { parseArgs } from 'node:util';
import { buildRuntime, buildReplayAdmission, requireSharedMemoryImport, run } from './build-runtime.mjs';
import { readDatadirRelease } from './datadir-release.mjs';

// Production runtimes are the threaded (shared-memory, rayon decode pool) build.
// The page is cross-origin isolated by deploy/public-headers.txt; where it is
// not, the runtime detects that and keeps sprite decode on its serial path.
export const RUNTIME_ADDITION_BUILD = Object.freeze({ threads: true, optimize: false, requireIdentity: true });

export function validateFullManifestSha256(fullSha) {
    if (fullSha && !/^[0-9a-f]{64}$/u.test(fullSha)) throw new Error('Full manifest identity must be lowercase SHA-256');
    return fullSha || null;
}

/**
 * The compatibility numbers copied into manifest.json. The document itself is
 * checked against the compiled Rust constants by `export_runtime_contract
 * --check` below, which catches a stale netProtocol before publishing.
 */
export async function readRuntimeContract(repoRoot = resolve(import.meta.dirname, '..', '..')) {
    const contract = JSON.parse(await readFile(resolve(repoRoot, 'wasm-www/runtime-contract.json'), 'utf8'));
    for (const key of ['netProtocol', 'ticketSchema', 'contentSchema']) {
        if (!Number.isSafeInteger(contract?.[key]) || contract[key] <= 0) {
            throw new Error(`runtime-contract.json has an invalid ${key}`);
        }
    }
    return contract;
}

function output(command, args) {
    const env = { ...process.env };
    if (command === 'git') {
        // Match Cargo's shared provenance policy, including when invoked by
        // a Git hook selecting a different repository in its environment.
        for (const key of ['GIT_DIR', 'GIT_WORK_TREE', 'GIT_COMMON_DIR', 'GIT_NAMESPACE']) delete env[key];
    }
    return run(command, args, { encoding: 'utf8', stdio: 'pipe', env }).stdout.trim();
}

export async function stageRuntimeAddition({
    root = 'target/static-runtime-addition', bindgen = 'wasm-bindgen', datadirRelease, fullSha,
}) {
    if (datadirRelease === undefined) throw new Error('runtime staging requires --datadir-release');
    const demo = await readDatadirRelease(datadirRelease);
    const full = validateFullManifestSha256(fullSha);
    const contract = await readRuntimeContract();
    // `/wasm/<short>/` is the same 12-hex prefix Rust records as
    // ROBIN_GIT_HASH in every replay, so viewers select a run's runtime by it.
    const commit = output('git', ['rev-parse', 'HEAD']);
    const short = commit.slice(0, 12);
    // Refuse to mix prior outputs or silently replace an immutable addition.
    await mkdir(dirname(root), { recursive: true });
    await mkdir(root, { recursive: false });
    const artifact = join(root, 'wasm', short);
    await mkdir(artifact, { recursive: true });
    // Operator staging can run on an unreviewed checkout. Validating JSON alone
    // cannot detect a stale generated document after an engine protocol change.
    run('cargo', ['build', '--locked', '-p', 'robin_rs', '--example', 'export_runtime_contract'],
        { env: { ...process.env, ROBIN_REQUIRE_BUILD_IDENTITY: '1' } });
    run(resolve('target/debug/examples/export_runtime_contract'),
        ['--check', 'wasm-www/runtime-contract.json']);
    buildRuntime({ outDir: artifact, bindgen, ...RUNTIME_ADDITION_BUILD });
    await rm(join(artifact, 'robin.d.ts'));
    await rm(join(artifact, 'robin_bg.wasm.d.ts'));
    run(process.execPath, ['wasm-www/scripts/stage-browser-identity-origin.mjs', 'engine', artifact, 'robin.js']);
    // Retain production's existing optimization order and exact role staging.
    const optimized = join(artifact, 'robin_bg.opt.wasm');
    run('wasm-opt', ['-Oz', '--strip-debug', '--strip-dwarf', '-o', optimized, join(artifact, 'robin_bg.wasm')]);
    await copyFile(optimized, join(artifact, 'robin_bg.wasm'));
    await rm(optimized);
    run('wasm-strip', [join(artifact, 'robin_bg.wasm')]);
    // The published engine, not only Cargo's output, must keep the shared
    // memory contract its decode workers instantiate against.
    requireSharedMemoryImport(join(artifact, 'robin_bg.wasm'));
    run('gzip', ['-9', '-n', '-k', join(artifact, 'robin.js'), join(artifact, 'robin_bg.wasm')]);
    await writeBrotliWasm(join(artifact, 'robin_bg.wasm'));
    buildReplayAdmission({ outDir: artifact, bindgen, requireIdentity: true });
    await rm(join(artifact, 'replay_admission.d.ts'));
    await rm(join(artifact, 'replay_admission_bg.wasm.d.ts'));
    run(process.execPath, ['wasm-www/scripts/stage-engine-preload-assets.mjs', 'assets/core-datadir', artifact]);
    const javascriptModules = JSON.parse(output(process.execPath, ['wasm-www/scripts/runtime-javascript-modules.mjs', artifact, '--replay-admission']));
    const hash = async name => createHash('sha256').update(await readFile(join(artifact, name))).digest('hex');
    const manifest = {
        // Runtime manifests use canonical whole-second UTC timestamps.
        commit, short, builtAt: new Date().toISOString().replace(/\.\d{3}Z$/u, 'Z'), netProtocol: contract.netProtocol,
        ticketSchema: contract.ticketSchema,
        multiplayerContent: { schema: contract.contentSchema,
            demo: { url: demo.url, sha256: demo.sha256, byteLength: demo.byte_length, nativeContentSha256: demo.native_content_sha256 },
            full: full ? { manifestSha256: full } : null },
        files: { js: 'robin.js', jsGzip: 'robin.js.gz', wasm: 'robin_bg.wasm', wasmGzip: 'robin_bg.wasm.gz', wasmBrotli: 'robin_bg.wasm.br', replayAdmissionJs: 'replay_admission.js', replayAdmissionWasm: 'replay_admission_bg.wasm' },
        javascriptModules, sha256: { wasm: await hash('robin_bg.wasm'), wasmGzip: await hash('robin_bg.wasm.gz'), wasmBrotli: await hash('robin_bg.wasm.br'), replayAdmissionJs: await hash('replay_admission.js'), replayAdmissionWasm: await hash('replay_admission_bg.wasm') },
    };
    await writeFile(join(artifact, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`, { flag: 'wx' });
    await copyFile(join(artifact, 'manifest.json'), join(root, 'wasm/latest.json'));
    run(process.execPath, ['wasm-www/scripts/verify-runtime-corpus.mjs', '--addition', root]);
    return short;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
    const { values } = parseArgs({ options: {
        root: { type: 'string' },
        bindgen: { type: 'string' },
        'datadir-release': { type: 'string' },
        'full-content-manifest-sha256': { type: 'string' },
    } });
    // Resolve operator paths before moving to the repository root.
    const datadirRelease = values['datadir-release'] === undefined ? undefined : resolve(values['datadir-release']);
    const root = values.root === undefined ? undefined : resolve(values.root);
    process.chdir(resolve(import.meta.dirname, '../..'));
    const short = await stageRuntimeAddition({ root, bindgen: values.bindgen, datadirRelease,
        fullSha: values['full-content-manifest-sha256'] });
    console.log(`staged immutable runtime addition ${short}`);
}
