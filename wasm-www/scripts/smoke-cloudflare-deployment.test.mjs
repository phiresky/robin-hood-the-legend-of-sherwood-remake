import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import test from 'node:test';
import { DEPLOYMENT } from './verify-cloudflare-deployment.mjs';
import { smokeCloudflareDeployment } from './smoke-cloudflare-deployment.mjs';

const DEMO_BYTES = Uint8Array.from({ length: 123 }, (_, index) => index);
const DEMO_SHA256 = createHash('sha256').update(DEMO_BYTES).digest('hex');

function fixtureFetch({
    apiStatic = false,
    apiCacheControl = 'no-store',
    apiCfCacheStatus,
    apiSecondCfCacheStatus,
    apiAge,
    apiAllowOrigin,
    apiAllowCredentials,
    optionsStatus = 405,
    optionsAllowOrigin,
    optionsAllowCredentials,
    immutableCacheControl = 'public, max-age=31536000, immutable',
    immutableContentTypeOptions = 'nosniff',
    immutableAllowOrigin,
    immutableAllowCredentials,
    demoBytes = DEMO_BYTES,
    demoContentLength,
    demoManifestByteLength = DEMO_BYTES.byteLength,
    demoManifestSha256 = DEMO_SHA256,
} = {}) {
    const paths = [];
    const requests = [];
    let metadataProbes = 0;
    const short = '1234567890ab';
    const rulesetDigest = '1234567890abcdef'.repeat(4);
    const runtimeManifest = `${JSON.stringify({
        short,
        ticketSchema: 3,
        multiplayerContent: {
            schema: 2,
            demo: {
                url: `${DEPLOYMENT.publicOrigin}/datadirs/demo-leicester/v8-web-opus-q80.rhdata.zst`,
                byteLength: demoManifestByteLength,
                sha256: demoManifestSha256,
            },
        },
    })}\n`;
    const fetchImpl = async (input, options = {}) => {
        const url = new URL(input);
        paths.push(`${options.method ?? 'GET'} ${url.origin}${url.pathname}`);
        requests.push({ url, options });
        if (url.origin === DEPLOYMENT.publicOrigin && url.pathname.startsWith('/api/')) {
            if (options.method === 'OPTIONS') {
                return new Response(null, {
                    status: optionsStatus,
                    headers: {
                        ...(optionsAllowOrigin === undefined
                            ? {}
                            : { 'access-control-allow-origin': optionsAllowOrigin }),
                        ...(optionsAllowCredentials === undefined
                            ? {}
                            : { 'access-control-allow-credentials': optionsAllowCredentials }),
                    },
                });
            }
            if (url.pathname === `/api/v1/ruleset-manifests/${rulesetDigest}`) {
                return new Response(JSON.stringify({ schema_version: 1 }), {
                    status: 200,
                    headers: {
                        'content-type': 'application/json',
                        'cache-control': immutableCacheControl,
                        ...(immutableContentTypeOptions === null
                            ? {}
                            : { 'x-content-type-options': immutableContentTypeOptions }),
                        ...(immutableAllowOrigin === undefined
                            ? {}
                            : { 'access-control-allow-origin': immutableAllowOrigin }),
                        ...(immutableAllowCredentials === undefined
                            ? {}
                            : { 'access-control-allow-credentials': immutableAllowCredentials }),
                    },
                });
            }
            metadataProbes += 1;
            const cfCacheStatus = metadataProbes === 2
                ? apiSecondCfCacheStatus ?? apiCfCacheStatus
                : apiCfCacheStatus;
            return new Response(JSON.stringify({
                schema_version: 1,
                rulesets: [{ ruleset_manifest_sha256: rulesetDigest }],
            }), {
                status: 200,
                headers: {
                    'content-type': 'application/json',
                    'cache-control': apiCacheControl,
                    ...(cfCacheStatus === undefined
                        ? {}
                        : { 'cf-cache-status': cfCacheStatus }),
                    ...(apiAge === undefined ? {} : { age: apiAge }),
                    ...(apiAllowOrigin === undefined
                        ? {}
                        : { 'access-control-allow-origin': apiAllowOrigin }),
                    ...(apiAllowCredentials === undefined
                        ? {}
                        : { 'access-control-allow-credentials': apiAllowCredentials }),
                    ...(apiStatic ? { 'x-robinhood-static-origin': 'public-v1' } : {}),
                },
            });
        }
        if (url.pathname.startsWith('/.well-known/acme-challenge/')) {
            return new Response('', { status: 404, headers: { 'cf-cache-status': 'DYNAMIC' } });
        }
        if (url.pathname.startsWith('/assets/')) {
            return new Response('export {};', {
                status: 200,
                headers: { 'cache-control': 'public, max-age=31536000, immutable' },
            });
        }
        if (url.pathname === '/wasm/latest.json'
            || url.pathname === `/wasm/${short}/manifest.json`) {
            return new Response(runtimeManifest, {
                status: 200,
                headers: {
                    'content-type': 'application/json',
                    'cache-control': url.pathname.endsWith('/latest.json')
                        ? 'public, max-age=0, must-revalidate'
                        : 'public, max-age=31536000, immutable',
                    'x-robinhood-static-origin': 'runtime-v1',
                },
            });
        }
        if (url.pathname === `/wasm/${short}/robin_bg.wasm`) {
            return new Response(null, {
                status: 200,
                headers: {
                    'cache-control': 'public, max-age=31536000, immutable',
                    'x-robinhood-static-origin': 'runtime-v1',
                },
            });
        }
        if (url.pathname === '/datadirs/demo-leicester/v8-web-opus-q80.rhdata.zst') {
            return new Response(demoBytes, {
                status: 200,
                headers: {
                    'cache-control': 'public, max-age=31536000, immutable',
                    ...(demoContentLength === undefined
                        ? {}
                        : { 'content-length': demoContentLength }),
                    'x-robinhood-static-origin': 'datadir-v1',
                },
            });
        }
        if (url.origin === DEPLOYMENT.signerOrigin) {
            return new Response('<script type="module" src="../assets/signer.js"></script>', {
                status: 200,
                headers: {
                    'content-type': 'text/html',
                    'x-content-type-options': 'nosniff',
                    'x-robinhood-static-origin': 'signer-v1',
                    'content-security-policy': `default-src 'none'; frame-ancestors ${DEPLOYMENT.publicOrigin}`,
                },
            });
        }
        const assetPath = url.pathname.startsWith('/leaderboards/')
            ? '../assets/public.js'
            : './assets/public.js';
        return new Response(`<script type="module" src="${assetPath}"></script>`, {
            status: 200,
            headers: {
                'content-type': 'text/html',
                'x-content-type-options': 'nosniff',
                'x-robinhood-static-origin': 'public-v1',
                'content-security-policy': "default-src 'none'; frame-ancestors 'none'",
            },
        });
    };
    return { fetchImpl, paths, requests };
}

test('deployment smoke covers both static origins, a deep link, assets, and API bypass', async () => {
    const fixture = fixtureFetch();
    await smokeCloudflareDeployment(fixture.fetchImpl);
    assert(fixture.paths.includes(`GET ${DEPLOYMENT.publicOrigin}/leaderboards/`));
    assert(fixture.paths.includes(`GET ${DEPLOYMENT.publicOrigin}/.well-known/acme-challenge/robinhood-deployment-smoke-absent`));
    assert(fixture.paths.includes(`GET ${DEPLOYMENT.signerOrigin}/identity-signer/`));
    assert(fixture.paths.includes(`GET ${DEPLOYMENT.publicOrigin}/wasm/latest.json`));
    assert(fixture.paths.includes(`GET ${DEPLOYMENT.publicOrigin}/datadirs/demo-leicester/v8-web-opus-q80.rhdata.zst`));
    const metadataRequests = fixture.requests.filter(request =>
        request.url.href === `${DEPLOYMENT.publicOrigin}/api/v1/leaderboard-metadata`
        && request.options.method === 'GET');
    assert.equal(metadataRequests.length, 2);
    assert(metadataRequests.every(request => request.options.cache === 'no-store'));
    assert(fixture.paths.includes(`OPTIONS ${DEPLOYMENT.publicOrigin}/api/v1/leaderboard-metadata`));
    const preflight = fixture.requests.find(request => request.options.method === 'OPTIONS');
    assert(preflight !== undefined);
    const preflightHeaders = new Headers(preflight.options.headers);
    assert.equal(preflightHeaders.get('origin'), 'https://attacker.invalid');
    assert.equal(preflightHeaders.get('access-control-request-method'), 'GET');
    assert(fixture.paths.includes(
        `GET ${DEPLOYMENT.publicOrigin}/api/v1/ruleset-manifests/${'1234567890abcdef'.repeat(4)}`,
    ));
});

test('deployment smoke rejects a Demo object with a false Content-Length', async () => {
    const fixture = fixtureFetch({ demoContentLength: '124' });
    await assert.rejects(
        smokeCloudflareDeployment(fixture.fetchImpl),
        /Content-Length differs from its manifest identity/u,
    );
});

test('deployment smoke rejects an oversized Demo body without trusting headers', async () => {
    const fixture = fixtureFetch({ demoBytes: new Uint8Array(DEMO_BYTES.byteLength + 1) });
    await assert.rejects(
        smokeCloudflareDeployment(fixture.fetchImpl),
        /body exceeds its manifest byte length/u,
    );
});

test('deployment smoke rejects a same-length Demo body with the wrong digest', async () => {
    const demoBytes = DEMO_BYTES.slice();
    demoBytes[0] ^= 1;
    const fixture = fixtureFetch({ demoBytes });
    await assert.rejects(
        smokeCloudflareDeployment(fixture.fetchImpl),
        /body SHA-256 differs from its manifest identity/u,
    );
});

test('deployment smoke rejects an API response served by the static Worker', async () => {
    const fixture = fixtureFetch({ apiStatic: true });
    await assert.rejects(smokeCloudflareDeployment(fixture.fetchImpl), /served by the public static Worker/u);
});

test('deployment smoke rejects cached dynamic metadata', async () => {
    const fixture = fixtureFetch({ apiSecondCfCacheStatus: 'HIT' });
    await assert.rejects(smokeCloudflareDeployment(fixture.fetchImpl), /Cloudflare cache HIT/u);
});

test('deployment smoke rejects a weakened dynamic cache policy', async () => {
    const fixture = fixtureFetch({ apiCacheControl: 'public, max-age=60' });
    await assert.rejects(smokeCloudflareDeployment(fixture.fetchImpl), /Cache-Control: no-store/u);
});

test('deployment smoke rejects an Age header on dynamic metadata', async () => {
    const fixture = fixtureFetch({ apiAge: '0' });
    await assert.rejects(smokeCloudflareDeployment(fixture.fetchImpl), /unexpectedly exposes age/u);
});

for (const apiAllowOrigin of ['*', DEPLOYMENT.publicOrigin]) {
    test(`deployment smoke rejects metadata CORS origin ${apiAllowOrigin}`, async () => {
        const fixture = fixtureFetch({ apiAllowOrigin });
        await assert.rejects(
            smokeCloudflareDeployment(fixture.fetchImpl),
            /unexpectedly exposes access-control-allow-origin/u,
        );
    });
}

test('deployment smoke rejects credentialed metadata CORS', async () => {
    const fixture = fixtureFetch({ apiAllowCredentials: 'true' });
    await assert.rejects(
        smokeCloudflareDeployment(fixture.fetchImpl),
        /unexpectedly exposes access-control-allow-credentials/u,
    );
});

test('deployment smoke freezes the attacker-origin preflight status', async () => {
    const fixture = fixtureFetch({ optionsStatus: 204 });
    await assert.rejects(smokeCloudflareDeployment(fixture.fetchImpl), /returned 204, expected 405/u);
});

test('deployment smoke rejects permissive attacker-origin preflight headers', async () => {
    const fixture = fixtureFetch({
        optionsAllowOrigin: '*',
        optionsAllowCredentials: 'true',
    });
    await assert.rejects(
        smokeCloudflareDeployment(fixture.fetchImpl),
        /unexpectedly exposes access-control-allow-origin/u,
    );
});

test('deployment smoke rejects weakened immutable API caching', async () => {
    const fixture = fixtureFetch({ immutableCacheControl: 'public, max-age=60' });
    await assert.rejects(
        smokeCloudflareDeployment(fixture.fetchImpl),
        /public, max-age=31536000, immutable/u,
    );
});

test('deployment smoke rejects an immutable API document without nosniff', async () => {
    const fixture = fixtureFetch({ immutableContentTypeOptions: null });
    await assert.rejects(
        smokeCloudflareDeployment(fixture.fetchImpl),
        /X-Content-Type-Options: nosniff/u,
    );
});

test('deployment smoke rejects CORS on immutable API documents', async () => {
    const fixture = fixtureFetch({ immutableAllowOrigin: DEPLOYMENT.publicOrigin });
    await assert.rejects(
        smokeCloudflareDeployment(fixture.fetchImpl),
        /unexpectedly exposes access-control-allow-origin/u,
    );
});
