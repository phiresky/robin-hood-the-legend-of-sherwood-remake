import assert from 'node:assert/strict';
import test from 'node:test';
import { runtimeBuildPlan } from './build-runtime.mjs';

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
