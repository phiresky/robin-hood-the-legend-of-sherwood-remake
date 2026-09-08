const LOCAL_HTTP_HOSTS = new Set(['127.0.0.1', 'localhost', '[::1]']);
const SAME_ORIGIN_API_BASE = '/api/v1';

export const API_QUERY_KEY = 'api';

export type ApiConfigSources = {
    readonly pageUrl: string;
    readonly queryValue?: string | null | undefined;
    readonly globalValue?: string | null | undefined;
    readonly metaValue?: string | null | undefined;
    readonly buildValue?: string | null | undefined;
};

/**
 * Production always uses the same-origin `/api/v1` route which Cloudflare
 * excludes from the static Worker and forwards to the VPS. Loopback
 * development keeps explicit overrides for local integration tests.
 */
export function resolveApiBase(sources: ApiConfigSources): string {
    const page = new URL(sources.pageUrl);
    const localDevelopment = (page.protocol === 'http:' || page.protocol === 'https:')
        && LOCAL_HTTP_HOSTS.has(page.hostname);
    if (!localDevelopment) {
        if ([sources.queryValue, sources.globalValue, sources.metaValue, sources.buildValue].some(hasValue)) {
            throw new Error(
                'Production highscore API overrides are forbidden; the deployment uses same-origin /api/v1.',
            );
        }
        return SAME_ORIGIN_API_BASE;
    }
    const raw = [sources.queryValue, sources.globalValue, sources.metaValue, sources.buildValue]
        .find(hasValue) ?? SAME_ORIGIN_API_BASE;

    let url: URL;
    try {
        url = new URL(raw, sources.pageUrl);
    } catch {
        throw new Error('The configured highscore API is not a valid URL.');
    }
    if (url.username.length > 0 || url.password.length > 0) {
        throw new Error('The configured highscore API must not contain credentials.');
    }
    if (url.search.length > 0 || url.hash.length > 0) {
        throw new Error('The configured highscore API must not contain a query or fragment.');
    }
    const isLocalHttp = url.protocol === 'http:' && LOCAL_HTTP_HOSTS.has(url.hostname);
    if (url.protocol !== 'https:' && !isLocalHttp) {
        throw new Error('The highscore API must use HTTPS (HTTP is allowed only on localhost).');
    }
    url.pathname = url.pathname.replace(/\/+$/u, '');
    if (!url.pathname.endsWith('/api/v1')) {
        throw new Error('The configured highscore API must include the exact /api/v1 route prefix.');
    }
    return url.origin === page.origin
        ? url.pathname
        : url.toString().replace(/\/$/u, '');
}

/** Privileged signing never follows a shareable production API override. */
export function ownerWritesArePinned(sources: ApiConfigSources): boolean {
    const page = new URL(sources.pageUrl);
    const localDevelopment = (page.protocol === 'http:' || page.protocol === 'https:')
        && LOCAL_HTTP_HOSTS.has(page.hostname);
    if (localDevelopment) return true;
    return ![sources.queryValue, sources.globalValue, sources.metaValue, sources.buildValue].some(hasValue);
}

export function apiUrl(
    base: string,
    pathSegments: readonly string[],
    params: Readonly<Record<string, string | number | null | undefined>> = {},
): string {
    if (base.startsWith('/')) {
        const path = `${base.replace(/\/+$/u, '')}/${pathSegments
            .map(segment => encodeURIComponent(segment)).join('/')}`;
        const query = new URLSearchParams();
        for (const [key, value] of Object.entries(params)) {
            if (value !== null && value !== undefined && String(value).length > 0) {
                query.set(key, String(value));
            }
        }
        const encoded = query.toString();
        return encoded.length === 0 ? path : `${path}?${encoded}`;
    }
    const url = new URL(`${base}/`);
    const basePath = url.pathname.replace(/\/+$/u, '');
    url.pathname = `${basePath}/${pathSegments.map(segment => encodeURIComponent(segment)).join('/')}`;
    for (const [key, value] of Object.entries(params)) {
        if (value !== null && value !== undefined && String(value).length > 0) {
            url.searchParams.set(key, String(value));
        }
    }
    return url.toString();
}

export function apiBaseFromBrowser(): string {
    const params = new URLSearchParams(window.location.search);
    const meta = document.querySelector<HTMLMetaElement>('meta[name="robin-highscores-api"]');
    return resolveApiBase({
        pageUrl: window.location.href,
        queryValue: params.get(API_QUERY_KEY),
        globalValue: globalThis.ROBIN_HIGHSCORES_API_BASE,
        metaValue: meta?.content,
        buildValue: import.meta.env.VITE_ROBIN_HIGHSCORES_API,
    });
}

export function ownerWritesArePinnedFromBrowser(): boolean {
    const params = new URLSearchParams(window.location.search);
    const meta = document.querySelector<HTMLMetaElement>('meta[name="robin-highscores-api"]');
    return ownerWritesArePinned({
        pageUrl: window.location.href,
        queryValue: params.get(API_QUERY_KEY),
        globalValue: globalThis.ROBIN_HIGHSCORES_API_BASE,
        metaValue: meta?.content,
        buildValue: import.meta.env.VITE_ROBIN_HIGHSCORES_API,
    });
}

function hasValue(value: string | null | undefined): value is string {
    return value !== undefined && value !== null && value.trim().length > 0;
}

declare global {
    var ROBIN_HIGHSCORES_API_BASE: string | undefined;
}
