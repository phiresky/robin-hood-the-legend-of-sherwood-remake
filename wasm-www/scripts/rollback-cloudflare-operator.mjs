import { createHash } from 'node:crypto';
import { constants as fsConstants } from 'node:fs';
import {
    chmod,
    lstat,
    mkdir,
    open,
    readFile,
    readdir,
    rename,
    unlink,
} from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { currentWorkerDeployment } from './cloudflare-worker-version.mjs';
import { requireUserOwnedCloudflareToken } from './cloudflare-zone-audit.mjs';
import { validateOperatorDeploymentBundle } from './operator-deployment-bundle.mjs';
import { smokeCloudflareDeployment } from './smoke-cloudflare-deployment.mjs';
import { DEPLOYMENT, verifyDeploymentConfig } from './verify-cloudflare-deployment.mjs';

const NODE_VERSION = 'v24.19.0';
const DIGEST = /^[0-9a-f]{64}$/u;
const UUID = /^[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$/u;
const ACCOUNT_OR_ZONE = /^[0-9a-f]{32}$/u;
const WORKER_NAME = /^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$/u;
const WORKERS = Object.freeze({
    datadir: DEPLOYMENT.datadirWorker,
    public: DEPLOYMENT.publicWorker,
    runtime: DEPLOYMENT.runtimeWorker,
    signer: DEPLOYMENT.signerWorker,
});
const WORKER_ORDER = Object.freeze(['runtime', 'signer', 'public']);
const EVENT_NAMES = Object.freeze([
    '00-authority.json',
    '10-runtime-intent.json', '11-runtime-done.json',
    '20-signer-intent.json', '21-signer-done.json',
    '30-public-intent.json', '31-public-done.json',
    '40-routes-intent.json', '41-routes-done.json',
    '50-complete.json',
]);

function utf8Order(left, right) {
    return Buffer.compare(Buffer.from(left, 'utf8'), Buffer.from(right, 'utf8'));
}

function canonical(value) {
    if (Array.isArray(value)) return value.map(canonical);
    if (value !== null && typeof value === 'object') {
        return Object.fromEntries(Object.entries(value)
            .sort(([left], [right]) => utf8Order(left, right))
            .map(([key, item]) => [key, canonical(item)]));
    }
    return value;
}

function canonicalBytes(value) {
    return Buffer.from(`${JSON.stringify(canonical(value))}\n`);
}

function sha256(bytes) {
    return createHash('sha256').update(bytes).digest('hex');
}

function exactKeys(value, keys, label) {
    if (value === null || typeof value !== 'object' || Array.isArray(value)) {
        throw new Error(`${label} must be an object`);
    }
    const actual = Object.keys(value).sort(utf8Order);
    const expected = [...keys].sort(utf8Order);
    if (JSON.stringify(actual) !== JSON.stringify(expected)) {
        throw new Error(`${label} has unexpected keys: ${actual.join(', ')}`);
    }
}

function redact(message, apiToken) {
    const text = String(message);
    return typeof apiToken === 'string' && apiToken.length > 0
        ? text.replaceAll(apiToken, '[REDACTED]')
        : text;
}

async function redactedCall(action, apiToken, label) {
    try {
        return await action();
    } catch (error) {
        throw new Error(`${label}: ${redact(error instanceof Error ? error.message : String(error), apiToken)}`);
    }
}

async function cloudflareEnvelope(response, operation, apiToken) {
    let body;
    try {
        body = await response.json();
    } catch {
        throw new Error(`Cloudflare ${operation} returned non-JSON status ${response.status}`);
    }
    if (!response.ok || body?.success !== true) {
        const messages = Array.isArray(body?.errors)
            ? body.errors.map(item => item?.message).filter(Boolean).join('; ')
            : '';
        const detail = messages === '' ? '' : `: ${redact(messages, apiToken)}`;
        throw new Error(`Cloudflare ${operation} failed (${response.status})${detail}`);
    }
    return body.result;
}

async function cloudflareRequest({
    apiToken,
    body,
    fetchImpl,
    method = 'GET',
    operation,
    url,
}) {
    let response;
    try {
        response = await fetchImpl(url, {
            body: body === undefined ? undefined : JSON.stringify(body),
            headers: {
                authorization: `Bearer ${apiToken}`,
                ...(body === undefined ? {} : { 'content-type': 'application/json' }),
            },
            method,
            redirect: 'error',
            signal: AbortSignal.timeout(30_000),
        });
    } catch (error) {
        throw new Error(`Cloudflare ${operation} request failed: ${redact(
            error instanceof Error ? error.message : String(error), apiToken,
        )}`);
    }
    return cloudflareEnvelope(response, operation, apiToken);
}

function requireMode(facts, mode, label) {
    if ((Number(facts.mode) & 0o777) !== mode) {
        throw new Error(`${label} must have mode ${mode.toString(8).padStart(4, '0')}`);
    }
}

async function readCanonicalPrivateJson(path, expectedSha256, label) {
    if (!DIGEST.test(expectedSha256)) throw new Error(`${label} expected SHA-256 is invalid`);
    const facts = await lstat(path, { bigint: true }).catch(() => undefined);
    if (facts === undefined || !facts.isFile() || facts.isSymbolicLink() || facts.nlink !== 1n) {
        throw new Error(`${label} must be one real, singly linked file`);
    }
    requireMode(facts, 0o600, label);
    if (facts.size > 1024n * 1024n) throw new Error(`${label} is unreasonably large`);
    const handle = await open(path, fsConstants.O_RDONLY | fsConstants.O_NOFOLLOW);
    const opened = await handle.stat({ bigint: true });
    if (opened.dev !== facts.dev || opened.ino !== facts.ino) {
        await handle.close();
        throw new Error(`${label} changed while it was pinned`);
    }
    let bytes;
    try {
        bytes = await handle.readFile();
        const after = await handle.stat({ bigint: true });
        if (after.dev !== opened.dev || after.ino !== opened.ino || after.size !== opened.size
            || after.mode !== opened.mode || after.nlink !== opened.nlink) {
            throw new Error(`${label} changed while it was read`);
        }
    } finally {
        await handle.close();
    }
    if (sha256(bytes) !== expectedSha256) throw new Error(`${label} SHA-256 mismatch`);
    let value;
    try {
        value = JSON.parse(bytes.toString('utf8'));
    } catch (error) {
        throw new Error(`${label} is not JSON: ${error instanceof Error ? error.message : String(error)}`);
    }
    if (!bytes.equals(canonicalBytes(value))) throw new Error(`${label} must be canonical JSON`);
    return value;
}

async function openEvidenceDirectory(evidence) {
    const facts = await lstat(evidence, { bigint: true }).catch(() => undefined);
    if (facts === undefined || !facts.isDirectory() || facts.isSymbolicLink()) {
        throw new Error('deployment evidence must be a real directory');
    }
    requireMode(facts, 0o700, 'deployment evidence directory');
    const handle = await open(evidence, fsConstants.O_RDONLY | fsConstants.O_DIRECTORY | fsConstants.O_NOFOLLOW);
    const opened = await handle.stat({ bigint: true });
    if (opened.dev !== facts.dev || opened.ino !== facts.ino) {
        await handle.close();
        throw new Error('deployment evidence directory changed while it was pinned');
    }
    return handle;
}

function workerIdentity(value, label, { allowAbsent = false } = {}) {
    return namedWorkerIdentity(value, WORKERS[label], `${label} Worker identity`, { allowAbsent });
}

function namedWorkerIdentity(value, expectedWorker, label, { allowAbsent = false } = {}) {
    if (allowAbsent && value === null) return null;
    exactKeys(value, ['deployment_id', 'version_id', 'worker_name'], label);
    if (!UUID.test(value.deployment_id) || !UUID.test(value.version_id)
        || value.worker_name !== expectedWorker || !WORKER_NAME.test(expectedWorker)) {
        throw new Error(`${label} is malformed or names another script`);
    }
    return value;
}

function strippedRoutePattern(pattern) {
    return pattern.replace(/^https?:\/\//u, '');
}

function matchesPublicHost(pattern) {
    const stripped = strippedRoutePattern(pattern);
    const host = DEPLOYMENT.publicHost;
    return stripped === host || stripped.startsWith(`${host}/`)
        || stripped.startsWith(`${host}?`) || stripped.startsWith(`${host}*`);
}

function validateRoutes(routes, label, { allowedScripts } = {}) {
    if (!Array.isArray(routes) || routes.length > 64) throw new Error(`${label} must be a bounded array`);
    const patterns = new Set();
    for (const route of routes) {
        exactKeys(route, ['pattern', 'script'], `${label} entry`);
        if (typeof route.pattern !== 'string' || route.pattern.length === 0
            || !(route.script === null || (typeof route.script === 'string' && WORKER_NAME.test(route.script)))) {
            throw new Error(`${label} contains a malformed route`);
        }
        if (route.script !== null && allowedScripts !== undefined && !allowedScripts.has(route.script)) {
            throw new Error(`${label} names an unauthenticated Worker script`);
        }
        if (!matchesPublicHost(route.pattern)) {
            throw new Error(`${label} contains a route for another hostname`);
        }
        if (patterns.has(route.pattern)) throw new Error(`${label} contains duplicate patterns`);
        patterns.add(route.pattern);
    }
}

async function validateRollbackEvidence({
    accountId,
    bundle,
    deploymentSha256,
    evidence,
    expectedManifestSha256,
    repoRoot,
    rollbackSha256,
    validateBundleImpl,
    verifyConfigImpl,
    zoneId,
}) {
    if (!ACCOUNT_OR_ZONE.test(accountId) || !ACCOUNT_OR_ZONE.test(zoneId)) {
        throw new Error('Cloudflare account and zone IDs must be 32 lowercase hexadecimal characters');
    }
    if (!DIGEST.test(expectedManifestSha256)) throw new Error('expected deployment manifest SHA-256 is invalid');
    await verifyConfigImpl(resolve(repoRoot, 'wasm-www'));
    const routesPath = resolve(repoRoot, 'wasm-www/deploy/public-routes.json');
    const verified = await validateBundleImpl({
        bundle,
        expectedManifestSha256,
        repoRoot,
        routesPath,
    });
    if (!Array.isArray(verified.routes)) throw new Error('verified bundle omits frozen routes');
    const evidenceHandle = await openEvidenceDirectory(evidence);
    const evidenceRoot = `/proc/self/fd/${evidenceHandle.fd}`;
    let deployment;
    let rollback;
    try {
        deployment = await readCanonicalPrivateJson(
            resolve(evidenceRoot, 'deployment.json'), deploymentSha256, 'deployment evidence',
        );
        rollback = await readCanonicalPrivateJson(
            resolve(evidenceRoot, 'rollback.json'), rollbackSha256, 'rollback evidence',
        );
    } finally {
        await evidenceHandle.close();
    }
    exactKeys(deployment, [
        'account_id', 'datadir', 'deployed', 'deployment_manifest_sha256', 'finished_at',
        'materialization_receipt_sha256', 'publication_lock_sha256',
        'rollback_evidence_sha256', 'routes', 'schema_version', 'source_commit',
        'zone_audit_sha256', 'zone_id',
    ], 'deployment evidence');
    if (deployment.schema_version !== 2
        || deployment.account_id !== accountId || deployment.zone_id !== zoneId
        || deployment.deployment_manifest_sha256 !== expectedManifestSha256
        || deployment.rollback_evidence_sha256 !== rollbackSha256
        || deployment.source_commit !== verified.manifest.source_commit
        || deployment.materialization_receipt_sha256 !== verified.manifest.materialization?.approved_receipt_sha256
        || deployment.publication_lock_sha256 !== verified.manifest.publication_lock_sha256
        || JSON.stringify(canonical(deployment.datadir)) !== JSON.stringify(canonical(verified.manifest.datadir))) {
        throw new Error('deployment evidence differs from the accepted OperatorBundleV2 authority');
    }
    if (typeof deployment.finished_at !== 'string' || Number.isNaN(Date.parse(deployment.finished_at))) {
        throw new Error('deployment evidence completion time is malformed');
    }
    exactKeys(deployment.deployed, WORKER_ORDER, 'deployed Worker identities');
    for (const label of WORKER_ORDER) workerIdentity(deployment.deployed[label], label);
    validateRoutes(deployment.routes, 'deployed route authority');
    if (JSON.stringify(deployment.routes) !== JSON.stringify(verified.routes)) {
        throw new Error('deployment evidence routes differ from OperatorBundleV2');
    }

    exactKeys(rollback, [
        'account_id', 'deployment_manifest_sha256', 'previous_routes', 'public_host',
        'route_workers', 'schema_version', 'source_commit', 'workers',
        'zone_audit_sha256', 'zone_id',
    ], 'rollback evidence');
    if (rollback.schema_version !== 2
        || rollback.account_id !== accountId || rollback.zone_id !== zoneId
        || rollback.public_host !== DEPLOYMENT.publicHost
        || rollback.deployment_manifest_sha256 !== expectedManifestSha256
        || rollback.source_commit !== deployment.source_commit
        || rollback.zone_audit_sha256 !== deployment.zone_audit_sha256) {
        throw new Error('rollback evidence identity differs from its deployment');
    }
    validateRoutes(rollback.previous_routes, 'previous route authority');
    exactKeys(rollback.workers, ['datadir', ...WORKER_ORDER], 'rollback Worker identities');
    for (const label of WORKER_ORDER) {
        if (workerIdentity(rollback.workers[label], label, { allowAbsent: true }) === null) {
            throw new Error(`cannot roll back ${label}: deployment evidence records no previous Worker version`);
        }
    }
    const datadir = workerIdentity(rollback.workers.datadir, 'datadir');
    if (datadir.version_id !== deployment.datadir.worker_version_id) {
        throw new Error('rollback evidence substitutes the immutable datadir Worker version');
    }
    if (rollback.route_workers === null || typeof rollback.route_workers !== 'object'
        || Array.isArray(rollback.route_workers)) {
        throw new Error('previous route Worker identities must be an object');
    }
    const fixedWorkers = new Set(Object.values(WORKERS));
    const expectedRouteWorkers = [...new Set(rollback.previous_routes
        .map(route => route.script)
        .filter(worker => worker !== null && !fixedWorkers.has(worker)))]
        .sort(utf8Order);
    if (JSON.stringify(Object.keys(rollback.route_workers).sort(utf8Order))
        !== JSON.stringify(expectedRouteWorkers)) {
        throw new Error('previous route Worker identities differ from the routed scripts');
    }
    for (const worker of expectedRouteWorkers) {
        namedWorkerIdentity(
            rollback.route_workers[worker], worker, `previous route Worker ${worker} identity`,
        );
    }
    return { deployment, rollback, verified };
}

async function validateRecordedDeployment({ accountId, apiToken, fetchImpl, identity }) {
    const root = `https://api.cloudflare.com/client/v4/accounts/${accountId}/workers/scripts/${identity.worker_name}`;
    const deployment = await cloudflareRequest({
        apiToken,
        fetchImpl,
        operation: `${identity.worker_name} recorded deployment lookup`,
        url: `${root}/deployments/${identity.deployment_id}`,
    });
    if (deployment?.id !== identity.deployment_id || !Array.isArray(deployment.versions)
        || deployment.versions.length !== 1
        || deployment.versions[0]?.version_id !== identity.version_id
        || deployment.versions[0]?.percentage !== 100) {
        throw new Error(`${identity.worker_name} recorded deployment does not name its exact 100% version`);
    }
    const version = await cloudflareRequest({
        apiToken,
        fetchImpl,
        operation: `${identity.worker_name} recorded version lookup`,
        url: `${root}/versions/${identity.version_id}`,
    });
    if (version?.id !== identity.version_id) {
        throw new Error(`${identity.worker_name} recorded version lookup returned another version`);
    }
}

function normalizeCurrent(value, label) {
    if (value === null) throw new Error(`${label} Worker is absent`);
    if (!UUID.test(value.deploymentId) || !UUID.test(value.versionId)) {
        throw new Error(`${label} Worker current deployment is malformed`);
    }
    return value;
}

async function readLiveWorkers({ accountId, apiToken, currentWorkerImpl, fetchImpl }) {
    const result = {};
    for (const label of [...WORKER_ORDER, 'datadir']) {
        result[label] = normalizeCurrent(await redactedCall(
            () => currentWorkerImpl({ accountId, apiToken, fetchImpl, worker: WORKERS[label] }),
            apiToken,
            `${label} current deployment lookup failed`,
        ), label);
    }
    return result;
}

async function validateLiveRouteWorkers({
    accountId, apiToken, currentWorkerImpl, fetchImpl, routeWorkers,
}) {
    for (const [worker, identity] of Object.entries(routeWorkers)) {
        const current = normalizeCurrent(await redactedCall(
            () => currentWorkerImpl({ accountId, apiToken, fetchImpl, worker }),
            apiToken,
            `previous route ${worker} current deployment lookup failed`,
        ), `previous route ${worker}`);
        if (current.deploymentId !== identity.deployment_id || current.versionId !== identity.version_id) {
            throw new Error(`previous route Worker ${worker} changed from its authenticated deployment`);
        }
    }
}

function logicalRoutes(routes) {
    return routes.map(route => ({ pattern: route.pattern, script: route.script ?? null }))
        .sort((left, right) => utf8Order(left.pattern, right.pattern));
}

function sameRoutes(left, right) {
    return JSON.stringify(logicalRoutes(left)) === JSON.stringify(logicalRoutes(right));
}

function publicRoute(value) {
    return value !== null && typeof value === 'object'
        && typeof value.pattern === 'string'
        && matchesPublicHost(value.pattern);
}

async function listPublicRoutes({ apiToken, fetchImpl, zoneId }) {
    const routes = await cloudflareRequest({
        apiToken,
        fetchImpl,
        operation: 'Worker route list',
        url: `https://api.cloudflare.com/client/v4/zones/${zoneId}/workers/routes`,
    });
    if (!Array.isArray(routes)) throw new Error('Cloudflare Worker route list is not an array');
    const result = routes.filter(publicRoute).map(route => {
        if (!ACCOUNT_OR_ZONE.test(route.id) || typeof route.pattern !== 'string'
            || !(route.script === undefined || route.script === null
                || (typeof route.script === 'string' && WORKER_NAME.test(route.script)))) {
            throw new Error('Cloudflare returned a malformed public-host Worker route');
        }
        return { id: route.id, pattern: route.pattern, script: route.script ?? null };
    });
    validateRoutes(result.map(({ pattern, script }) => ({ pattern, script })), 'live public routes');
    return result;
}

function buildRoutePlan(fromRoutes, toRoutes) {
    const from = new Map(fromRoutes.map(route => [route.pattern, route.script]));
    const to = new Map(toRoutes.map(route => [route.pattern, route.script]));
    const operations = [];
    for (const route of toRoutes) {
        if (!from.has(route.pattern)) operations.push({ method: 'POST', ...route });
        else if (from.get(route.pattern) !== route.script) operations.push({ method: 'PUT', ...route });
    }
    for (const pattern of [...from.keys()].filter(pattern => !to.has(pattern)).sort(utf8Order)) {
        operations.push({ method: 'DELETE', pattern });
    }
    return operations;
}

function routePrefixStates(fromRoutes, operations) {
    const state = new Map(fromRoutes.map(route => [route.pattern, route.script]));
    const states = [logicalRoutes(fromRoutes)];
    for (const operation of operations) {
        if (operation.method === 'DELETE') state.delete(operation.pattern);
        else state.set(operation.pattern, operation.script);
        states.push(logicalRoutes([...state].map(([pattern, script]) => ({ pattern, script }))));
    }
    return states;
}

function detectRoutePrefix(live, states) {
    const normalized = JSON.stringify(logicalRoutes(live));
    const matches = states.map((state, index) => [JSON.stringify(state), index])
        .filter(([encoded]) => encoded === normalized)
        .map(([, index]) => index);
    if (matches.length !== 1) throw new Error('live public routes are not one exact rollback transaction prefix');
    return matches[0];
}

async function mutateRoute({ apiToken, expectedBefore, fetchImpl, operation, zoneId }) {
    const live = await listPublicRoutes({ apiToken, fetchImpl, zoneId });
    if (!sameRoutes(live, expectedBefore)) {
        throw new Error(`live public routes changed before rollback ${operation.method}`);
    }
    const current = live.find(route => route.pattern === operation.pattern);
    let suffix = '';
    let body;
    if (operation.method === 'POST') {
        if (current !== undefined) throw new Error(`route ${operation.pattern} appeared before its recorded create`);
        body = operation.script === null
            ? { pattern: operation.pattern }
            : { pattern: operation.pattern, script: operation.script };
    } else {
        if (current === undefined) throw new Error(`route ${operation.pattern} disappeared before rollback mutation`);
        suffix = `/${current.id}`;
        if (operation.method === 'PUT') {
            body = operation.script === null
                ? { pattern: operation.pattern }
                : { pattern: operation.pattern, script: operation.script };
        }
    }
    await cloudflareRequest({
        apiToken,
        body,
        fetchImpl,
        method: operation.method,
        operation: `Worker route ${operation.method}`,
        url: `https://api.cloudflare.com/client/v4/zones/${zoneId}/workers/routes${suffix}`,
    });
}

async function syncDirectory(path) {
    const handle = await open(path, 'r');
    try { await handle.sync(); } finally { await handle.close(); }
}

async function readCanonicalEvent(path, label) {
    const facts = await lstat(path, { bigint: true }).catch(() => undefined);
    if (facts === undefined) return undefined;
    if (!facts.isFile() || facts.isSymbolicLink() || facts.nlink !== 1n) {
        throw new Error(`${label} must be one real, singly linked file`);
    }
    requireMode(facts, 0o600, label);
    const handle = await open(path, fsConstants.O_RDONLY | fsConstants.O_NOFOLLOW);
    const opened = await handle.stat({ bigint: true });
    if (opened.dev !== facts.dev || opened.ino !== facts.ino) {
        await handle.close();
        throw new Error(`${label} changed while it was pinned`);
    }
    let bytes;
    try {
        bytes = await handle.readFile();
        const after = await handle.stat({ bigint: true });
        if (after.dev !== opened.dev || after.ino !== opened.ino || after.size !== opened.size
            || after.mode !== opened.mode || after.nlink !== opened.nlink) {
            throw new Error(`${label} changed while it was read`);
        }
    } finally {
        await handle.close();
    }
    let value;
    try { value = JSON.parse(bytes.toString('utf8')); } catch {
        throw new Error(`${label} is not complete canonical JSON`);
    }
    if (!bytes.equals(canonicalBytes(value))) throw new Error(`${label} is not canonical JSON`);
    return value;
}

async function readPrivateEventBytes(path, label) {
    const facts = await lstat(path, { bigint: true }).catch(() => undefined);
    if (facts === undefined) return undefined;
    if (!facts.isFile() || facts.isSymbolicLink() || facts.nlink !== 1n || facts.size > 64n * 1024n) {
        throw new Error(`${label} must be one bounded, real, singly linked file`);
    }
    requireMode(facts, 0o600, label);
    const handle = await open(path, fsConstants.O_RDONLY | fsConstants.O_NOFOLLOW);
    const opened = await handle.stat({ bigint: true });
    if (opened.dev !== facts.dev || opened.ino !== facts.ino) {
        await handle.close();
        throw new Error(`${label} changed while it was pinned`);
    }
    try {
        return await handle.readFile();
    } finally {
        await handle.close();
    }
}

async function writeEvent(transaction, name, value) {
    const path = resolve(transaction, name);
    const bytes = canonicalBytes(value);
    const existing = await readCanonicalEvent(path, name);
    if (existing !== undefined) {
        if (!bytes.equals(canonicalBytes(existing))) throw new Error(`${name} differs from the resumed transaction`);
        return;
    }
    const partial = `${path}.partial`;
    const partialBytes = await readPrivateEventBytes(partial, `${name}.partial`);
    if (partialBytes !== undefined && partialBytes.equals(bytes)) {
        // The bytes and file were durable; only the final same-directory
        // rename was interrupted.
    } else {
        if (partialBytes !== undefined) {
            if (partialBytes.length > bytes.length
                || !bytes.subarray(0, partialBytes.length).equals(partialBytes)) {
                throw new Error(`${name}.partial differs from the resumed transaction`);
            }
            // A torn write is safe to replace: no Cloudflare operation starts
            // until its intent has the final durable name, while post-write
            // completions are reconstructed from the authenticated live
            // transaction prefix.
            await unlink(partial);
            await syncDirectory(transaction);
        }
        const handle = await open(partial, 'wx', 0o600);
        try {
            await handle.writeFile(bytes);
            await handle.chmod(0o600);
            await handle.sync();
        } finally {
            await handle.close();
        }
    }
    await rename(partial, path);
    await syncDirectory(transaction);
}

function authorityEvent({ accountId, deploymentSha256, rollbackSha256, zoneId }) {
    return canonical({
        account_id: accountId,
        deployment_evidence_sha256: deploymentSha256,
        rollback_evidence_sha256: rollbackSha256,
        schema_version: 1,
        zone_id: zoneId,
    });
}

function intentEvent(label, hashes) {
    return canonical({
        deployment_evidence_sha256: hashes.deploymentSha256,
        operation: label,
        rollback_evidence_sha256: hashes.rollbackSha256,
        schema_version: 1,
    });
}

async function loadJournal({ accountId, deploymentSha256, rollbackSha256, transaction, zoneId }) {
    if (transaction === undefined) return { exists: false, events: {} };
    const facts = await lstat(transaction, { bigint: true }).catch(() => undefined);
    if (facts === undefined) return { exists: false, events: {} };
    if (!facts.isDirectory() || facts.isSymbolicLink()) throw new Error('rollback transaction must be a real directory');
    requireMode(facts, 0o700, 'rollback transaction directory');
    const entries = await readdir(transaction);
    const allowed = new Set(EVENT_NAMES.flatMap(name => [name, `${name}.partial`]));
    const unexpected = entries.filter(name => !allowed.has(name));
    if (unexpected.length > 0) throw new Error(`rollback transaction has unexpected entries: ${unexpected.join(', ')}`);
    const partialNames = entries.filter(name => name.endsWith('.partial'));
    if (partialNames.length > 1) throw new Error('rollback transaction has multiple incomplete event writes');
    const events = {};
    for (const name of EVENT_NAMES) {
        const event = await readCanonicalEvent(resolve(transaction, name), name);
        if (event !== undefined) events[name] = event;
    }
    if (events['00-authority.json'] === undefined
        && Object.keys(events).some(name => name !== '00-authority.json')) {
        throw new Error('rollback transaction events have no durable authority');
    }
    if (events['00-authority.json'] !== undefined
        && JSON.stringify(events['00-authority.json']) !== JSON.stringify(authorityEvent({
            accountId, deploymentSha256, rollbackSha256, zoneId,
        }))) {
        throw new Error('rollback transaction authority differs from this invocation');
    }
    const finalNames = new Set(Object.keys(events));
    const firstMissingIndex = EVENT_NAMES.findIndex(name => !finalNames.has(name));
    const prefixLength = firstMissingIndex === -1 ? EVENT_NAMES.length : firstMissingIndex;
    if (EVENT_NAMES.slice(prefixLength + 1).some(name => finalNames.has(name))) {
        throw new Error('rollback transaction durable events are not one exact prefix');
    }
    let partial;
    if (partialNames.length === 1) {
        const finalName = partialNames[0].slice(0, -'.partial'.length);
        if (finalNames.has(finalName) || EVENT_NAMES[prefixLength] !== finalName) {
            throw new Error('rollback transaction partial event is not the exact next prefix');
        }
        partial = {
            bytes: await readPrivateEventBytes(resolve(transaction, partialNames[0]), partialNames[0]),
            name: finalName,
        };
    }
    return { exists: true, events, partial };
}

function validateJournal(events, hashes, rollback, { accountId, zoneId }) {
    let priorDone = true;
    for (let index = 0; index < WORKER_ORDER.length; index += 1) {
        const label = WORKER_ORDER[index];
        const intentName = `${(index + 1) * 10}-${label}-intent.json`;
        const doneName = `${(index + 1) * 10 + 1}-${label}-done.json`;
        const intent = events[intentName];
        const done = events[doneName];
        if (done !== undefined && intent === undefined) throw new Error(`${label} completion has no durable intent`);
        if (intent !== undefined
            && JSON.stringify(intent) !== JSON.stringify(intentEvent(`worker:${label}`, hashes))) {
            throw new Error(`${label} rollback intent is malformed`);
        }
        if (intent !== undefined && !priorDone) throw new Error(`${label} rollback intent is out of order`);
        if (done !== undefined) {
            exactKeys(done, ['active_deployment_id', 'schema_version', 'version_id', 'worker_name'], `${label} rollback completion`);
            if (done.schema_version !== 1 || done.worker_name !== WORKERS[label]
                || done.version_id !== rollback.workers[label].version_id
                || !UUID.test(done.active_deployment_id)) {
                throw new Error(`${label} rollback completion is malformed`);
            }
        }
        priorDone = done !== undefined;
    }
    const routeIntent = events['40-routes-intent.json'];
    const routeDone = events['41-routes-done.json'];
    if (routeDone !== undefined && routeIntent === undefined) throw new Error('route completion has no durable intent');
    if (routeIntent !== undefined
        && JSON.stringify(routeIntent) !== JSON.stringify(intentEvent('routes', hashes))) {
        throw new Error('route rollback intent is malformed');
    }
    if (routeIntent !== undefined && !priorDone) throw new Error('route rollback intent precedes Worker completion');
    if (routeDone !== undefined) {
        exactKeys(routeDone, ['routes', 'schema_version'], 'route rollback completion');
        if (routeDone.schema_version !== 1 || !sameRoutes(routeDone.routes, rollback.previous_routes)) {
            throw new Error('route rollback completion differs from its authority');
        }
    }
    const complete = events['50-complete.json'];
    if (complete !== undefined) {
        if (routeDone === undefined) throw new Error('terminal rollback evidence precedes route completion');
        exactKeys(complete, [
            'account_id', 'deployment_evidence_sha256', 'rollback_evidence_sha256',
            'routes', 'schema_version', 'status', 'workers', 'zone_id',
        ], 'terminal rollback evidence');
        if (complete.schema_version !== 1 || complete.status !== 'complete'
            || complete.account_id !== accountId || complete.zone_id !== zoneId
            || complete.deployment_evidence_sha256 !== hashes.deploymentSha256
            || complete.rollback_evidence_sha256 !== hashes.rollbackSha256
            || !sameRoutes(complete.routes, rollback.previous_routes)) {
            throw new Error('terminal rollback evidence differs from its transaction authority');
        }
        exactKeys(complete.workers, WORKER_ORDER, 'terminal rollback Worker identities');
        for (let index = 0; index < WORKER_ORDER.length; index += 1) {
            const label = WORKER_ORDER[index];
            const worker = complete.workers[label];
            const done = events[`${(index + 1) * 10 + 1}-${label}-done.json`];
            exactKeys(worker, ['active_deployment_id', 'version_id', 'worker_name'], `${label} terminal Worker identity`);
            if (done === undefined || worker.active_deployment_id !== done.active_deployment_id
                || worker.version_id !== done.version_id || worker.worker_name !== done.worker_name) {
                throw new Error(`${label} terminal Worker identity differs from its durable completion`);
            }
        }
    }
}

function classifyWorkers({ deployment, events, live, rollback }) {
    const completed = [];
    let sawPending = false;
    for (let index = 0; index < WORKER_ORDER.length; index += 1) {
        const label = WORKER_ORDER[index];
        const current = live[label];
        const target = deployment.deployed[label];
        const previous = rollback.workers[label];
        const done = events[`${(index + 1) * 10 + 1}-${label}-done.json`];
        const intent = events[`${(index + 1) * 10}-${label}-intent.json`];
        if (done !== undefined) {
            if (current.versionId !== done.version_id || current.deploymentId !== done.active_deployment_id) {
                throw new Error(`${label} Worker changed after its durable rollback completion`);
            }
            if (sawPending) throw new Error('live Worker rollback state is not a prefix');
            completed.push(label);
            continue;
        }
        const exactTarget = current.versionId === target.version_id
            && current.deploymentId === target.deployment_id;
        const inferredPrevious = current.versionId === previous.version_id
            && ((previous.version_id === target.version_id && exactTarget)
                || (previous.version_id !== target.version_id && intent !== undefined));
        if (inferredPrevious && !sawPending) {
            completed.push(label);
        } else if (exactTarget) {
            sawPending = true;
        } else {
            throw new Error(`${label} Worker is neither its authenticated deployed state nor one resumable rollback prefix`);
        }
    }
    const datadir = rollback.workers.datadir;
    if (live.datadir.versionId !== datadir.version_id || live.datadir.deploymentId !== datadir.deployment_id) {
        throw new Error('immutable datadir Worker changed during the normal release rollback');
    }
    return completed;
}

async function activateWorkerVersion({ accountId, apiToken, fetchImpl, identity }) {
    const result = await cloudflareRequest({
        apiToken,
        body: {
            annotations: {
                'workers/message': 'Robin Hood operator evidence rollback',
                'workers/triggered_by': 'rollback-cloudflare-operator',
            },
            strategy: 'percentage',
            versions: [{ percentage: 100, version_id: identity.version_id }],
        },
        fetchImpl,
        method: 'POST',
        operation: `${identity.worker_name} rollback deployment creation`,
        url: `https://api.cloudflare.com/client/v4/accounts/${accountId}/workers/scripts/${identity.worker_name}/deployments`,
    });
    if (!UUID.test(result?.id) || !Array.isArray(result.versions)
        || result.versions.length !== 1
        || result.versions[0]?.version_id !== identity.version_id
        || result.versions[0]?.percentage !== 100) {
        throw new Error(`${identity.worker_name} rollback returned a malformed deployment`);
    }
    return result.id;
}

function doneEvent(label, current) {
    return canonical({
        active_deployment_id: current.deploymentId,
        schema_version: 1,
        version_id: current.versionId,
        worker_name: WORKERS[label],
    });
}

export async function runOperatorRollback({
    accountId,
    apiToken,
    bundle,
    deploymentSha256,
    evidence,
    execute,
    expectedManifestSha256,
    fetchImpl = fetch,
    processNodeVersion = process.version,
    repoRoot,
    rollbackSha256,
    transaction,
    zoneId,
    currentWorkerImpl = currentWorkerDeployment,
    requireUserTokenImpl = requireUserOwnedCloudflareToken,
    smokeImpl = smokeCloudflareDeployment,
    validateBundleImpl = validateOperatorDeploymentBundle,
    verifyConfigImpl = verifyDeploymentConfig,
}) {
    if (process.version !== NODE_VERSION || processNodeVersion !== NODE_VERSION) {
        throw new Error(`operator rollback requires exact Node.js ${NODE_VERSION}, got ${process.version} (asserted ${processNodeVersion})`);
    }
    if (typeof apiToken !== 'string' || apiToken.length < 20) {
        throw new Error('Cloudflare API token is missing or malformed');
    }
    if (execute && transaction === undefined) throw new Error('--execute requires an absent or resumable --transaction directory');
    const authority = await validateRollbackEvidence({
        accountId,
        bundle,
        deploymentSha256,
        evidence,
        expectedManifestSha256,
        repoRoot,
        rollbackSha256,
        validateBundleImpl,
        verifyConfigImpl,
        zoneId,
    });
    await redactedCall(
        () => requireUserTokenImpl({ accountId, apiToken, fetchImpl }),
        apiToken,
        'Cloudflare token verification failed',
    );
    const zone = await cloudflareRequest({
        apiToken,
        fetchImpl,
        operation: 'zone identity lookup',
        url: `https://api.cloudflare.com/client/v4/zones/${zoneId}`,
    });
    if (zone?.id !== zoneId || zone?.name !== DEPLOYMENT.zoneName
        || zone?.status !== 'active' || zone?.account?.id !== accountId) {
        throw new Error(`Cloudflare zone is not the active ${DEPLOYMENT.zoneName} zone in the recorded account`);
    }
    for (const label of [...WORKER_ORDER, 'datadir']) {
        await validateRecordedDeployment({
            accountId,
            apiToken,
            fetchImpl,
            identity: authority.rollback.workers[label],
        });
    }
    for (const identity of Object.values(authority.rollback.route_workers)) {
        await validateRecordedDeployment({ accountId, apiToken, fetchImpl, identity });
    }
    const hashes = { deploymentSha256, rollbackSha256 };
    let journal = await loadJournal({ accountId, deploymentSha256, rollbackSha256, transaction, zoneId });
    if (!execute && journal.partial !== undefined) {
        throw new Error(`rollback transaction has incomplete ${journal.partial.name}; rerun the reviewed command with --execute to repair it`);
    }
    validateJournal(journal.events, hashes, authority.rollback, { accountId, zoneId });
    let live = await readLiveWorkers({ accountId, apiToken, currentWorkerImpl, fetchImpl });
    await validateLiveRouteWorkers({
        accountId,
        apiToken,
        currentWorkerImpl,
        fetchImpl,
        routeWorkers: authority.rollback.route_workers,
    });
    let completed = classifyWorkers({
        deployment: authority.deployment,
        events: journal.events,
        live,
        rollback: authority.rollback,
    });
    const operations = buildRoutePlan(authority.deployment.routes, authority.rollback.previous_routes);
    const routeStates = routePrefixStates(authority.deployment.routes, operations);
    let liveRoutes = await listPublicRoutes({ apiToken, fetchImpl, zoneId });
    const allWorkersComplete = completed.length === WORKER_ORDER.length;
    let routePrefix;
    if (!allWorkersComplete) {
        if (!sameRoutes(liveRoutes, authority.deployment.routes)
            || journal.events['40-routes-intent.json'] !== undefined) {
            throw new Error('public routes changed before the Worker rollback prefix completed');
        }
        routePrefix = 0;
    } else if (journal.events['40-routes-intent.json'] === undefined) {
        if (!sameRoutes(liveRoutes, authority.deployment.routes)) {
            throw new Error('public routes changed before durable route rollback intent');
        }
        routePrefix = 0;
    } else {
        routePrefix = detectRoutePrefix(liveRoutes, routeStates);
    }
    if (journal.events['41-routes-done.json'] !== undefined && routePrefix !== operations.length) {
        throw new Error('public routes changed after durable rollback completion');
    }
    if (!execute) {
        return canonical({
            completed_workers: completed,
            deployment_evidence_sha256: deploymentSha256,
            mutated: false,
            remaining_route_mutations: operations.length - routePrefix,
            rollback_evidence_sha256: rollbackSha256,
        });
    }
    if (!journal.exists) {
        await mkdir(transaction, { mode: 0o700 });
        await chmod(transaction, 0o700);
        await syncDirectory(dirname(transaction));
    }
    await writeEvent(transaction, '00-authority.json', authorityEvent({
        accountId, deploymentSha256, rollbackSha256, zoneId,
    }));
    journal = await loadJournal({ accountId, deploymentSha256, rollbackSha256, transaction, zoneId });

    for (let index = 0; index < WORKER_ORDER.length; index += 1) {
        const label = WORKER_ORDER[index];
        const intentName = `${(index + 1) * 10}-${label}-intent.json`;
        const doneName = `${(index + 1) * 10 + 1}-${label}-done.json`;
        if (journal.events[doneName] !== undefined) continue;
        const intent = intentEvent(`worker:${label}`, hashes);
        await writeEvent(transaction, intentName, intent);
        journal.events[intentName] = intent;
        live = await readLiveWorkers({ accountId, apiToken, currentWorkerImpl, fetchImpl });
        classifyWorkers({
            deployment: authority.deployment,
            events: journal.events,
            live,
            rollback: authority.rollback,
        });
        const previous = authority.rollback.workers[label];
        const target = authority.deployment.deployed[label];
        let current = live[label];
        if (current.versionId !== previous.version_id) {
            if (current.versionId !== target.version_id || current.deploymentId !== target.deployment_id) {
                throw new Error(`${label} Worker changed immediately before rollback mutation`);
            }
            const createdId = await activateWorkerVersion({ accountId, apiToken, fetchImpl, identity: previous });
            live = await readLiveWorkers({ accountId, apiToken, currentWorkerImpl, fetchImpl });
            current = live[label];
            if (current.versionId !== previous.version_id || current.deploymentId !== createdId) {
                throw new Error(`${label} Worker did not select the exact rollback version at 100%`);
            }
            classifyWorkers({
                deployment: authority.deployment,
                events: journal.events,
                live,
                rollback: authority.rollback,
            });
        }
        await writeEvent(transaction, doneName, doneEvent(label, current));
        journal.events[doneName] = doneEvent(label, current);
    }

    live = await readLiveWorkers({ accountId, apiToken, currentWorkerImpl, fetchImpl });
    classifyWorkers({
        deployment: authority.deployment,
        events: journal.events,
        live,
        rollback: authority.rollback,
    });
    await validateLiveRouteWorkers({
        accountId,
        apiToken,
        currentWorkerImpl,
        fetchImpl,
        routeWorkers: authority.rollback.route_workers,
    });
    await writeEvent(transaction, '40-routes-intent.json', intentEvent('routes', hashes));
    liveRoutes = await listPublicRoutes({ apiToken, fetchImpl, zoneId });
    routePrefix = detectRoutePrefix(liveRoutes, routeStates);
    for (let index = routePrefix; index < operations.length; index += 1) {
        await mutateRoute({
            apiToken,
            expectedBefore: routeStates[index],
            fetchImpl,
            operation: operations[index],
            zoneId,
        });
        liveRoutes = await listPublicRoutes({ apiToken, fetchImpl, zoneId });
        if (!sameRoutes(liveRoutes, routeStates[index + 1])) {
            throw new Error(`Cloudflare routes did not reach rollback prefix ${index + 1}`);
        }
    }
    await writeEvent(transaction, '41-routes-done.json', canonical({
        routes: authority.rollback.previous_routes,
        schema_version: 1,
    }));
    await smokeImpl(fetchImpl);
    journal = await loadJournal({ accountId, deploymentSha256, rollbackSha256, transaction, zoneId });
    validateJournal(journal.events, hashes, authority.rollback, { accountId, zoneId });
    live = await readLiveWorkers({ accountId, apiToken, currentWorkerImpl, fetchImpl });
    classifyWorkers({
        deployment: authority.deployment,
        events: journal.events,
        live,
        rollback: authority.rollback,
    });
    await validateLiveRouteWorkers({
        accountId,
        apiToken,
        currentWorkerImpl,
        fetchImpl,
        routeWorkers: authority.rollback.route_workers,
    });
    liveRoutes = await listPublicRoutes({ apiToken, fetchImpl, zoneId });
    if (!sameRoutes(liveRoutes, authority.rollback.previous_routes)) {
        throw new Error('public routes changed after rollback smoke validation');
    }
    const workers = Object.fromEntries(WORKER_ORDER.map(label => [label, canonical({
        active_deployment_id: live[label].deploymentId,
        version_id: live[label].versionId,
        worker_name: WORKERS[label],
    })]));
    const receipt = canonical({
        account_id: accountId,
        deployment_evidence_sha256: deploymentSha256,
        rollback_evidence_sha256: rollbackSha256,
        routes: authority.rollback.previous_routes,
        schema_version: 1,
        status: 'complete',
        workers,
        zone_id: zoneId,
    });
    await writeEvent(transaction, '50-complete.json', receipt);
    return {
        mutated: true,
        receipt,
        rollbackReceiptSha256: sha256(canonicalBytes(receipt)),
        transaction,
    };
}

function parseArguments(args) {
    const values = { execute: false };
    for (let index = 0; index < args.length; index += 1) {
        const key = args[index];
        if (key === '--execute') { values.execute = true; continue; }
        const value = args[index + 1];
        if (![
            '--bundle', '--manifest-sha256', '--evidence', '--deployment-sha256',
            '--rollback-sha256', '--transaction',
        ].includes(key) || value === undefined || value.startsWith('--')) {
            throw new Error(`invalid operator rollback argument ${key}`);
        }
        values[key.slice(2).replaceAll('-', '_')] = value;
        index += 1;
    }
    for (const key of ['bundle', 'manifest_sha256', 'evidence', 'deployment_sha256', 'rollback_sha256']) {
        if (values[key] === undefined) throw new Error(`missing --${key.replaceAll('_', '-')}`);
    }
    if (values.execute && values.transaction === undefined) {
        throw new Error('--execute requires an absent or resumable --transaction directory');
    }
    return values;
}

async function main() {
    const values = parseArguments(process.argv.slice(2));
    const result = await runOperatorRollback({
        accountId: process.env.CLOUDFLARE_ACCOUNT_ID,
        apiToken: process.env.CLOUDFLARE_API_TOKEN,
        bundle: values.bundle,
        deploymentSha256: values.deployment_sha256,
        evidence: values.evidence,
        execute: values.execute,
        expectedManifestSha256: values.manifest_sha256,
        repoRoot: resolve(import.meta.dirname, '../..'),
        rollbackSha256: values.rollback_sha256,
        transaction: values.transaction,
        zoneId: process.env.CLOUDFLARE_ZONE_ID,
    });
    console.log(result.mutated
        ? `rolled back exact Cloudflare release; transaction evidence: ${result.transaction}; rollback receipt SHA-256: ${result.rollbackReceiptSha256}`
        : `operator rollback preflight passed without mutation: ${result.rollback_evidence_sha256}`);
}

const invokedPath = process.argv[1];
if (invokedPath !== undefined && import.meta.url === pathToFileURL(resolve(invokedPath)).href) {
    main().catch(error => {
        console.error(redact(error instanceof Error ? error.message : String(error), process.env.CLOUDFLARE_API_TOKEN));
        process.exitCode = 1;
    });
}
