import assert from 'node:assert/strict';
import test from 'node:test';
import {
    currentWorkerDeployment,
    proveWorkerVersion,
} from './cloudflare-worker-version.mjs';

const accountId = 'ab'.repeat(16);
const apiToken = 'test-token-that-is-long-enough';
const worker = 'robinhood-runtime-assets';
const deploymentId = 'aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee';
const versionId = '12345678-90ab-4def-8123-456789abcdef';

function response(result, { ok = true, status = 200 } = {}) {
    return {
        ok,
        status,
        async json() { return { errors: ok ? [] : [{ message: 'denied' }], result, success: ok }; },
    };
}

function workerApi({ activeVersion = versionId, versionExists = true } = {}) {
    const requests = [];
    const fetchImpl = async url => {
        requests.push(url);
        if (url.endsWith(`/versions/${versionId}`)) {
            return versionExists ? response({ id: versionId }) : response(null, { ok: false, status: 404 });
        }
        if (url.endsWith('/deployments')) {
            return response({ deployments: [{
                id: deploymentId,
                versions: [{ percentage: 100, version_id: activeVersion }],
            }] });
        }
        throw new Error(`unexpected URL ${url}`);
    };
    return { fetchImpl, requests };
}

test('proves one exact current 100% Worker version and returns rollback identity', async () => {
    const api = workerApi();
    const proof = await proveWorkerVersion({ accountId, apiToken, worker, versionId, fetchImpl: api.fetchImpl });
    assert.deepEqual(proof.current, { deploymentId, versionId });
    assert.equal(proof.version.id, versionId);
    assert.equal(api.requests.length, 2);
});

test('rejects absent, substituted, split, and malformed deployments', async () => {
    for (const options of [
        { versionExists: false },
        { activeVersion: '11111111-2222-4333-8444-555555555555' },
    ]) {
        const api = workerApi(options);
        await assert.rejects(
            proveWorkerVersion({ accountId, apiToken, worker, versionId, fetchImpl: api.fetchImpl }),
            /failed|not the current 100%/u,
        );
    }
    const malformed = async () => response({ deployments: [{
        id: deploymentId,
        versions: [{ percentage: 50, version_id: versionId }, { percentage: 50, version_id: versionId }],
    }] });
    await assert.rejects(
        currentWorkerDeployment({ accountId, apiToken, worker, fetchImpl: malformed }),
        /not one canonical 100%/u,
    );
});

test('rollback capture permits only an explicit absent Worker', async () => {
    const absent = async () => response(null, { ok: false, status: 404 });
    assert.equal(await currentWorkerDeployment({
        accountId,
        allowAbsent: true,
        apiToken,
        fetchImpl: absent,
        worker,
    }), null);
    await assert.rejects(currentWorkerDeployment({
        accountId,
        apiToken,
        fetchImpl: absent,
        worker,
    }), /failed \(404\)/u);
});
