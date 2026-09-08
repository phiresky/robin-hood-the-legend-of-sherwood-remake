import assert from 'node:assert/strict';
import test from 'node:test';
import { runtimeBuildPlan, replayAdmissionBuildPlan } from './build-runtime.mjs';

test('game and benchmark plans preserve profile, feature and output boundaries', () => {
    const plain = runtimeBuildPlan();
    assert(plain.cargo.includes('--locked'));
    assert(plain.cargo.includes('audio'));
    assert(!plain.cargo.includes('--config'));
    assert(plain.bindgenArgs.includes('--out-name'));
    const threaded = runtimeBuildPlan({ threads: true, bench: true, profile: 'wasm-dev', optimize: false });
    assert(threaded.cargo.includes('wasm-threads'));
    assert(threaded.cargo.includes('scripts/wasm-threads.cargo-config.toml'));
    assert.equal(threaded.wasm, 'target/wasm32-unknown-unknown/wasm-dev/examples/wasm_decode_bench.wasm');
    assert(!threaded.bindgenArgs.includes('--out-name'));
    assert.equal(threaded.optimize, null);
    assert.throws(() => runtimeBuildPlan({ profile: 'release' }), /unsupported/u);
});

test('replay admission has its own capped non-threaded build and never overwrites game outputs', () => {
    const plan = replayAdmissionBuildPlan({ outDir: '/tmp/test-admission' });
    assert(plan.cargo.includes('scripts/replay-admission-wasm.cargo-config.toml'));
    assert(plan.cargo.includes('robin_replay_admission_wasm'));
    assert(!plan.cargo.includes('wasm-threads'));
    assert(plan.bindgenArgs.includes('replay_admission'));
    assert.equal(plan.optimizedWasm, '/tmp/test-admission/replay_admission_bg.wasm');
});
