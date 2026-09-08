import assert from 'node:assert/strict';
import test from 'node:test';
import {
    DEPLOYMENT,
    loadDeploymentSnapshot,
    validateDeploymentSnapshot,
    verifyDeploymentConfig,
} from './verify-cloudflare-deployment.mjs';

function clone(snapshot) {
    return structuredClone(snapshot);
}

test('checked-in Cloudflare topology is internally exact', async () => {
    await verifyDeploymentConfig();
});

test('public topology isolates API, wasm, and datadir routes ahead of the broad site Worker', async () => {
    const snapshot = await loadDeploymentSnapshot();
    for (const mutate of [
        candidate => candidate.routes.routes.shift(),
        candidate => candidate.routes.routes.splice(1, 1),
        candidate => { candidate.routes.routes[0].pattern = `${DEPLOYMENT.publicHost}/api`; },
        candidate => { candidate.publicWrangler.routes = [{ pattern: `${DEPLOYMENT.publicHost}/*` }]; },
        candidate => { candidate.publicWrangler.assets.not_found_handling = 'single-page-application'; },
        candidate => { candidate.routes.routes[2].script = DEPLOYMENT.publicWorker; },
        candidate => { candidate.runtimeWrangler.routes = [{ pattern: `${DEPLOYMENT.publicHost}/wasm/*` }]; },
        candidate => { candidate.datadirWrangler.name = DEPLOYMENT.runtimeWorker; },
    ]) {
        const candidate = clone(snapshot);
        mutate(candidate);
        assert.throws(
            () => validateDeploymentSnapshot(candidate),
            /API\/ACME bypasses|unexpected keys|missing-asset policy|datadir Worker name/u,
        );
    }
});

test('signer remains an exact distinct Custom Domain', async () => {
    const snapshot = await loadDeploymentSnapshot();
    for (const [field, value] of [
        ['pattern', DEPLOYMENT.publicHost],
        ['pattern', `${DEPLOYMENT.signerHost}/identity-signer/`],
        ['custom_domain', false],
    ]) {
        const candidate = clone(snapshot);
        candidate.signerWrangler.routes[0][field] = value;
        assert.throws(() => validateDeploymentSnapshot(candidate), /signer (Custom Domain|route type)/u);
    }
});

test('header policies fail closed on clickjacking and cache weakening', async () => {
    const snapshot = await loadDeploymentSnapshot();
    const hostile = [
        candidate => { candidate.publicHeaders = candidate.publicHeaders.replace("frame-ancestors 'none'", 'frame-ancestors *'); },
        candidate => { candidate.signerHeaders = candidate.signerHeaders.replace(`frame-ancestors ${DEPLOYMENT.publicOrigin}`, 'frame-ancestors *'); },
        candidate => { candidate.signerHeaders += '\n/*\n  X-Frame-Options: DENY\n'; },
        candidate => { candidate.publicHeaders = candidate.publicHeaders.replace('max-age=31536000, immutable', 'max-age=60'); },
        candidate => { candidate.publicHeaders = candidate.publicHeaders.replace("connect-src 'self' https: wss:", "connect-src 'self' https:"); },
        candidate => { candidate.publicHeaders = candidate.publicHeaders.replace('! Content-Security-Policy', '! Referrer-Policy'); },
        candidate => { candidate.publicHeaders = candidate.publicHeaders.replace("connect-src 'self'; img-src", "connect-src 'self' https:; img-src"); },
        candidate => { candidate.runtimeHeaders = candidate.runtimeHeaders.replace('/wasm/latest.json', '/wasm/*'); },
        candidate => { candidate.datadirHeaders = candidate.datadirHeaders.replace('datadir-v1', 'runtime-v1'); },
    ];
    for (const mutate of hostile) {
        const candidate = clone(snapshot);
        mutate(candidate);
        assert.throws(() => validateDeploymentSnapshot(candidate), /frame-ancestors|authorized parent|cache|revalidate|relay origins|leaderboard|Static-Origin/u);
    }
});

test('Wrangler is an exact lockfile-controlled tool, not a moving range', async () => {
    const snapshot = await loadDeploymentSnapshot();
    snapshot.packageJson.devDependencies.wrangler = '^4.127.1';
    assert.throws(() => validateDeploymentSnapshot(snapshot), /exactly pinned/u);
});
