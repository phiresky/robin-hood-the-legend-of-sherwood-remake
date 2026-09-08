import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

export const DEPLOYMENT = Object.freeze({
    apiBase: 'https://robinhood.phiresky.xyz/api/v1',
    publicHost: 'robinhood.phiresky.xyz',
    publicOrigin: 'https://robinhood.phiresky.xyz',
    publicWorker: 'robinhood-public-site',
    datadirWorker: 'robinhood-datadir-assets',
    runtimeWorker: 'robinhood-runtime-assets',
    signerHost: 'identity.robinhood.phiresky.xyz',
    signerOrigin: 'https://identity.robinhood.phiresky.xyz',
    signerWorker: 'robinhood-identity-signer',
    wranglerVersion: '4.127.1',
    zoneName: 'phiresky.xyz',
});

export const EXPECTED_PUBLIC_ROUTES = Object.freeze([
    // Cloudflare route matching includes the query string. A literal `/api`
    // route would therefore miss `/api?query`; the trailing wildcard keeps
    // the complete API prefix on the origin and away from the public Worker.
    Object.freeze({ pattern: `${DEPLOYMENT.publicHost}/api*`, script: null }),
    // Keep the permanent HTTP-01 challenge path on nginx. This route is
    // deliberately narrower than every application/static prefix.
    Object.freeze({ pattern: `${DEPLOYMENT.publicHost}/.well-known/acme-challenge/*`, script: null }),
    Object.freeze({ pattern: `${DEPLOYMENT.publicHost}/wasm/*`, script: DEPLOYMENT.runtimeWorker }),
    Object.freeze({ pattern: `${DEPLOYMENT.publicHost}/datadirs/*`, script: DEPLOYMENT.datadirWorker }),
    Object.freeze({ pattern: `${DEPLOYMENT.publicHost}/*`, script: DEPLOYMENT.publicWorker }),
]);

function requireEqual(actual, expected, message) {
    if (actual !== expected) throw new Error(`${message}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
}

function requireExactKeys(value, expected, label) {
    if (value === null || typeof value !== 'object' || Array.isArray(value)) {
        throw new Error(`${label} must be an object`);
    }
    const actual = Object.keys(value).sort();
    const wanted = [...expected].sort();
    if (JSON.stringify(actual) !== JSON.stringify(wanted)) {
        throw new Error(`${label} has unexpected keys: ${actual.join(', ')}`);
    }
}

export function validateDeploymentSnapshot(snapshot) {
    const {
        packageJson,
        publicWrangler,
        runtimeWrangler,
        datadirWrangler,
        signerWrangler,
        routes,
        publicHeaders,
        runtimeHeaders,
        datadirHeaders,
        signerHeaders,
    } = snapshot;
    requireEqual(packageJson.devDependencies?.wrangler, DEPLOYMENT.wranglerVersion, 'Wrangler must be exactly pinned');

    requireExactKeys(publicWrangler, [
        '$schema', 'name', 'compatibility_date', 'workers_dev', 'preview_urls', 'assets',
    ], 'public Wrangler configuration');
    requireEqual(publicWrangler.name, DEPLOYMENT.publicWorker, 'public Worker name');
    requireEqual(publicWrangler.workers_dev, false, 'public workers.dev exposure');
    requireEqual(publicWrangler.preview_urls, false, 'public preview URL exposure');
    validateAssets(publicWrangler.assets, '../dist', 'public');
    if ('route' in publicWrangler || 'routes' in publicWrangler) {
        throw new Error('public Wrangler configuration must not attach the broad route before API negations');
    }

    requireExactKeys(runtimeWrangler, [
        '$schema', 'name', 'compatibility_date', 'workers_dev', 'preview_urls', 'assets',
    ], 'runtime Wrangler configuration');
    requireEqual(runtimeWrangler.name, DEPLOYMENT.runtimeWorker, 'runtime Worker name');
    requireEqual(runtimeWrangler.workers_dev, false, 'runtime workers.dev exposure');
    requireEqual(runtimeWrangler.preview_urls, false, 'runtime preview URL exposure');
    validateAssets(runtimeWrangler.assets, '../runtime-dist', 'runtime', 'none');
    if ('route' in runtimeWrangler || 'routes' in runtimeWrangler) {
        throw new Error('runtime Wrangler configuration must use only the audited zone-route manifest');
    }

    requireExactKeys(datadirWrangler, [
        '$schema', 'name', 'compatibility_date', 'workers_dev', 'preview_urls', 'assets',
    ], 'datadir Wrangler configuration');
    requireEqual(datadirWrangler.name, DEPLOYMENT.datadirWorker, 'datadir Worker name');
    requireEqual(datadirWrangler.workers_dev, false, 'datadir workers.dev exposure');
    requireEqual(datadirWrangler.preview_urls, false, 'datadir preview URL exposure');
    validateAssets(datadirWrangler.assets, '../datadir-dist', 'datadir', 'none');
    if ('route' in datadirWrangler || 'routes' in datadirWrangler) {
        throw new Error('datadir Wrangler configuration must use only the audited zone-route manifest');
    }

    requireExactKeys(signerWrangler, [
        '$schema', 'name', 'compatibility_date', 'workers_dev', 'preview_urls', 'routes', 'assets',
    ], 'signer Wrangler configuration');
    requireEqual(signerWrangler.name, DEPLOYMENT.signerWorker, 'signer Worker name');
    requireEqual(signerWrangler.workers_dev, false, 'signer workers.dev exposure');
    requireEqual(signerWrangler.preview_urls, false, 'signer preview URL exposure');
    validateAssets(signerWrangler.assets, '../signer-dist', 'signer');
    if (!Array.isArray(signerWrangler.routes) || signerWrangler.routes.length !== 1) {
        throw new Error('signer must declare exactly one Custom Domain');
    }
    requireExactKeys(signerWrangler.routes[0], ['pattern', 'custom_domain'], 'signer route');
    requireEqual(signerWrangler.routes[0].pattern, DEPLOYMENT.signerHost, 'signer Custom Domain');
    requireEqual(signerWrangler.routes[0].custom_domain, true, 'signer route type');

    requireExactKeys(routes, ['schema_version', 'zone_name', 'worker_name', 'routes'], 'public route manifest');
    requireEqual(routes.schema_version, 1, 'public route schema');
    requireEqual(routes.zone_name, DEPLOYMENT.zoneName, 'public route zone');
    requireEqual(routes.worker_name, DEPLOYMENT.publicWorker, 'public route Worker');
    if (!Array.isArray(routes.routes)
        || JSON.stringify(routes.routes) !== JSON.stringify(EXPECTED_PUBLIC_ROUTES)) {
        throw new Error('public routes must be the exact ordered API/ACME bypasses, separated asset routes, and broad site route');
    }

    validatePublicHeaders(publicHeaders);
    validateRuntimeHeaders(runtimeHeaders);
    validateDatadirHeaders(datadirHeaders);
    validateSignerHeaders(signerHeaders);
}

function validateAssets(assets, directory, label, htmlHandling = 'auto-trailing-slash') {
    requireExactKeys(assets, ['directory', 'html_handling', 'not_found_handling'], `${label} assets`);
    requireEqual(assets.directory, directory, `${label} assets directory`);
    requireEqual(assets.html_handling, htmlHandling, `${label} HTML routing`);
    requireEqual(assets.not_found_handling, '404-page', `${label} missing-asset policy`);
}

export function validatePublicHeaders(text) {
    requireHeader(text, 'Content-Security-Policy', [
        "frame-ancestors 'none'",
        `frame-src ${DEPLOYMENT.signerOrigin}`,
        "object-src 'none'",
        "base-uri 'none'",
    ], 'public');
    requireHeader(text, 'X-Frame-Options', ['DENY'], 'public');
    requireHeader(text, 'X-Content-Type-Options', ['nosniff'], 'public');
    requireHeader(text, 'Referrer-Policy', ['no-referrer'], 'public');
    requireHeader(text, 'X-Robinhood-Static-Origin', ['public-v1'], 'public');
    if (!/(?:^|;)\s*connect-src\s+'self'\s+https:\s+wss:\s*(?:;|$)/u.test(text)) {
        throw new Error('public response CSP must allow only its own assets and signed HTTPS/WSS relay origins');
    }
    const leaderboardBlock = routeBlock(text, '/leaderboards/*');
    if (!/^\s*! Content-Security-Policy\s*$/mu.test(leaderboardBlock)) {
        throw new Error('leaderboard headers must detach the broad game Content-Security-Policy');
    }
    const leaderboardCsp = leaderboardBlock.match(/^\s*Content-Security-Policy:\s*(.+)$/imu)?.[1];
    if (leaderboardCsp === undefined
        || !/(?:^|;)\s*connect-src\s+'self'\s*(?:;|$)/u.test(leaderboardCsp)
        || /(?:^|\s)(?:https:|wss:)(?:\s|;|$)/u.test(leaderboardCsp)
        || !leaderboardCsp.includes(`frame-src ${DEPLOYMENT.signerOrigin}`)
        || !leaderboardCsp.includes("frame-ancestors 'none'")) {
        throw new Error('leaderboard response CSP must be self-only except for the isolated signer frame');
    }
    requireCacheRules(text, 'public');
}

function routeBlock(text, route) {
    const escaped = route.replace(/[.*+?^${}()|[\]\\]/gu, '\\$&');
    const match = text.match(new RegExp(`^${escaped}\\s*\\r?\\n((?:^[ \\t].*(?:\\r?\\n|$))+)`, 'mu'));
    if (match?.[1] === undefined) throw new Error(`headers are missing route block ${route}`);
    return match[1];
}

export function validateSignerHeaders(text) {
    requireHeader(text, 'Content-Security-Policy', [
        `frame-ancestors ${DEPLOYMENT.publicOrigin}`,
        "default-src 'none'",
        "connect-src 'self'",
        "object-src 'none'",
    ], 'signer');
    if (/^\s*X-Frame-Options\s*:/imu.test(text)) {
        throw new Error('signer headers must not deny the one authorized parent frame');
    }
    requireHeader(text, 'X-Content-Type-Options', ['nosniff'], 'signer');
    requireHeader(text, 'Referrer-Policy', ['no-referrer'], 'signer');
    requireHeader(text, 'X-Robinhood-Static-Origin', ['signer-v1'], 'signer');
    requireCacheRules(text, 'signer');
}

export function validateRuntimeHeaders(text) {
    requireHeader(text, 'Content-Security-Policy', [
        "default-src 'none'",
        "frame-ancestors 'none'",
        'sandbox',
        "object-src 'none'",
    ], 'runtime');
    requireHeader(text, 'X-Frame-Options', ['DENY'], 'runtime');
    requireHeader(text, 'X-Content-Type-Options', ['nosniff'], 'runtime');
    requireHeader(text, 'Referrer-Policy', ['no-referrer'], 'runtime');
    requireHeader(text, 'X-Robinhood-Static-Origin', ['runtime-v1'], 'runtime');
    requireCacheRules(text, 'runtime', /^\/wasm\/\*\s*$/mu);
    if (!/^\/wasm\/latest\.json\s*$/mu.test(text)
        || !/^\/wasm\/datadir-deployment\.json\s*$/mu.test(text)
        || /^\/datadirs\/\*\s*$/mu.test(text)) {
        throw new Error('runtime headers must revalidate its mutable pointers and must not govern datadirs');
    }
}

export function validateDatadirHeaders(text) {
    requireHeader(text, 'Content-Security-Policy', [
        "default-src 'none'",
        "frame-ancestors 'none'",
        'sandbox',
        "object-src 'none'",
    ], 'datadir');
    requireHeader(text, 'X-Frame-Options', ['DENY'], 'datadir');
    requireHeader(text, 'X-Content-Type-Options', ['nosniff'], 'datadir');
    requireHeader(text, 'Referrer-Policy', ['no-referrer'], 'datadir');
    requireHeader(text, 'X-Robinhood-Static-Origin', ['datadir-v1'], 'datadir');
    if (!/^\/\*\s*$/mu.test(text)
        || !/^\/datadirs\/\*\s*$/mu.test(text)
        || !/^\s*Cache-Control:\s*public, max-age=31536000, immutable\s*$/mu.test(text)
        || /max-age=0|must-revalidate/mu.test(text)) {
        throw new Error('datadir cache policy must make the entire immutable corpus long-lived');
    }
    if (/^\/wasm\/\*\s*$/mu.test(text)) throw new Error('datadir headers must not govern wasm');
}

function requireHeader(text, name, requiredValues, label) {
    const escaped = name.replace(/[.*+?^${}()|[\]\\]/gu, '\\$&');
    const match = text.match(new RegExp(`^\\s*${escaped}:\\s*(.+)$`, 'imu'));
    if (match?.[1] === undefined) throw new Error(`${label} headers are missing ${name}`);
    for (const required of requiredValues) {
        if (!match[1].includes(required)) {
            throw new Error(`${label} ${name} is missing ${required}`);
        }
    }
}

function requireCacheRules(text, label, immutableRule = /^\/assets\/\*\s*$/mu) {
    if (!/^\/\*\s*$/mu.test(text)
        || !immutableRule.test(text)
        || !/^\s*Cache-Control:\s*public, max-age=0, must-revalidate\s*$/mu.test(text)
        || !/^\s*! Cache-Control\s*$/mu.test(text)
        || !/^\s*Cache-Control:\s*public, max-age=31536000, immutable\s*$/mu.test(text)) {
        throw new Error(`${label} headers must revalidate documents and cache only fingerprinted assets immutably`);
    }
}

export async function loadDeploymentSnapshot(root = resolve(import.meta.dirname, '..')) {
    const readJson = async path => JSON.parse(await readFile(resolve(root, path), 'utf8'));
    return {
        packageJson: await readJson('package.json'),
        publicWrangler: await readJson('deploy/wrangler-public.json'),
        runtimeWrangler: await readJson('deploy/wrangler-runtime.json'),
        datadirWrangler: await readJson('deploy/wrangler-datadir.json'),
        signerWrangler: await readJson('deploy/wrangler-signer.json'),
        routes: await readJson('deploy/public-routes.json'),
        publicHeaders: await readFile(resolve(root, 'deploy/public-headers.txt'), 'utf8'),
        runtimeHeaders: await readFile(resolve(root, 'deploy/runtime-headers.txt'), 'utf8'),
        datadirHeaders: await readFile(resolve(root, 'deploy/datadir-headers.txt'), 'utf8'),
        signerHeaders: await readFile(resolve(root, 'deploy/signer-headers.txt'), 'utf8'),
    };
}

export async function verifyDeploymentConfig(root) {
    validateDeploymentSnapshot(await loadDeploymentSnapshot(root));
}

async function main() {
    if (process.argv.length !== 2) {
        throw new Error('usage: node scripts/verify-cloudflare-deployment.mjs');
    }
    await verifyDeploymentConfig();
}

const invokedPath = process.argv[1];
if (invokedPath !== undefined && import.meta.url === pathToFileURL(resolve(invokedPath)).href) {
    main().catch(error => {
        console.error(error instanceof Error ? error.message : String(error));
        process.exitCode = 1;
    });
}
