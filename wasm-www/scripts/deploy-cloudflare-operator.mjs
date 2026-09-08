import { execFile as execFileCallback } from 'node:child_process';
import { createHash } from 'node:crypto';
import { chmod, lstat, mkdir, open, readFile, realpath, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { promisify } from 'node:util';
import { verifyApiOriginReady } from './operator-api-readiness.mjs';
import {
    currentWorkerDeployment,
    proveWorkerVersion,
} from './cloudflare-worker-version.mjs';
import { validateCloudflareZoneAudit } from './cloudflare-zone-audit.mjs';
import { extractWranglerDeployVersion } from './extract-wrangler-deploy-version.mjs';
import {
    createOperatorWranglerSnapshot,
    removeOperatorWranglerSnapshot,
    stageOperatorDeploymentBundle,
    validateOperatorDeploymentBundle,
    validateOperatorDeploymentStage,
    validateOperatorWranglerSnapshot,
} from './operator-deployment-bundle.mjs';
import {
    reconcileOperatorRoutes,
    validateOperatorRouteAuthority,
} from './operator-cloudflare-routes.mjs';
import { smokeCloudflareDeployment } from './smoke-cloudflare-deployment.mjs';
import {
    DEPLOYMENT,
    verifyDeploymentConfig,
} from './verify-cloudflare-deployment.mjs';

const execFile = promisify(execFileCallback);
const NODE_VERSION = 'v24.19.0';
const WORKERS = Object.freeze({
    public: 'robinhood-public-site',
    runtime: 'robinhood-runtime-assets',
    signer: 'robinhood-identity-signer',
});

function canonical(value) {
    if (Array.isArray(value)) return value.map(canonical);
    if (value !== null && typeof value === 'object') {
        return Object.fromEntries(Object.entries(value).sort(([left], [right]) => Buffer.compare(Buffer.from(left, 'utf8'), Buffer.from(right, 'utf8')))
            .map(([key, item]) => [key, canonical(item)]));
    }
    return value;
}

function sha256(bytes) {
    return createHash('sha256').update(bytes).digest('hex');
}

function redact(message, apiToken) {
    const text = String(message);
    return typeof apiToken === 'string' && apiToken.length > 0
        ? text.replaceAll(apiToken, '[REDACTED]')
        : text;
}

function rollbackWorker(worker, current) {
    if (current === null) return null;
    return canonical({
        deployment_id: current.deploymentId,
        version_id: current.versionId,
        worker_name: worker,
    });
}

function publicHostRoutes(routes) {
    if (!Array.isArray(routes)) throw new Error('approved zone audit omits its Worker routes');
    return routes
        .filter(route => {
            if (typeof route?.pattern !== 'string') return false;
            const pattern = route.pattern.replace(/^https?:\/\//u, '');
            const host = DEPLOYMENT.publicHost;
            return pattern === host || pattern.startsWith(`${host}/`)
                || pattern.startsWith(`${host}?`) || pattern.startsWith(`${host}*`);
        })
        .map(route => ({ pattern: route.pattern, script: route.script ?? null }));
}

async function writePrivateJson(path, value) {
    await writeFile(path, `${JSON.stringify(canonical(value))}\n`, { flag: 'wx', mode: 0o600 });
    await chmod(path, 0o600);
}

async function requireExecutable(path, label) {
    const facts = await lstat(path).catch(() => undefined);
    if (facts === undefined || !facts.isFile() || facts.isSymbolicLink() || (facts.mode & 0o111) === 0) {
        throw new Error(`${label} must be a real executable file`);
    }
    return realpath(path);
}

async function runWrangler({
    wranglerPath,
    configPath,
    cwd,
    outputPath,
    dryRun,
    accountId,
    apiToken,
    execFileImpl = execFile,
}) {
    const args = ['deploy', ...(dryRun ? ['--dry-run'] : []), '--config', configPath];
    const env = { ...process.env };
    for (const key of Object.keys(env)) {
        if (key.startsWith('WRANGLER_') || key.startsWith('CLOUDFLARE_')) delete env[key];
    }
    delete env.NODE_OPTIONS;
    delete env.NODE_PATH;
    env.PATH = `${dirname(process.execPath)}:${process.env.PATH ?? '/usr/bin:/bin'}`;
    env.HOME = resolve(cwd, '.home');
    env.XDG_CACHE_HOME = resolve(cwd, '.xdg-cache');
    env.XDG_CONFIG_HOME = resolve(cwd, '.xdg-config');
    env.WRANGLER_LOG_PATH = resolve(cwd, '.wrangler-logs');
    env.WRANGLER_NO_SKILLS_UPDATE_PROMPTS = 'true';
    env.WRANGLER_SEND_METRICS = 'false';
    env.CLOUDFLARE_ACCOUNT_ID = accountId;
    env.CLOUDFLARE_API_TOKEN = apiToken;
    if (outputPath !== undefined) env.WRANGLER_OUTPUT_FILE_PATH = outputPath;
    await execFileImpl(wranglerPath, args, {
        cwd,
        encoding: 'utf8',
        env,
        maxBuffer: 16 * 1024 * 1024,
        timeout: 10 * 60 * 1000,
    });
}

async function deployWorker({
    label,
    worker,
    stage,
    evidence,
    wranglerPath,
    cwd,
    accountId,
    apiToken,
    fetchImpl,
    execFileImpl,
    proveWorkerImpl = proveWorkerVersion,
    configPath,
}) {
    const outputPath = resolve(evidence, `wrangler-${label}.ndjson`);
    await runWrangler({
        accountId,
        apiToken,
        configPath,
        cwd,
        dryRun: false,
        execFileImpl,
        outputPath,
        wranglerPath,
    });
    await chmod(outputPath, 0o600);
    const versionId = extractWranglerDeployVersion(await readFile(outputPath, 'utf8'), worker);
    const proof = await proveWorkerImpl({ accountId, apiToken, fetchImpl, versionId, worker });
    return {
        deployment_id: proof.current.deploymentId,
        version_id: versionId,
        worker_name: worker,
    };
}

function observingFetch(fetchImpl, transcript) {
    return async (url, options) => {
        const response = await fetchImpl(url, options);
        transcript.push(canonical({
            headers: Object.fromEntries([...response.headers.entries()].sort(([left], [right]) => left.localeCompare(right))),
            method: options?.method ?? 'GET',
            status: response.status,
            url: String(url),
        }));
        return response;
    };
}

async function runOperatorDeploymentImpl({
    bundle,
    expectedManifestSha256,
    stage,
    evidence,
    zoneAuditPath,
    expectedZoneAuditSha256,
    repoRoot,
    execute,
    accountId,
    zoneId,
    apiToken,
    fetchImpl = fetch,
    execFileImpl = execFile,
    processNodeVersion = process.version,
    stageImpl = stageOperatorDeploymentBundle,
    validateBundleImpl = validateOperatorDeploymentBundle,
    validateStageImpl = validateOperatorDeploymentStage,
    verifyConfigImpl = verifyDeploymentConfig,
    smokeImpl = smokeCloudflareDeployment,
    validateZoneAuditImpl = validateCloudflareZoneAudit,
    apiReadinessImpl = verifyApiOriginReady,
    proveWorkerImpl = proveWorkerVersion,
    currentWorkerImpl = currentWorkerDeployment,
    reconcileRoutesImpl = reconcileOperatorRoutes,
    createWranglerSnapshotImpl = createOperatorWranglerSnapshot,
    removeWranglerSnapshotImpl = removeOperatorWranglerSnapshot,
    validateWranglerSnapshotImpl = validateOperatorWranglerSnapshot,
}) {
    if (process.version !== NODE_VERSION || processNodeVersion !== NODE_VERSION) {
        throw new Error(`operator deployment requires exact Node.js ${NODE_VERSION}, got ${process.version} (asserted ${processNodeVersion})`);
    }
    const wasmRoot = resolve(repoRoot, 'wasm-www');
    const routesPath = resolve(wasmRoot, 'deploy/public-routes.json');
    const wranglerPath = await requireExecutable(resolve(wasmRoot, 'node_modules/.bin/wrangler'), 'pinned Wrangler');
    const wranglerVersion = await execFileImpl(wranglerPath, ['--version'], {
        cwd: wasmRoot,
        encoding: 'utf8',
        timeout: 30_000,
    });
    if (wranglerVersion.stdout.trim() !== DEPLOYMENT.wranglerVersion) {
        throw new Error(`operator deployment requires exact Wrangler ${DEPLOYMENT.wranglerVersion}`);
    }
    await verifyConfigImpl(wasmRoot);
    const verified = await validateBundleImpl({
        bundle,
        expectedManifestSha256,
        repoRoot,
        routesPath,
    });
    const routes = verified.routes;
    if (!Array.isArray(routes)) throw new Error('verified bundle omits its frozen route authority');
    validateOperatorRouteAuthority({
        datadirWorker: verified.manifest.datadir.worker_name,
        expectedRoutes: routes,
        publicHost: DEPLOYMENT.publicHost,
        publicWorker: WORKERS.public,
        runtimeWorker: WORKERS.runtime,
    });
    const staged = await stageImpl({
        bundle,
        deploymentConfigDirectory: resolve(wasmRoot, 'deploy'),
        expectedManifestSha256,
        repoRoot,
        routesPath,
        stage,
    });
    const stagedFacts = await lstat(staged.stage, { bigint: true });
    if (!stagedFacts.isDirectory() || stagedFacts.isSymbolicLink()) throw new Error('operator deployment stage is not a real directory');
    const stageHandle = await open(staged.stage, 'r');
    const openedStage = await stageHandle.stat({ bigint: true });
    if (openedStage.dev !== stagedFacts.dev || openedStage.ino !== stagedFacts.ino) {
        await stageHandle.close();
        throw new Error('operator deployment stage changed while it was pinned');
    }
    const pinnedStage = `/proc/self/fd/${stageHandle.fd}/.`;
    try {
    const validateExactStage = (phase, origin) => validateStageImpl({
        deploymentConfigDirectory: resolve(wasmRoot, 'deploy'),
        expectedManifestSha256,
        origin,
        phase,
        repoRoot,
        routesPath,
        stage: pinnedStage,
    });
    const withWranglerSnapshot = async (label, action) => {
        const authority = staged.wranglerSnapshotAuthorities?.[label];
        if (authority === undefined) throw new Error(`staged deployment omits Wrangler ${label} snapshot authority`);
        const snapshot = await createWranglerSnapshotImpl({ authority, label, stage: pinnedStage });
        const failures = [];
        let result;
        try {
            await validateWranglerSnapshotImpl({ authority, label, snapshot: snapshot.sealedPath });
            await validateExactStage('snapshot-pre', label);
            try {
                result = await action(snapshot);
            } catch (error) {
                failures.push(error);
            }
            try {
                await validateWranglerSnapshotImpl({ authority, label, snapshot: snapshot.sealedPath });
                await validateExactStage('snapshot-post', label);
            } catch (error) {
                failures.push(error);
            }
        } catch (error) {
            failures.push(error);
        } finally {
            try {
                await removeWranglerSnapshotImpl(snapshot);
            } catch (error) {
                failures.push(error);
            }
        }
        if (failures.length === 1) throw failures[0];
        if (failures.length > 1) throw new AggregateError(failures, `Wrangler ${label} invocation and authority validation failed`);
        return result;
    };
    await validateExactStage('post-stage');
    for (const label of ['runtime', 'signer', 'public']) {
        await withWranglerSnapshot(label, snapshot => runWrangler({
            accountId,
            apiToken,
            configPath: snapshot.configPath,
            cwd: snapshot.cwd,
            dryRun: true,
            execFileImpl,
            wranglerPath,
        }));
    }
    const zoneAudit = await validateZoneAuditImpl({
        accountId,
        apiToken,
        auditPath: zoneAuditPath,
        expectedAuditSha256: expectedZoneAuditSha256,
        fetchImpl,
        zoneId,
    });
    await apiReadinessImpl({ publicOrigin: DEPLOYMENT.publicOrigin, fetchImpl });
    await proveWorkerImpl({
        accountId,
        apiToken,
        fetchImpl,
        versionId: verified.manifest.datadir.worker_version_id,
        worker: verified.manifest.datadir.worker_name,
    });
    if (!execute) {
        return {
            datadirVersionId: verified.manifest.datadir.worker_version_id,
            manifestSha256: expectedManifestSha256,
            mutated: false,
        stage: staged.stage,
            zoneAuditSha256: zoneAudit.auditSha256,
        };
    }
    if (await lstat(evidence).catch(() => undefined) !== undefined) {
        throw new Error(`private evidence output must be absent: ${evidence}`);
    }
    await mkdir(evidence, { mode: 0o700 });
    await chmod(evidence, 0o700);
    const approvedZoneAudit = canonical({
        approval: zoneAudit.approval,
        schema_version: 1,
        snapshot: zoneAudit.snapshot,
    });
    const approvedZoneAuditBytes = Buffer.from(`${JSON.stringify(approvedZoneAudit)}\n`);
    if (createHash('sha256').update(approvedZoneAuditBytes).digest('hex') !== zoneAudit.auditSha256) {
        throw new Error('validated zone audit object differs from its approved byte authority');
    }
    await writeFile(resolve(evidence, 'approved-zone-audit.json'), approvedZoneAuditBytes, { flag: 'wx', mode: 0o600 });
    await chmod(resolve(evidence, 'approved-zone-audit.json'), 0o600);
    const previousRoutes = publicHostRoutes(zoneAudit.snapshot.worker_routes);
    const rollbackWorkers = {};
    for (const [label, worker] of Object.entries(WORKERS)) {
        rollbackWorkers[label] = rollbackWorker(worker, await currentWorkerImpl({
            accountId,
            allowAbsent: true,
            apiToken,
            fetchImpl,
            worker,
        }));
    }
    rollbackWorkers.datadir = rollbackWorker(verified.manifest.datadir.worker_name, await currentWorkerImpl({
        accountId,
        apiToken,
        fetchImpl,
        worker: verified.manifest.datadir.worker_name,
    }));
    const fixedWorkers = new Set([
        ...Object.values(WORKERS),
        verified.manifest.datadir.worker_name,
    ]);
    const routeWorkerNames = [...new Set(previousRoutes
        .map(route => route.script)
        .filter(worker => worker !== null && !fixedWorkers.has(worker)))]
        .sort((left, right) => Buffer.compare(Buffer.from(left, 'utf8'), Buffer.from(right, 'utf8')));
    const routeWorkers = {};
    for (const worker of routeWorkerNames) {
        const current = await currentWorkerImpl({ accountId, apiToken, fetchImpl, worker });
        if (current === null) throw new Error(`approved previous route names absent Worker ${worker}`);
        routeWorkers[worker] = rollbackWorker(worker, current);
    }
    const rollback = canonical({
        account_id: accountId,
        deployment_manifest_sha256: expectedManifestSha256,
        previous_routes: previousRoutes,
        public_host: DEPLOYMENT.publicHost,
        route_workers: routeWorkers,
        schema_version: 2,
        source_commit: verified.manifest.source_commit,
        workers: rollbackWorkers,
        zone_audit_sha256: zoneAudit.auditSha256,
        zone_id: zoneId,
    });
    const rollbackBytes = Buffer.from(`${JSON.stringify(rollback)}\n`);
    const rollbackSha256 = sha256(rollbackBytes);
    await writeFile(resolve(evidence, 'rollback.json'), rollbackBytes, { flag: 'wx', mode: 0o600 });
    await chmod(resolve(evidence, 'rollback.json'), 0o600);

    const assertCapturedWorker = async (worker, captured, label) => {
        const current = await currentWorkerImpl({
            accountId,
            allowAbsent: true,
            apiToken,
            fetchImpl,
            worker,
        });
        const observed = rollbackWorker(worker, current);
        if (JSON.stringify(observed) !== JSON.stringify(captured)) {
            throw new Error(`${label} Worker changed after rollback capture`);
        }
    };

    const deployed = {};
    // The post-stage proof is not a lease. Revalidate the complete sealed
    // stage immediately before the first live upload, then again for the
    // exact origin consumed by each Wrangler invocation.
    await validateExactStage('pre-live');
    for (const label of ['runtime', 'signer', 'public']) {
        await validateExactStage('origin', label);
        deployed[label] = await withWranglerSnapshot(label, async snapshot => {
            await assertCapturedWorker(WORKERS[label], rollbackWorkers[label], label);
            return deployWorker({
                accountId,
                apiToken,
                configPath: snapshot.configPath,
                cwd: snapshot.cwd,
                evidence,
                execFileImpl,
                fetchImpl,
                label,
                proveWorkerImpl,
                stage: staged.stage,
                worker: WORKERS[label],
                wranglerPath,
            });
        });
    }
    const assertDeployedWorker = async label => {
        const identity = deployed[label];
        const proof = await proveWorkerImpl({
            accountId,
            apiToken,
            fetchImpl,
            versionId: identity.version_id,
            worker: identity.worker_name,
        });
        if (proof.current?.deploymentId !== identity.deployment_id
            || proof.current?.versionId !== identity.version_id) {
            throw new Error(`${label} Worker changed after its exact deployment`);
        }
    };
    const assertDatadirWorker = async () => {
        const proof = await proveWorkerImpl({
            accountId,
            apiToken,
            fetchImpl,
            versionId: rollbackWorkers.datadir.version_id,
            worker: rollbackWorkers.datadir.worker_name,
        });
        if (proof.current?.deploymentId !== rollbackWorkers.datadir.deployment_id
            || proof.current?.versionId !== rollbackWorkers.datadir.version_id) {
            throw new Error('datadir Worker changed after rollback capture');
        }
    };
    // Re-prove the complete deployed Worker closure immediately before the
    // first route mutation; an earlier proof is not a lease.
    for (const label of ['runtime', 'signer', 'public']) await assertDeployedWorker(label);
    await assertDatadirWorker();
    for (const [worker, captured] of Object.entries(routeWorkers)) {
        await assertCapturedWorker(worker, captured, `previous route ${worker}`);
    }
    await reconcileRoutesImpl({
        apiToken,
        apply: true,
        approvedSnapshotRoutes: zoneAudit.snapshot.worker_routes,
        expectedRoutes: routes,
        fetchImpl,
        publicHost: DEPLOYMENT.publicHost,
        retirePatterns: zoneAudit.approval.retire_public_routes,
        zoneId,
    });
    const transcript = [];
    await smokeImpl(observingFetch(fetchImpl, transcript));
    // Smoke is not a lease. Close every Worker and route identity again before
    // signing the terminal deployment evidence.
    for (const label of ['runtime', 'signer', 'public']) await assertDeployedWorker(label);
    await assertDatadirWorker();
    for (const [worker, captured] of Object.entries(routeWorkers)) {
        await assertCapturedWorker(worker, captured, `previous route ${worker}`);
    }
    await reconcileRoutesImpl({
        apiToken,
        apply: false,
        approvedSnapshotRoutes: routes,
        expectedRoutes: routes,
        fetchImpl,
        publicHost: DEPLOYMENT.publicHost,
        retirePatterns: [],
        zoneId,
    });
    await writePrivateJson(resolve(evidence, 'live-smoke.json'), { responses: transcript, schema_version: 1 });
    const receipt = canonical({
        account_id: accountId,
        datadir: verified.manifest.datadir,
        deployed,
        deployment_manifest_sha256: expectedManifestSha256,
        finished_at: new Date().toISOString(),
        materialization_receipt_sha256: verified.manifest.materialization.approved_receipt_sha256,
        publication_lock_sha256: verified.manifest.publication_lock_sha256,
        rollback_evidence_sha256: rollbackSha256,
        // Route reconciliation proved this complete authority order-
        // independently. Persist the bundle's canonical order, not the
        // Cloudflare API's ambient response order.
        routes,
        schema_version: 2,
        source_commit: verified.manifest.source_commit,
        zone_audit_sha256: zoneAudit.auditSha256,
        zone_id: zoneId,
    });
    await writePrivateJson(resolve(evidence, 'deployment.json'), receipt);
    return {
        deploymentEvidenceSha256: sha256(Buffer.from(`${JSON.stringify(receipt)}\n`)),
        evidence,
        manifestSha256: expectedManifestSha256,
        mutated: true,
        receipt,
        rollbackEvidenceSha256: rollbackSha256,
    };
    } finally {
        await stageHandle.close();
    }
}

export async function runOperatorDeployment(options) {
    try {
        return await runOperatorDeploymentImpl(options);
    } catch (error) {
        const apiToken = options?.apiToken;
        const message = error instanceof Error ? error.message : String(error);
        if (typeof apiToken !== 'string' || apiToken.length === 0 || !message.includes(apiToken)) throw error;
        if (error instanceof AggregateError) {
            throw new AggregateError(
                error.errors.map(item => new Error(redact(
                    item instanceof Error ? item.message : String(item), apiToken,
                ))),
                redact(message, apiToken),
            );
        }
        throw new Error(redact(message, apiToken));
    }
}

function parseArguments(args) {
    const values = { execute: false };
    for (let index = 0; index < args.length; index += 1) {
        const key = args[index];
        if (key === '--execute') { values.execute = true; continue; }
        const value = args[index + 1];
        if (!['--bundle', '--manifest-sha256', '--stage', '--evidence', '--zone-audit', '--zone-audit-sha256'].includes(key)
            || value === undefined || value.startsWith('--')) throw new Error(`invalid operator deployment argument ${key}`);
        values[key.slice(2).replaceAll('-', '_')] = value;
        index += 1;
    }
    for (const key of ['bundle', 'manifest_sha256', 'stage', 'zone_audit', 'zone_audit_sha256']) {
        if (values[key] === undefined) throw new Error(`missing --${key.replaceAll('_', '-')}`);
    }
    if (values.execute && values.evidence === undefined) throw new Error('--execute requires an absent --evidence path');
    return values;
}

async function main() {
    const values = parseArguments(process.argv.slice(2));
    const repoRoot = resolve(import.meta.dirname, '../..');
    const result = await runOperatorDeployment({
        accountId: process.env.CLOUDFLARE_ACCOUNT_ID,
        apiToken: process.env.CLOUDFLARE_API_TOKEN,
        bundle: values.bundle,
        evidence: values.evidence,
        execute: values.execute,
        expectedManifestSha256: values.manifest_sha256,
        expectedZoneAuditSha256: values.zone_audit_sha256,
        repoRoot,
        stage: values.stage,
        zoneAuditPath: values.zone_audit,
        zoneId: process.env.CLOUDFLARE_ZONE_ID,
    });
    console.log(result.mutated
        ? `deployed exact Cloudflare release; private evidence: ${result.evidence}; deployment SHA-256: ${result.deploymentEvidenceSha256}; rollback SHA-256: ${result.rollbackEvidenceSha256}`
        : `operator deployment preflight passed without mutation: ${result.manifestSha256}`);
}

const invokedPath = process.argv[1];
if (invokedPath !== undefined && import.meta.url === pathToFileURL(resolve(invokedPath)).href) {
    main().catch(error => {
        console.error(redact(
            error instanceof Error ? error.message : String(error),
            process.env.CLOUDFLARE_API_TOKEN,
        ));
        process.exitCode = 1;
    });
}
