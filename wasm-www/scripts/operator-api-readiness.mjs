const ATTACKER_ORIGIN = 'https://attacker.invalid';

function requireNoCors(response, label) {
    for (const header of ['access-control-allow-origin', 'access-control-allow-credentials']) {
        if (response.headers.has(header)) throw new Error(`${label} unexpectedly exposes ${header}`);
    }
}

function requireNoStore(response, label) {
    const directives = (response.headers.get('cache-control') ?? '').split(',')
        .map(item => item.trim().toLowerCase()).filter(Boolean);
    if (directives.length !== 1 || directives[0] !== 'no-store') throw new Error(`${label} must set Cache-Control: no-store`);
    if (response.headers.has('age') || (response.headers.get('cf-cache-status') ?? '').toUpperCase() === 'HIT') {
        throw new Error(`${label} was served from a cache`);
    }
    requireNoCors(response, label);
}

async function request(fetchImpl, url, options, expectedStatus) {
    const response = await fetchImpl(url, {
        cache: 'no-store',
        redirect: 'error',
        signal: AbortSignal.timeout(15_000),
        ...options,
    });
    if (response.status !== expectedStatus) throw new Error(`${url} returned ${response.status}, expected ${expectedStatus}`);
    return response;
}

export async function verifyApiOriginReady({ publicOrigin, fetchImpl = fetch }) {
    const url = `${publicOrigin}/api/v1/leaderboard-metadata`;
    for (let probe = 1; probe <= 2; probe += 1) {
        const response = await request(fetchImpl, url, { method: 'GET' }, 200);
        if (response.headers.has('x-robinhood-static-origin')) throw new Error('API readiness probe reached a static Worker');
        if (!(response.headers.get('content-type') ?? '').includes('application/json')) {
            throw new Error(`API readiness probe ${probe} is not JSON`);
        }
        requireNoStore(response, `API readiness probe ${probe}`);
        await response.json();
    }
    const options = await request(fetchImpl, url, {
        headers: {
            Origin: ATTACKER_ORIGIN,
            'Access-Control-Request-Method': 'GET',
        },
        method: 'OPTIONS',
    }, 405);
    requireNoCors(options, 'attacker-origin API readiness preflight');
}
