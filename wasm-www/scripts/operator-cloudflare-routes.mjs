const HEX_32 = /^[0-9a-f]{32}$/u;

function normalizedScript(value) {
    return typeof value === 'string' && value.length > 0 ? value : null;
}

function routeBody(route) {
    return route.script === null
        ? { pattern: route.pattern }
        : { pattern: route.pattern, script: route.script };
}

async function envelope(response, operation) {
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
        throw new Error(`Cloudflare ${operation} failed (${response.status})${messages === '' ? '' : `: ${messages}`}`);
    }
    return body.result;
}

async function routeRequest(fetchImpl, zoneId, apiToken, method, suffix = '', body = undefined) {
    return envelope(await fetchImpl(
        `https://api.cloudflare.com/client/v4/zones/${zoneId}/workers/routes${suffix}`,
        {
            body: body === undefined ? undefined : JSON.stringify(body),
            headers: {
                authorization: `Bearer ${apiToken}`,
                'content-type': 'application/json',
            },
            method,
            redirect: 'error',
            signal: AbortSignal.timeout(30_000),
        },
    ), `route API ${method}`);
}

function matchesHost(pattern, host) {
    const stripped = pattern.replace(/^https?:\/\//u, '');
    return stripped === host
        || stripped.startsWith(`${host}/`)
        || stripped.startsWith(`${host}?`)
        || stripped.startsWith(`${host}*`);
}

function publicRoutes(routes, host) {
    if (!Array.isArray(routes)) throw new Error('Cloudflare route list is not an array');
    const result = routes.filter(route => route !== null
        && typeof route === 'object'
        && typeof route.pattern === 'string'
        && matchesHost(route.pattern, host));
    const patterns = result.map(route => route.pattern);
    if (new Set(patterns).size !== patterns.length) throw new Error('Cloudflare has duplicate public-host route patterns');
    return result;
}

function routeState(routes) {
    return routes.map(route => ({ pattern: route.pattern, script: normalizedScript(route.script) }))
        .sort((left, right) => left.pattern.localeCompare(right.pattern));
}

function assertExactRouteState(actual, expected, label) {
    if (JSON.stringify(routeState(actual)) !== JSON.stringify(routeState(expected))) {
        throw new Error(`${label} differs from its exact authority`);
    }
}

export function validateOperatorRouteAuthority({
    expectedRoutes,
    publicHost,
    publicWorker,
    runtimeWorker,
    datadirWorker,
}) {
    const expected = [
        { pattern: `${publicHost}/api*`, script: null },
        { pattern: `${publicHost}/.well-known/acme-challenge/*`, script: null },
        { pattern: `${publicHost}/wasm/*`, script: runtimeWorker },
        { pattern: `${publicHost}/datadirs/*`, script: datadirWorker },
        { pattern: `${publicHost}/*`, script: publicWorker },
    ];
    if (JSON.stringify(expectedRoutes) !== JSON.stringify(expected)) {
        throw new Error('operator routes must be the exact ordered API, WASM, datadir, and broad public authority');
    }
}

export async function reconcileOperatorRoutes({
    zoneId,
    apiToken,
    publicHost,
    expectedRoutes,
    approvedSnapshotRoutes,
    retirePatterns,
    apply,
    fetchImpl = fetch,
}) {
    if (!HEX_32.test(zoneId)) throw new Error('Cloudflare zone ID must be 32 lowercase hexadecimal characters');
    if (typeof apiToken !== 'string' || apiToken.length < 20) throw new Error('Cloudflare API token is missing or malformed');
    if (typeof publicHost !== 'string' || publicHost.length === 0) throw new Error('public host is missing');
    if (!Array.isArray(expectedRoutes) || expectedRoutes.length !== 5
        || expectedRoutes[0]?.script !== null || expectedRoutes[1]?.script !== null) {
        throw new Error('ordered operator route authority is malformed');
    }
    if (!Array.isArray(retirePatterns) || new Set(retirePatterns).size !== retirePatterns.length) {
        throw new Error('approved route retirement list is malformed');
    }
    const targetPatterns = new Set(expectedRoutes.map(route => route.pattern));
    if (retirePatterns.some(pattern => targetPatterns.has(pattern) || !matchesHost(pattern, publicHost))) {
        throw new Error('approved route retirement overlaps the target authority or another hostname');
    }
    const list = () => routeRequest(fetchImpl, zoneId, apiToken, 'GET');
    const beforeAll = await list();
    const before = publicRoutes(beforeAll, publicHost);
    assertExactRouteState(before, publicRoutes(approvedSnapshotRoutes, publicHost), 'live pre-deployment public routes');
    if (!apply) return { after: before, before, writes: [] };

    const writes = [];
    const byPattern = new Map(before.map(route => [route.pattern, route]));
    for (const route of expectedRoutes) {
        const current = byPattern.get(route.pattern);
        if (current !== undefined && normalizedScript(current.script) === route.script) continue;
        if (current === undefined) {
            await routeRequest(fetchImpl, zoneId, apiToken, 'POST', '', routeBody(route));
            writes.push({ method: 'POST', ...routeBody(route) });
        } else {
            if (!HEX_32.test(current.id)) throw new Error(`existing Cloudflare route ${route.pattern} has no canonical ID`);
            await routeRequest(fetchImpl, zoneId, apiToken, 'PUT', `/${current.id}`, routeBody(route));
            writes.push({ method: 'PUT', ...routeBody(route) });
        }
    }

    // Retire only patterns the operator reviewed in the exact pre-deployment
    // snapshot, and only after every safer/more-specific target was written.
    const afterWrites = publicRoutes(await list(), publicHost);
    const afterByPattern = new Map(afterWrites.map(route => [route.pattern, route]));
    for (const expected of expectedRoutes) {
        const actual = afterByPattern.get(expected.pattern);
        if (actual === undefined || normalizedScript(actual.script) !== expected.script) {
            throw new Error(`Cloudflare route ${expected.pattern} was not established before retirement`);
        }
    }
    for (const pattern of retirePatterns) {
        const route = afterByPattern.get(pattern);
        if (route === undefined) continue;
        if (!HEX_32.test(route.id)) throw new Error(`retired Cloudflare route ${pattern} has no canonical ID`);
        await routeRequest(fetchImpl, zoneId, apiToken, 'DELETE', `/${route.id}`);
        writes.push({ method: 'DELETE', pattern });
    }
    const after = publicRoutes(await list(), publicHost);
    assertExactRouteState(after, expectedRoutes, 'live post-deployment public routes');
    return { after, before, writes };
}

export async function establishBootstrapBypasses({
    zoneId,
    apiToken,
    publicHost,
    approvedSnapshotRoutes,
    fetchImpl = fetch,
}) {
    if (!HEX_32.test(zoneId)) throw new Error('Cloudflare zone ID must be 32 lowercase hexadecimal characters');
    if (typeof apiToken !== 'string' || apiToken.length < 20) throw new Error('Cloudflare API token is missing or malformed');
    const expected = [
        { pattern: `${publicHost}/api*`, script: null },
        { pattern: `${publicHost}/.well-known/acme-challenge/*`, script: null },
    ];
    const before = publicRoutes(await routeRequest(fetchImpl, zoneId, apiToken, 'GET'), publicHost);
    assertExactRouteState(before, publicRoutes(approvedSnapshotRoutes, publicHost), 'live bootstrap public routes');
    const existing = new Map(before.map(route => [route.pattern, route]));
    for (const bypass of expected) {
        const current = existing.get(bypass.pattern);
        if (current !== undefined) {
            if (normalizedScript(current.script) !== null) throw new Error(`bootstrap bypass ${bypass.pattern} is scripted`);
            continue;
        }
        await routeRequest(fetchImpl, zoneId, apiToken, 'POST', '', routeBody(bypass));
    }
    const after = publicRoutes(await routeRequest(fetchImpl, zoneId, apiToken, 'GET'), publicHost);
    assertExactRouteState(after, [
        ...before,
        ...expected.filter(bypass => !existing.has(bypass.pattern)),
    ], 'live bootstrap public routes after mutation');
    return { after, before };
}
