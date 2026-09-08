/** Experimental probe only: not wired into deployment.
 * Serve packaged Brotli without Workers compressing it again.
 * https://developers.cloudflare.com/workers/runtime-apis/response/#constructor
 */
export function acceptsBrotli(value) {
    const weights = new Map();
    for (const entry of (value ?? '').toLowerCase().split(',')) {
        const [name, ...parameters] = entry.trim().split(';');
        const quality = parameters.map(value => value.trim()).find(value => value.startsWith('q='));
        const weight = quality === undefined ? 1 : Number(quality.slice(2));
        weights.set(name.trim(), Number.isFinite(weight) && weight >= 0 && weight <= 1 ? weight : 0);
    }
    return (weights.get('br') ?? weights.get('*') ?? 0) > 0;
}

function varyEncoding(response) {
    const outgoing = new Headers(response.headers);
    const vary = outgoing.get('Vary');
    if (!vary?.split(',').some(value => ['accept-encoding', '*'].includes(value.trim().toLowerCase()))) {
        outgoing.set('Vary', vary ? `${vary}, Accept-Encoding` : 'Accept-Encoding');
    }
    return outgoing;
}

export default {
    async fetch(request, env) {
        const url = new URL(request.url);
        const canonicalWasm = url.pathname.startsWith('/wasm/') && url.pathname.endsWith('.wasm');
        const original = async () => {
            const response = await env.ASSETS.fetch(request);
            return canonicalWasm ? new Response(response.body, { status: response.status, headers: varyEncoding(response), encodeBody: 'manual' }) : response;
        };
        // Keep the asset service's ordinary range semantics.
        if (!['GET', 'HEAD'].includes(request.method) || !canonicalWasm || request.headers.has('Range')
            || !acceptsBrotli(request.cf?.clientAcceptEncoding ?? request.headers.get('Accept-Encoding'))) {
            return original();
        }
        url.pathname += '.br';
        const headers = new Headers(request.headers);
        headers.set('Accept-Encoding', 'identity');
        const response = await env.ASSETS.fetch(new Request(url, { method: request.method, headers }));
        // Historical runtime versions may have no sidecar.
        if (response.status === 404) {
            await response.body?.cancel();
            return original();
        }
        if (response.status !== 200 && response.status !== 304) return response;
        const outgoing = varyEncoding(response);
        outgoing.set('Content-Type', 'application/wasm');
        outgoing.set('Content-Encoding', 'br');
        outgoing.set('Cache-Control', `${outgoing.get('Cache-Control') ?? 'public, max-age=0'}, no-transform`);
        return new Response(response.body, { status: response.status, headers: outgoing, encodeBody: 'manual' });
    },
};
