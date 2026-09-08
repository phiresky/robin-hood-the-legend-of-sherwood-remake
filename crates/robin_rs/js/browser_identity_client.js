/* Typed cross-origin client for the dedicated browser identity signer. */

const PROTOCOL = 'robinhood.browser-identity.v1';
const SIGNER_PATH = '/identity-signer/';
const REQUEST_TIMEOUT_MS = 30_000;
const REQUEST_ID_RE = /^[0-9a-f]{32}$/u;

const OPERATIONS = new Set([
    'status',
    'public_key',
    'sign_username_update',
    'sign_submission',
    'sign_multiplayer_leaderboard_request',
    'sign_named_seat_join',
    'sign_replay_session_genesis',
    'sign_competition_run_grant_request',
    'sign_fresh_run_preflight_request',
    'sign_campaign_continuation_preflight_as_host',
    'sign_campaign_continuation_preflight_as_controller',
    'sign_campaign_continuation',
    'sign_submission_owner_status',
    'sign_deletion_request',
]);

let signerFrame = null;
let signerOrigin = null;
let signerReady = null;
let readyResolve = null;
const pending = new Map();
let listenerInstalled = false;

function validateSignerOrigin(configured, parentOrigin) {
    let url;
    try {
        url = new URL(configured);
    } catch {
        throw new Error('Browser identity signer origin is not a valid absolute URL');
    }
    const loopback = url.protocol === 'http:'
        && (url.hostname === 'localhost' || url.hostname === '127.0.0.1' || url.hostname === '[::1]');
    if (url.protocol !== 'https:' && !loopback) {
        throw new Error('Browser identity signer requires HTTPS outside loopback development');
    }
    if (url.username !== '' || url.password !== '' || url.search !== '' || url.hash !== ''
        || configured !== url.origin) {
        throw new Error('Browser identity signer must be configured as one exact origin');
    }
    if (!loopback && url.origin === parentOrigin) {
        throw new Error('Browser identity signer origin must differ from the game origin');
    }
    return url.origin;
}

function randomRequestId() {
    const bytes = crypto.getRandomValues(new Uint8Array(16));
    return Array.from(bytes, byte => byte.toString(16).padStart(2, '0')).join('');
}

function ownRecord(value) {
    if (value === null || typeof value !== 'object' || Array.isArray(value)) return null;
    const prototype = Object.getPrototypeOf(value);
    if (prototype !== Object.prototype && prototype !== null) return null;
    return value;
}

function installResponseListener() {
    if (listenerInstalled) return;
    listenerInstalled = true;
    window.addEventListener('message', event => {
        if (event.origin !== signerOrigin || event.source !== signerFrame?.contentWindow) return;
        const response = ownRecord(event.data);
        if (response === null || response.protocol !== PROTOCOL) return;
        if (response.kind === 'ready' && Object.keys(response).length === 2) {
            readyResolve?.();
            readyResolve = null;
            return;
        }
        if (typeof response.requestId !== 'string' || !REQUEST_ID_RE.test(response.requestId)) return;
        const request = pending.get(response.requestId);
        if (request === undefined) return;
        pending.delete(response.requestId);
        clearTimeout(request.timeout);
        if (response.ok === true) request.resolve(response.result);
        else request.reject(new Error(
            `Secure browser identity ${String(response.error?.code ?? 'error')}: ${String(response.error?.message ?? 'request failed')}`,
        ));
    });
}

async function frame(configuredOrigin) {
    const expectedOrigin = validateSignerOrigin(configuredOrigin, window.location.origin);
    if (signerFrame !== null && signerOrigin !== expectedOrigin) {
        throw new Error('Browser identity signer origin changed after initialization');
    }
    if (signerFrame !== null) {
        await signerReady;
        return signerFrame;
    }
    signerOrigin = expectedOrigin;
    installResponseListener();
    const element = document.createElement('iframe');
    element.hidden = true;
    element.tabIndex = -1;
    element.title = 'Secure Robin Hood identity signer';
    element.referrerPolicy = 'no-referrer';
    element.sandbox.add('allow-scripts', 'allow-same-origin');
    element.src = `${expectedOrigin}${SIGNER_PATH}`;
    signerFrame = element;
    signerReady = new Promise((resolve, reject) => {
        const timeout = setTimeout(() => {
            readyResolve = null;
            reject(new Error('Secure browser identity signer did not become ready'));
        }, REQUEST_TIMEOUT_MS);
        readyResolve = () => {
            clearTimeout(timeout);
            resolve();
        };
        element.addEventListener('error', () => {
            clearTimeout(timeout);
            readyResolve = null;
            reject(new Error('Secure browser identity signer failed to load'));
        }, { once: true });
    });
    document.body.append(element);
    await signerReady;
    return element;
}

export async function robinhoodRequestLeaderboardIdentity(configuredOrigin, operation, payloadJson) {
    if (!OPERATIONS.has(operation)) {
        throw new Error(`Unsupported browser identity operation: ${String(operation)}`);
    }
    const signing = operation.startsWith('sign_');
    if (signing !== (typeof payloadJson === 'string')) {
        throw new Error(`Browser identity operation ${operation} has an invalid payload shape`);
    }
    if (typeof payloadJson === 'string' && new TextEncoder().encode(payloadJson).byteLength > 128 * 1024) {
        throw new Error('Browser identity payload exceeds 128 KiB');
    }
    const target = await frame(configuredOrigin);
    const receiver = target.contentWindow;
    if (receiver === null) throw new Error('Secure browser identity signer has no content window');
    const requestId = randomRequestId();
    const request = {
        protocol: PROTOCOL,
        requestId,
        operation,
        ...(payloadJson === undefined ? {} : { payloadJson }),
    };
    const result = await new Promise((resolve, reject) => {
        const timeout = setTimeout(() => {
            pending.delete(requestId);
            reject(new Error(`Secure browser identity ${operation} request timed out`));
        }, REQUEST_TIMEOUT_MS);
        pending.set(requestId, { resolve, reject, timeout });
        try {
            receiver.postMessage(request, signerOrigin);
        } catch (error) {
            clearTimeout(timeout);
            pending.delete(requestId);
            reject(error);
        }
    });
    return JSON.stringify(result);
}

export const leaderboardIdentityClientTestHooks = Object.freeze({
    operations: Object.freeze([...OPERATIONS]),
    validateSignerOrigin,
});
