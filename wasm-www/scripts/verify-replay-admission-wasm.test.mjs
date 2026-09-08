import assert from 'node:assert/strict';
import test from 'node:test';
import { verifyReplayAdmissionWasm } from './verify-replay-admission-wasm.mjs';

import { admissionFixture } from './replay-admission-wasm-fixture.mjs';

test('admission module owns exactly the prescribed memory cap and validator export', () => {
    assert.doesNotThrow(() => verifyReplayAdmissionWasm(admissionFixture()));
    for (const options of [{ max: null }, { max: 6145 }, { max: 6000 }, { shared: true }, { imported: true }, { exported: false }]) {
        assert.throws(() => verifyReplayAdmissionWasm(admissionFixture(options)), /replay admission/);
    }
    assert.throws(() => verifyReplayAdmissionWasm(Buffer.from('not wasm')));
});
