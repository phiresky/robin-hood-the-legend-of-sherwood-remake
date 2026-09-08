import assert from 'node:assert/strict';
import test from 'node:test';
import { extractWranglerDeployVersion } from './extract-wrangler-deploy-version.mjs';

const worker = 'robinhood-runtime-assets';
const versionId = '12345678-90ab-cdef-8123-456789abcdef';

function output(event) {
    return [
        JSON.stringify({ type: 'wrangler-session', wrangler_version: '4.127.1' }),
        JSON.stringify(event),
        '',
    ].join('\n');
}

test('extracts the exact runtime Worker version from structured Wrangler output', () => {
    assert.equal(extractWranglerDeployVersion(output({
        type: 'deploy', worker_name: worker, version_id: versionId,
    }), worker), versionId);
});

test('rejects missing, duplicate, wrong-worker, and malformed deployment records', () => {
    assert.throws(() => extractWranglerDeployVersion('{}\n', worker), /exactly one/u);
    assert.throws(() => extractWranglerDeployVersion([
        output({ type: 'deploy', worker_name: worker, version_id: versionId }),
        output({ type: 'deploy', worker_name: worker, version_id: versionId }),
    ].join(''), worker), /exactly one/u);
    assert.throws(() => extractWranglerDeployVersion(output({
        type: 'deploy', worker_name: 'wrong-worker', version_id: versionId,
    }), worker), /expected/u);
    assert.throws(() => extractWranglerDeployVersion(output({
        type: 'deploy', worker_name: worker, version_id: 'not-a-version',
    }), worker), /canonical Worker version/u);
});
