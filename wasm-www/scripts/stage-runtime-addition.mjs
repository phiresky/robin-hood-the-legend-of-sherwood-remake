import { writeBrotliWasm } from './compress-runtime-wasm.mjs';
import { createHash } from 'node:crypto';
import { mkdir, readFile, writeFile, rm, copyFile } from 'node:fs/promises';
import { resolve, join, dirname } from 'node:path';
import { pathToFileURL } from 'node:url';
import { parseArgs } from 'node:util';
import { buildRuntime, buildReplayAdmission, run } from './build-runtime.mjs';
import { verifyRuntimeSourceContract } from './verify-runtime-source-contract.mjs';

export function validateContentIdentity({ demoSha, nativeDemoSha, demoBytes, fullSha }) {
    const digest = /^[0-9a-f]{64}$/u;
    if (!digest.test(demoSha) || !digest.test(nativeDemoSha)) throw new Error('Demo identities must be lowercase SHA-256');
    if (!/^[1-9][0-9]{0,7}$/u.test(String(demoBytes)) || Number(demoBytes) > 26214400) {
        throw new Error('Demo object must be between 1 byte and 25 MiB');
    }
    if (fullSha && !digest.test(fullSha)) throw new Error('Full manifest identity must be lowercase SHA-256');
    return { demoSha, nativeDemoSha, demoBytes: Number(demoBytes), fullSha: fullSha || null };
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

export async function stageRuntimeAddition({ root = 'target/static-runtime-addition', bindgen = 'wasm-bindgen', ...input }) {
    const identity = validateContentIdentity(input);
    const contract = await verifyRuntimeSourceContract();
    const commit = output('git', ['rev-parse', 'HEAD']);
    const short = commit.slice(0, 12);
    // Refuse to mix prior outputs or silently replace an immutable addition.
    await mkdir(dirname(root), { recursive: true });
    await mkdir(root, { recursive: false });
    const artifact = join(root, 'wasm', short);
    await mkdir(artifact, { recursive: true });
    // Operator staging can run on an unreviewed checkout. Verify compiled
    // owners here as well as in CI; validating JSON alone cannot detect a
    // stale generated document after an engine protocol change.
    run('cargo', ['build', '--locked', '-p', 'robin_rs', '--example', 'export_runtime_contract'],
        { env: { ...process.env, ROBIN_REQUIRE_BUILD_IDENTITY: '1' } });
    run(resolve('target/debug/examples/export_runtime_contract'),
        ['--check', 'wasm-www/runtime-contract.json']);
    buildRuntime({ outDir: artifact, bindgen, optimize: false, requireIdentity: true });
    await rm(join(artifact, 'robin.d.ts'));
    await rm(join(artifact, 'robin_bg.wasm.d.ts'));
    run(process.execPath, ['wasm-www/scripts/stage-browser-identity-origin.mjs', 'engine', artifact, 'robin.js']);
    // Retain production's existing optimization order and exact role staging.
    const optimized = join(artifact, 'robin_bg.opt.wasm');
    run('wasm-opt', ['-Oz', '--strip-debug', '--strip-dwarf', '-o', optimized, join(artifact, 'robin_bg.wasm')]);
    await copyFile(optimized, join(artifact, 'robin_bg.wasm'));
    await rm(optimized);
    run('wasm-strip', [join(artifact, 'robin_bg.wasm')]);
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
            demo: { url: 'https://robinhood.phiresky.xyz/datadirs/demo-leicester/v8-web-opus-q80.rhdata.zst',
                sha256: identity.demoSha, byteLength: identity.demoBytes, nativeContentSha256: identity.nativeDemoSha },
            full: identity.fullSha ? { manifestSha256: identity.fullSha } : null },
        files: { js: 'robin.js', jsGzip: 'robin.js.gz', wasm: 'robin_bg.wasm', wasmGzip: 'robin_bg.wasm.gz', wasmBrotli: 'robin_bg.wasm.br', replayAdmissionJs: 'replay_admission.js', replayAdmissionWasm: 'replay_admission_bg.wasm' },
        javascriptModules, sha256: { wasm: await hash('robin_bg.wasm'), wasmGzip: await hash('robin_bg.wasm.gz'), wasmBrotli: await hash('robin_bg.wasm.br'), replayAdmissionJs: await hash('replay_admission.js'), replayAdmissionWasm: await hash('replay_admission_bg.wasm') },
    };
    await writeFile(join(artifact, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`, { flag: 'wx' });
    await copyFile(join(artifact, 'manifest.json'), join(root, 'wasm/latest.json'));
    run(process.execPath, ['wasm-www/scripts/verify-runtime-corpus.mjs', '--addition', '--current-source', root]);
    return short;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
    const { values } = parseArgs({ options: { root: { type: 'string' }, bindgen: { type: 'string' } } });
    process.chdir(resolve(import.meta.dirname, '../..'));
    const short = await stageRuntimeAddition({ root: values.root, bindgen: values.bindgen,
        demoSha: process.env.DEMO_ASSET_SHA256, nativeDemoSha: process.env.DEMO_CONTENT_IDENTITY_SHA256,
        demoBytes: process.env.DEMO_ASSET_BYTE_LENGTH, fullSha: process.env.FULL_CONTENT_MANIFEST_SHA256 });
    console.log(`staged immutable runtime addition ${short}`);
}
