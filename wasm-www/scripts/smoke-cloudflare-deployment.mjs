import { createHash } from 'node:crypto';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { DEPLOYMENT } from './verify-cloudflare-deployment.mjs';

const ATTACKER_ORIGIN = 'https://attacker.invalid';
const API_OPTIONS_STATUS = 405;

async function request(fetchImpl, url, expectedStatus = 200, method = 'GET', headers = undefined) {
    const response = await fetchImpl(url, {
        method,
        headers,
        cache: 'no-store',
        redirect: 'error',
        signal: AbortSignal.timeout(15_000),
    });
    if (response.status !== expectedStatus) {
        throw new Error(`${url} returned ${response.status}, expected ${expectedStatus}`);
    }
    return response;
}

function requireHeader(response, name, expected) {
    const value = response.headers.get(name);
    if (value === null || !value.includes(expected)) {
        throw new Error(`${response.url} is missing ${name}: ${expected}`);
    }
}

function requireNoHeader(response, name, label) {
    if (response.headers.has(name)) {
        throw new Error(`${label} unexpectedly exposes ${name}`);
    }
}

function requireNoCors(response, label) {
    requireNoHeader(response, 'access-control-allow-origin', label);
    requireNoHeader(response, 'access-control-allow-credentials', label);
}

function requireNoStoreApiResponse(response, label) {
    const directives = (response.headers.get('cache-control') ?? '')
        .split(',')
        .map(directive => directive.trim().toLowerCase())
        .filter(Boolean);
    if (directives.length !== 1 || directives[0] !== 'no-store') {
        throw new Error(`${label} must set Cache-Control: no-store`);
    }
    requireNoHeader(response, 'age', label);
    if ((response.headers.get('cf-cache-status') ?? '').trim().toUpperCase() === 'HIT') {
        throw new Error(`${label} was a Cloudflare cache HIT`);
    }
    requireNoCors(response, label);
}

function requireImmutableApiResponse(response, label) {
    const directives = (response.headers.get('cache-control') ?? '')
        .split(',')
        .map(directive => directive.trim().toLowerCase())
        .filter(Boolean);
    const expected = ['public', 'max-age=31536000', 'immutable'];
    if (directives.length !== expected.length
        || expected.some(directive => !directives.includes(directive))) {
        throw new Error(`${label} must set Cache-Control: public, max-age=31536000, immutable`);
    }
    if ((response.headers.get('x-content-type-options') ?? '').trim().toLowerCase() !== 'nosniff') {
        throw new Error(`${label} must set X-Content-Type-Options: nosniff`);
    }
    requireNoCors(response, label);
}

async function requireBodyIdentity(response, identity, label) {
    const expectedLength = identity?.byteLength;
    const expectedSha256 = identity?.sha256;
    if (!Number.isSafeInteger(expectedLength) || expectedLength < 0) {
        throw new Error(`${label} manifest byte length is invalid`);
    }
    if (typeof expectedSha256 !== 'string' || !/^[0-9a-f]{64}$/u.test(expectedSha256)) {
        throw new Error(`${label} manifest SHA-256 is invalid`);
    }

    // Content-Length is optional on both HEAD and GET. Treat it as an early
    // rejection signal when present, then authenticate the actual GET body.
    const declaredLength = response.headers.get('content-length');
    if (declaredLength !== null && declaredLength !== String(expectedLength)) {
        throw new Error(`${label} Content-Length differs from its manifest identity`);
    }
    if (response.body === null) throw new Error(`${label} response has no body`);

    const hash = createHash('sha256');
    const reader = response.body.getReader();
    let actualLength = 0;
    try {
        while (true) {
            const { done, value } = await reader.read();
            if (done) break;
            actualLength += value.byteLength;
            if (actualLength > expectedLength) {
                await reader.cancel(`${label} exceeds its manifest byte length`);
                throw new Error(`${label} body exceeds its manifest byte length`);
            }
            hash.update(value);
        }
    } finally {
        reader.releaseLock();
    }

    if (actualLength !== expectedLength) {
        throw new Error(`${label} body length differs from its manifest identity`);
    }
    if (hash.digest('hex') !== expectedSha256) {
        throw new Error(`${label} body SHA-256 differs from its manifest identity`);
    }
}

function publishedRulesetDigest(metadata) {
    const digest = metadata?.rulesets?.[0]?.ruleset_manifest_sha256;
    if (typeof digest !== 'string' || !/^[0-9a-f]{64}$/u.test(digest)) {
        throw new Error('leaderboard metadata has no published immutable ruleset digest');
    }
    return digest;
}

async function smokeHtml(fetchImpl, url, marker, frameAncestor) {
    const response = await request(fetchImpl, url);
    requireHeader(response, 'content-type', 'text/html');
    requireHeader(response, 'x-content-type-options', 'nosniff');
    requireHeader(response, 'x-robinhood-static-origin', marker);
    requireHeader(response, 'content-security-policy', frameAncestor);
    const html = await response.text();
    const assetReference = html.match(/<(?:script|link)\b[^>]*(?:src|href)="([^"]*assets\/[^"]+)"/iu)?.[1];
    if (assetReference === undefined) throw new Error(`${url} has no fingerprinted asset reference`);
    const asset = await request(fetchImpl, new URL(assetReference, url));
    requireHeader(asset, 'cache-control', 'immutable');
    return response;
}

export async function smokeCloudflareDeployment(fetchImpl = fetch) {
    const acmeProbe = await request(
        fetchImpl,
        `${DEPLOYMENT.publicOrigin}/.well-known/acme-challenge/robinhood-deployment-smoke-absent`,
        404,
    );
    if (acmeProbe.headers.has('x-robinhood-static-origin')) {
        throw new Error('the permanent ACME challenge bypass was served by a static Worker');
    }
    if ((acmeProbe.headers.get('cf-cache-status') ?? '').trim().toUpperCase() === 'HIT') {
        throw new Error('the permanent ACME challenge 404 was cached');
    }
    await smokeHtml(fetchImpl, `${DEPLOYMENT.publicOrigin}/`, 'public-v1', "frame-ancestors 'none'");
    await smokeHtml(
        fetchImpl,
        `${DEPLOYMENT.publicOrigin}/leaderboards/?run=deployment-smoke`,
        'public-v1',
        "frame-ancestors 'none'",
    );
    const signer = await smokeHtml(
        fetchImpl,
        `${DEPLOYMENT.signerOrigin}/identity-signer/`,
        'signer-v1',
        `frame-ancestors ${DEPLOYMENT.publicOrigin}`,
    );
    if (signer.headers.has('x-frame-options')) {
        throw new Error('identity signer response unexpectedly sets X-Frame-Options');
    }

    const latestResponse = await request(fetchImpl, `${DEPLOYMENT.publicOrigin}/wasm/latest.json`);
    requireHeader(latestResponse, 'content-type', 'application/json');
    requireHeader(latestResponse, 'x-robinhood-static-origin', 'runtime-v1');
    requireHeader(latestResponse, 'cache-control', 'must-revalidate');
    const latestText = await latestResponse.text();
    const latest = JSON.parse(latestText);
    if (!/^[0-9a-f]{12}$/u.test(latest?.short)
        || latest?.ticketSchema !== 3
        || latest?.multiplayerContent?.schema !== 2
        || latest?.multiplayerContent?.demo?.url
            !== `${DEPLOYMENT.publicOrigin}/datadirs/demo-leicester/v8-web-opus-q80.rhdata.zst`) {
        throw new Error('live runtime manifest is not the exact ticket3/content2 same-origin contract');
    }
    const versionedManifest = await request(
        fetchImpl,
        `${DEPLOYMENT.publicOrigin}/wasm/${latest.short}/manifest.json`,
    );
    requireHeader(versionedManifest, 'x-robinhood-static-origin', 'runtime-v1');
    requireHeader(versionedManifest, 'cache-control', 'immutable');
    if (await versionedManifest.text() !== latestText) {
        throw new Error('live latest runtime manifest differs from its immutable version');
    }
    const engine = await request(
        fetchImpl,
        `${DEPLOYMENT.publicOrigin}/wasm/${latest.short}/robin_bg.wasm`,
        200,
        'HEAD',
    );
    requireHeader(engine, 'x-robinhood-static-origin', 'runtime-v1');
    requireHeader(engine, 'cache-control', 'immutable');
    const demo = await request(fetchImpl, latest.multiplayerContent.demo.url);
    requireHeader(demo, 'x-robinhood-static-origin', 'datadir-v1');
    requireHeader(demo, 'cache-control', 'immutable');
    await requireBodyIdentity(
        demo,
        latest.multiplayerContent.demo,
        'live Demo object',
    );

    const metadataUrl = `${DEPLOYMENT.publicOrigin}/api/v1/leaderboard-metadata`;
    let metadata;
    for (let probe = 1; probe <= 2; probe += 1) {
        const api = await request(fetchImpl, metadataUrl);
        if (api.headers.has('x-robinhood-static-origin')) {
            throw new Error('the /api/* request was served by the public static Worker');
        }
        const label = `leaderboard metadata probe ${probe}`;
        const apiType = api.headers.get('content-type') ?? '';
        if (!apiType.includes('application/json')) {
            throw new Error(`${label} returned unexpected content type ${apiType}`);
        }
        requireNoStoreApiResponse(api, label);
        const document = await api.json();
        if (probe === 1) metadata = document;
    }

    const options = await request(fetchImpl, metadataUrl, API_OPTIONS_STATUS, 'OPTIONS', {
        Origin: ATTACKER_ORIGIN,
        'Access-Control-Request-Method': 'GET',
    });
    requireNoCors(options, 'attacker-origin API preflight');

    const rulesetDigest = publishedRulesetDigest(metadata);
    const immutable = await request(
        fetchImpl,
        `${DEPLOYMENT.publicOrigin}/api/v1/ruleset-manifests/${rulesetDigest}`,
    );
    requireHeader(immutable, 'content-type', 'application/json');
    requireImmutableApiResponse(immutable, 'published immutable ruleset manifest');
}

async function main() {
    if (process.argv.length !== 2) {
        throw new Error('usage: node scripts/smoke-cloudflare-deployment.mjs');
    }
    await smokeCloudflareDeployment();
}

const invokedPath = process.argv[1];
if (invokedPath !== undefined && import.meta.url === pathToFileURL(resolve(invokedPath)).href) {
    main().catch(error => {
        console.error(error instanceof Error ? error.message : String(error));
        process.exitCode = 1;
    });
}
