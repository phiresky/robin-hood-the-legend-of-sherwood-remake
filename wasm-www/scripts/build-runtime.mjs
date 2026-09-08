import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { parseArgs } from 'node:util';

export function runtimeBuildPlan({ outDir = 'wasm-www/pkg', profile = 'wasm-release',
    threads = false, bench = false, optimize = true, bindgen = 'wasm-bindgen' } = {}) {
    if (!['wasm-dev', 'wasm-release'].includes(profile)) throw new Error('unsupported wasm profile');
    const cargo = ['build', '--locked', '-Zbuild-std=std,panic_abort',
        '--target', 'wasm32-unknown-unknown', '--profile', profile];
    if (threads) cargo.push('--config', 'scripts/wasm-threads.cargo-config.toml');
    if (bench) cargo.push('-p', 'robin_assets', '--example', 'wasm_decode_bench',
        ...(threads ? ['--features', 'wasm-threads'] : []));
    else cargo.push('-p', 'robin_rs', '--bin', 'robin', '--no-default-features',
        '--features', threads ? 'audio,wasm-threads' : 'audio');
    const wasm = `target/wasm32-unknown-unknown/${profile}/${bench ? 'examples/wasm_decode_bench' : 'robin'}.wasm`;
    return { cargo, wasm, threads, bindgen,
        bindgenArgs: ['--target', 'web', '--out-dir', outDir,
            ...(!bench ? ['--out-name', 'robin'] : []), wasm],
        optimize: optimize ? ['wasm-www/scripts/optimize-wasm.mjs', outDir] : null };
}

export function replayAdmissionBuildPlan({ outDir = 'wasm-www/pkg', bindgen = 'wasm-bindgen' } = {}) {
    const wasm = 'target/wasm32-unknown-unknown/wasm-release/robin_replay_admission_wasm.wasm';
    return {
        cargo: ['build', '--locked', '--target', 'wasm32-unknown-unknown', '--profile', 'wasm-release',
            '--config', 'scripts/replay-admission-wasm.cargo-config.toml', '-p', 'robin_replay_admission_wasm', '-j2'],
        bindgen,
        bindgenArgs: ['--target', 'web', '--out-dir', outDir, '--out-name', 'replay_admission', wasm],
        optimizedWasm: resolve(outDir, 'replay_admission_bg.wasm'),
    };
}

export function buildReplayAdmission(options) {
    const plan = replayAdmissionBuildPlan(options);
    run('cargo', plan.cargo, options?.requireIdentity
        ? { env: { ...process.env, ROBIN_REQUIRE_BUILD_IDENTITY: '1' } } : {});
    run(plan.bindgen, plan.bindgenArgs);
    // Optimize only the validator; never re-optimize the already staged game.
    run(process.execPath, ['wasm-www/scripts/optimize-wasm.mjs', plan.optimizedWasm]);
    run('bash', ['scripts/assert-replay-admission-wasm-memory.sh', plan.optimizedWasm]);
}

export function run(command, args, options = {}) {
    const result = spawnSync(command, args, { stdio: 'inherit', ...options });
    if (result.error) throw result.error;
    if (result.status !== 0) throw new Error(`${command} failed (${result.status ?? result.signal})`);
    return result;
}

export function buildRuntime(options) {
    const plan = runtimeBuildPlan(options);
    run('cargo', plan.cargo, options?.requireIdentity
        ? { env: { ...process.env, ROBIN_REQUIRE_BUILD_IDENTITY: '1' } } : {});
    if (plan.threads) {
        const result = run('wasm-objdump', ['-x', '-j', 'Import', plan.wasm], { encoding: 'utf8', stdio: 'pipe' });
        if (!/memory\[[0-9]+\].* shared /u.test(result.stdout)) {
            throw new Error('threaded wasm has no shared memory import; check target flags');
        }
    }
    run(plan.bindgen, plan.bindgenArgs);
    if (plan.optimize) run(process.execPath, plan.optimize);
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
    const { values } = parseArgs({ options: {
        'out-dir': { type: 'string' }, profile: { type: 'string' },
        threads: { type: 'boolean' }, bench: { type: 'boolean' },
        'no-opt': { type: 'boolean' }, bindgen: { type: 'string' },
    } });
    process.chdir(resolve(import.meta.dirname, '../..'));
    buildRuntime({ outDir: values['out-dir'] ?? (values.bench ? 'wasm-www/pkg-bench' : 'wasm-www/pkg'),
        profile: values.profile, threads: values.threads, bench: values.bench,
        optimize: !values['no-opt'], bindgen: values.bindgen });
}
