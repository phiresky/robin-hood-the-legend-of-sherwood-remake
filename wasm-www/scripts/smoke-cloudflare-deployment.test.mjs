import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import test from 'node:test';
import { DEPLOYMENT, PUBLIC_ISOLATION } from './verify-cloudflare-deployment.mjs';
import { DEMO_PATH, RETAINED_DEMO_GENERATIONS } from './verify-datadir-corpus.mjs';
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
    demoBytes = DEMO_BYTES,
    demoContentLength,
    demoManifestByteLength = DEMO_BYTES.byteLength,
    demoManifestSha256 = DEMO_SHA256,
    retainedDemoStatus = 200,
    gameIsolation = { 'cross-origin-embedder-policy': 'require-corp', 'cross-origin-opener-policy': 'same-origin' },
    leaderboardIsolation = PUBLIC_ISOLATION,
    signerIsolation = { 'cross-origin-embedder-policy': 'require-corp', 'cross-origin-resource-policy': 'same-site' },
    runtimeResourcePolicy = 'same-origin',
    // The live datadir Worker predates CORP; the smoke must not require it.
    demoResourcePolicy = null,
} = {}) {
    // `null` omits the header; `undefined` would select the default above.
    const corp = value => (value === null ? {} : { 'cross-origin-resource-policy': value });
    const paths = [];
    const requests = [];
    let metadataProbes = 0;
    const short = '1234567890ab';
    const runtimeManifest = `${JSON.stringify({
        short,
        ticketSchema: 3,
        multiplayerContent: {
            schema: 2,
            demo: {
                url: `${DEPLOYMENT.publicOrigin}/${DEMO_PATH}`,
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
            metadataProbes += 1;
            const cfCacheStatus = metadataProbes === 2
                ? apiSecondCfCacheStatus ?? apiCfCacheStatus
                : apiCfCacheStatus;
            return new Response(JSON.stringify({
                schema_version: 2,
                tick_duration: { numerator_micros: 50_000, denominator: 1 },
                boards: [],
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
                headers: {
                    'cache-control': 'public, max-age=31536000, immutable',
                    ...(url.origin === DEPLOYMENT.publicOrigin
                        ? { ...gameIsolation, 'cross-origin-resource-policy': 'same-origin' }
                        : signerIsolation),
                },
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
                    ...corp(runtimeResourcePolicy),
                },
            });
        }
        if (url.pathname === `/wasm/${short}/robin_bg.wasm`) {
            return new Response(null, {
                status: 200,
                headers: {
                    'cache-control': 'public, max-age=31536000, immutable',
                    'x-robinhood-static-origin': 'runtime-v1',
                    ...corp(runtimeResourcePolicy),
                },
            });
        }
        const retained = RETAINED_DEMO_GENERATIONS.find(generation => url.pathname === `/${generation.datadirPath}`);
        if (retained !== undefined) {
            return new Response(null, {
                status: retainedDemoStatus,
                headers: {
                    'cache-control': 'public, max-age=31536000, immutable',
                    'content-length': String(retained.datadirByteLength),
                    'x-robinhood-static-origin': 'datadir-v1',
                },
            });
        }
        if (url.pathname === `/${DEMO_PATH}`) {
            return new Response(demoBytes, {
                status: 200,
                headers: {
                    'cache-control': 'public, max-age=31536000, immutable',
                    ...(demoContentLength === undefined
                        ? {}
                        : { 'content-length': demoContentLength }),
                    'x-robinhood-static-origin': 'datadir-v1',
                    ...corp(demoResourcePolicy),
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
                    ...signerIsolation,
                },
            });
        }
        const leaderboard = url.pathname.startsWith('/leaderboards/');
        const assetPath = leaderboard ? '../assets/public.js' : './assets/public.js';
        return new Response(`<script type="module" src="${assetPath}"></script>`, {
            status: 200,
            headers: {
                'content-type': 'text/html',
                'x-content-type-options': 'nosniff',
                'x-robinhood-static-origin': 'public-v1',
                'content-security-policy': leaderboard ? "default-src 'none'; frame-ancestors 'none'" : "default-src 'none'; frame-ancestors 'self'",
                'cross-origin-resource-policy': 'same-origin',
                ...(leaderboard ? leaderboardIsolation : gameIsolation),
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
    assert(fixture.paths.includes(`GET ${DEPLOYMENT.publicOrigin}/${DEMO_PATH}`));
    assert(fixture.paths.includes(`HEAD ${DEPLOYMENT.publicOrigin}/${RETAINED_DEMO_GENERATIONS[0].datadirPath}`));
    // A deployment that drops a retained Demo generation is a failure.
    await assert.rejects(
        smokeCloudflareDeployment(fixtureFetch({ retainedDemoStatus: 404 }).fetchImpl),
        /v8-web-opus-q80\.rhdata\.zst returned 404/u,
    );
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
});

test('deployment smoke requires game isolation, an isolation-compatible signer, and same-origin static resources', async () => {
    const hostile = [
        [{ gameIsolation: { 'cross-origin-opener-policy': 'same-origin' } }, /Cross-Origin-Embedder-Policy: require-corp/u],
        [{ gameIsolation: { 'cross-origin-embedder-policy': 'credentialless', 'cross-origin-opener-policy': 'same-origin' } }, /Cross-Origin-Embedder-Policy: require-corp/u],
        [{ gameIsolation: { 'cross-origin-embedder-policy': 'require-corp' } }, /Cross-Origin-Opener-Policy: same-origin/u],
        [{ leaderboardIsolation: { 'cross-origin-opener-policy': 'same-origin' } }, /leaderboard document.*Cross-Origin-Embedder-Policy/u],
        [{ signerIsolation: { 'cross-origin-resource-policy': 'same-site' } }, /identity signer document must set Cross-Origin-Embedder-Policy/u],
        [{ signerIsolation: { 'cross-origin-embedder-policy': 'require-corp' } }, /identity signer document must set Cross-Origin-Resource-Policy: same-site/u],
        [{ runtimeResourcePolicy: null }, /runtime pointer must set cross-origin-resource-policy/u],
    ];
    for (const [options, message] of hostile) {
        await assert.rejects(smokeCloudflareDeployment(fixtureFetch(options).fetchImpl), message);
    }
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
