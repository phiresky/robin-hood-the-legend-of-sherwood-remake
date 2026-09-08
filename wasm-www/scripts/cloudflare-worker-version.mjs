const ACCOUNT_ID = /^[0-9a-f]{32}$/u;
const VERSION_ID = /^[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$/u;
const WORKER_NAME = /^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$/u;

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

function requireCredentials(accountId, apiToken) {
    if (!ACCOUNT_ID.test(accountId)) {
        throw new Error('Cloudflare account ID must be 32 lowercase hexadecimal characters');
    }
    if (typeof apiToken !== 'string' || apiToken.length < 20) {
        throw new Error('Cloudflare API token is missing or malformed');
    }
}

function requireWorker(worker) {
    if (!WORKER_NAME.test(worker)) throw new Error('Cloudflare Worker name is not canonical');
}

function requestOptions(apiToken) {
    return {
        headers: { authorization: `Bearer ${apiToken}` },
        method: 'GET',
        redirect: 'error',
        signal: AbortSignal.timeout(30_000),
    };
}

export async function currentWorkerDeployment({
    accountId,
    apiToken,
    worker,
    allowAbsent = false,
    fetchImpl = fetch,
}) {
    requireCredentials(accountId, apiToken);
    requireWorker(worker);
    const url = `https://api.cloudflare.com/client/v4/accounts/${accountId}/workers/scripts/${worker}/deployments`;
    const response = await fetchImpl(url, requestOptions(apiToken));
    if (allowAbsent && response.status === 404) return null;
    const result = await cloudflareEnvelope(response, `${worker} Worker deployment lookup`);
    const current = result?.deployments?.[0];
    if (current === undefined) return null;
    if (current === null || typeof current !== 'object'
        || !VERSION_ID.test(current.id)
        || !Array.isArray(current.versions)
        || current.versions.length !== 1
        || !VERSION_ID.test(current.versions[0]?.version_id)
        || current.versions[0]?.percentage !== 100) {
        throw new Error(`${worker} current deployment is not one canonical 100% Worker version`);
    }
    return {
        deploymentId: current.id,
        versionId: current.versions[0].version_id,
    };
}

export async function proveWorkerVersion({
    accountId,
    apiToken,
    worker,
    versionId,
    fetchImpl = fetch,
}) {
    requireCredentials(accountId, apiToken);
    requireWorker(worker);
    if (!VERSION_ID.test(versionId)) throw new Error('Cloudflare Worker version ID must be a lowercase UUID');
    const workerUrl = `https://api.cloudflare.com/client/v4/accounts/${accountId}/workers/scripts/${worker}`;
    const version = await cloudflareEnvelope(
        await fetchImpl(`${workerUrl}/versions/${versionId}`, requestOptions(apiToken)),
        `${worker} Worker version lookup`,
    );
    if (version === null || typeof version !== 'object' || version.id !== versionId) {
        throw new Error(`Cloudflare returned the wrong ${worker} Worker version; expected ${versionId}`);
    }
    const current = await currentWorkerDeployment({
        accountId,
        apiToken,
        worker,
        fetchImpl,
    });
    if (current?.versionId !== versionId) {
        throw new Error(`${worker} Worker version ${versionId} is not the current 100% production deployment`);
    }
    return { current, version };
}

export const CLOUDFLARE_VERSION_ID_PATTERN = VERSION_ID;
