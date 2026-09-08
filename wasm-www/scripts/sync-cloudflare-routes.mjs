import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import {
    DEPLOYMENT,
    EXPECTED_PUBLIC_ROUTES,
    loadDeploymentSnapshot,
    validateDeploymentSnapshot,
} from './verify-cloudflare-deployment.mjs';

function normalizedScript(value) {
    return typeof value === 'string' && value.length > 0 ? value : null;
}

function routeBody(route) {
    return route.script === null
        ? { pattern: route.pattern }
        : { pattern: route.pattern, script: route.script };
}

const VERSION_ID = /^[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$/u;

async function cloudflareEnvelope(response, operation) {
    let envelope;
    try {
        envelope = await response.json();
    } catch {
        throw new Error(`Cloudflare ${operation} returned non-JSON status ${response.status}`);
    }
    if (!response.ok || envelope?.success !== true) {
        const messages = Array.isArray(envelope?.errors)
            ? envelope.errors.map(error => error?.message).filter(Boolean).join('; ')
            : '';
        throw new Error(`Cloudflare ${operation} failed (${response.status})${messages === '' ? '' : `: ${messages}`}`);
    }
    return envelope.result;
}

async function proveWorkerVersion({ accountId, apiToken, versionId, worker, label, fetchImpl = fetch }) {
    if (!/^[0-9a-f]{32}$/u.test(accountId)) {
        throw new Error('CLOUDFLARE_ACCOUNT_ID must be 32 lowercase hex characters');
    }
    if (typeof apiToken !== 'string' || apiToken.length < 20) {
        throw new Error('CLOUDFLARE_API_TOKEN is missing or malformed');
    }
    if (!VERSION_ID.test(versionId)) {
        throw new Error(`${label} version id must be a lowercase UUID`);
    }
    const workerApi = `https://api.cloudflare.com/client/v4/accounts/${accountId}/workers/scripts/${worker}`;
    const request = suffix => fetchImpl(
        `${workerApi}${suffix}`,
        {
            method: 'GET',
            headers: { authorization: `Bearer ${apiToken}` },
            redirect: 'error',
            signal: AbortSignal.timeout(30_000),
        },
    );
    const version = await cloudflareEnvelope(
        await request(`/versions/${versionId}`),
        `${label} version lookup`,
    );
    if (version === null || typeof version !== 'object' || version.id !== versionId) {
        throw new Error(`Cloudflare returned the wrong ${label} version; expected ${versionId}`);
    }
    const deploymentList = await cloudflareEnvelope(
        await request('/deployments'),
        `${label} deployment lookup`,
    );
    const current = deploymentList?.deployments?.[0];
    if (current === null || typeof current !== 'object'
        || !Array.isArray(current.versions)
        || current.versions.length !== 1
        || current.versions[0]?.version_id !== versionId
        || current.versions[0]?.percentage !== 100) {
        throw new Error(`${label} version ${versionId} is not the current 100% production deployment`);
    }
    return version;
}

export async function proveRuntimeWorkerVersion(options) {
    return proveWorkerVersion({
        ...options, worker: DEPLOYMENT.runtimeWorker, label: 'runtime Worker',
    });
}

export async function proveDatadirWorkerVersion(options) {
    return proveWorkerVersion({
        ...options, worker: DEPLOYMENT.datadirWorker, label: 'datadir Worker',
    });
}

function assertLiveRoutes(
    liveRoutes,
    requiredRoutes = EXPECTED_PUBLIC_ROUTES,
    allowedRoutes = requiredRoutes,
    repairableRoutes = [],
) {
    if (!Array.isArray(liveRoutes)) throw new Error('Cloudflare route list is not an array');
    const byPattern = new Map();
    for (const route of liveRoutes) {
        if (route === null || typeof route !== 'object' || typeof route.pattern !== 'string') continue;
        const strippedPattern = route.pattern.replace(/^https?:\/\//u, '');
        const matchesPublicHost = strippedPattern === DEPLOYMENT.publicHost
            || strippedPattern.startsWith(`${DEPLOYMENT.publicHost}/`)
            || strippedPattern.startsWith(`${DEPLOYMENT.publicHost}?`)
            || strippedPattern.startsWith(`${DEPLOYMENT.publicHost}*`);
        if (!matchesPublicHost) continue;
        if (byPattern.has(route.pattern)) throw new Error(`duplicate Cloudflare route: ${route.pattern}`);
        byPattern.set(route.pattern, normalizedScript(route.script));
    }
    const allowedPatterns = new Set(allowedRoutes.map(route => route.pattern));
    const extras = [...byPattern.keys()].filter(pattern => !allowedPatterns.has(pattern));
    if (extras.length > 0) {
        throw new Error(`unexpected routes overlap the public hostname: ${extras.sort().join(', ')}`);
    }
    const requiredPatterns = new Set(requiredRoutes.map(route => route.pattern));
    const repairablePatterns = new Set(repairableRoutes.map(route => route.pattern));
    for (const route of allowedRoutes) {
        if (!requiredPatterns.has(route.pattern) && !byPattern.has(route.pattern)) continue;
        if (!byPattern.has(route.pattern)) throw new Error(`missing Cloudflare route: ${route.pattern}`);
        const actual = byPattern.get(route.pattern);
        if (actual !== route.script) {
            if (repairablePatterns.has(route.pattern)) continue;
            throw new Error(`Cloudflare route ${route.pattern} has script ${JSON.stringify(actual)}, expected ${JSON.stringify(route.script)}`);
        }
    }
}

async function cloudflareRequest(fetchImpl, zoneId, apiToken, method, suffix, body) {
    const response = await fetchImpl(
        `https://api.cloudflare.com/client/v4/zones/${zoneId}/workers/routes${suffix}`,
        {
            method,
            headers: {
                authorization: `Bearer ${apiToken}`,
                'content-type': 'application/json',
            },
            body: body === undefined ? undefined : JSON.stringify(body),
            redirect: 'error',
            signal: AbortSignal.timeout(30_000),
        },
    );
    return cloudflareEnvelope(response, `route API ${method}`);
}

export async function reconcilePublicRoutes({
    zoneId,
    apiToken,
    apply,
    fetchImpl = fetch,
    expectedRoutes = EXPECTED_PUBLIC_ROUTES,
    allowedRoutes = expectedRoutes,
    runtimeVersionProof,
    datadirVersionProof,
}) {
    if (!/^[0-9a-f]{32}$/u.test(zoneId)) throw new Error('CLOUDFLARE_ZONE_ID must be 32 lowercase hex characters');
    if (typeof apiToken !== 'string' || apiToken.length < 20) throw new Error('CLOUDFLARE_API_TOKEN is missing or malformed');
    const attachesRuntime = expectedRoutes.some(route => route.script === DEPLOYMENT.runtimeWorker);
    if (apply && attachesRuntime) {
        if (runtimeVersionProof === undefined) {
            throw new Error('refusing to attach runtime routes without an exact runtime Worker version proof');
        }
        await proveRuntimeWorkerVersion({
            accountId: runtimeVersionProof.accountId,
            apiToken,
            versionId: runtimeVersionProof.versionId,
            fetchImpl,
        });
    }
    const attachesDatadir = expectedRoutes.some(route => route.script === DEPLOYMENT.datadirWorker);
    if (apply && attachesDatadir) {
        if (datadirVersionProof === undefined) {
            throw new Error('refusing to attach datadir routes without an exact datadir Worker version proof');
        }
        await proveDatadirWorkerVersion({
            accountId: datadirVersionProof.accountId,
            apiToken,
            versionId: datadirVersionProof.versionId,
            fetchImpl,
        });
    }
    const list = async () => cloudflareRequest(fetchImpl, zoneId, apiToken, 'GET', '', undefined);
    const before = await list();
    if (!apply) {
        assertLiveRoutes(before, expectedRoutes, allowedRoutes);
        return;
    }
    // Refuse unknown overlaps and substituted allowed routes before making a
    // partial change. Missing required routes are expected at this stage.
    assertLiveRoutes(before, [], allowedRoutes, expectedRoutes);
    const existing = new Map(before
        .filter(route => route !== null && typeof route === 'object' && typeof route.pattern === 'string')
        .map(route => [route.pattern, route]));
    // The manifest order is security-significant: establish the no-script
    // API prefix, then the exact runtime routes, before attaching or
    // updating the broad static-site route.
    for (const route of expectedRoutes) {
        const current = existing.get(route.pattern);
        if (current !== undefined && normalizedScript(current.script) === route.script) continue;
        if (current === undefined) {
            await cloudflareRequest(fetchImpl, zoneId, apiToken, 'POST', '', routeBody(route));
        } else {
            if (typeof current.id !== 'string' || !/^[0-9a-f]{32}$/u.test(current.id)) {
                throw new Error(`existing Cloudflare route ${route.pattern} has no canonical id`);
            }
            await cloudflareRequest(fetchImpl, zoneId, apiToken, 'PUT', `/${current.id}`, routeBody(route));
        }
    }
    assertLiveRoutes(await list(), expectedRoutes, allowedRoutes);
}

async function main() {
    const [mode, extra] = process.argv.slice(2);
    if (!['--prepare-api', '--prove-runtime', '--prove-datadir', '--apply', '--check'].includes(mode) || extra !== undefined) {
        throw new Error('usage: node scripts/sync-cloudflare-routes.mjs --prepare-api|--prove-runtime|--prove-datadir|--apply|--check');
    }
    const snapshot = await loadDeploymentSnapshot();
    validateDeploymentSnapshot(snapshot);
    if (mode === '--prove-runtime') {
        await proveRuntimeWorkerVersion({
            accountId: process.env.CLOUDFLARE_ACCOUNT_ID,
            apiToken: process.env.CLOUDFLARE_API_TOKEN,
            versionId: process.env.ROBINHOOD_RUNTIME_VERSION_ID,
        });
        return;
    }
    if (mode === '--prove-datadir') {
        await proveDatadirWorkerVersion({
            accountId: process.env.CLOUDFLARE_ACCOUNT_ID,
            apiToken: process.env.CLOUDFLARE_API_TOKEN,
            versionId: process.env.ROBINHOOD_DATADIR_VERSION_ID,
        });
        return;
    }
    const prepareApi = mode === '--prepare-api';
    await reconcilePublicRoutes({
        zoneId: process.env.CLOUDFLARE_ZONE_ID,
        apiToken: process.env.CLOUDFLARE_API_TOKEN,
        apply: mode !== '--check',
        expectedRoutes: prepareApi ? snapshot.routes.routes.slice(0, 1) : snapshot.routes.routes,
        // A previously attached, correctly named public route is safe during
        // a redeploy. Reject a substitution, but do not require the broad
        // route on the very first release before the Worker exists.
        allowedRoutes: snapshot.routes.routes,
        runtimeVersionProof: mode === '--apply' ? {
            accountId: process.env.CLOUDFLARE_ACCOUNT_ID,
            versionId: process.env.ROBINHOOD_RUNTIME_VERSION_ID,
        } : undefined,
        datadirVersionProof: mode === '--apply' ? {
            accountId: process.env.CLOUDFLARE_ACCOUNT_ID,
            versionId: process.env.ROBINHOOD_DATADIR_VERSION_ID,
        } : undefined,
    });
}

const invokedPath = process.argv[1];
if (invokedPath !== undefined && import.meta.url === pathToFileURL(resolve(invokedPath)).href) {
    main().catch(error => {
        console.error(error instanceof Error ? error.message : String(error));
        process.exitCode = 1;
    });
}
