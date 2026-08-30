import {
    MULTIPLAYER_IDENTITY_PROTOCOL,
    type MultiplayerIdentityRequest,
    type MultiplayerIdentityResponse,
} from './multiplayer_identity_protocol.js';

const DEFAULT_SIGNER_URL = 'https://identity.robinhood.phiresky.xyz/identity-signer/';
const REQUEST_TIMEOUT_MS = 15_000;
const REQUEST_ID_RE = /^[0-9a-f]{32}$/;
const BASE64URL_32_RE = /^[A-Za-z0-9_-]{43}$/;
const BASE64URL_64_RE = /^[A-Za-z0-9_-]{86}$/;
const SEAT_PROOF_DOMAIN = new TextEncoder().encode('robinhood/browser-seat-proof/v1\0');

export type BrowserMultiplayerIdentity = {
    readonly publicKey: Uint8Array<ArrayBuffer>;
    readonly sign: (message: Uint8Array) => Promise<Uint8Array<ArrayBuffer>>;
};

declare global {
    var robinMultiplayerIdentity: BrowserMultiplayerIdentity | undefined;
    var robinMarkMultiplayerInvitationRedeemed: ((sessionId: string) => Promise<void>) | undefined;
}

type PendingRequest = {
    readonly resolve: (result: unknown) => void;
    readonly reject: (error: Error) => void;
    readonly timeout: ReturnType<typeof setTimeout>;
};

let signerFrame: HTMLIFrameElement | undefined;
let signerOrigin: string | undefined;
let signerReady: Promise<void> | undefined;
let resolveSignerReady: (() => void) | undefined;
let rejectSignerReady: ((error: Error) => void) | undefined;
let responseListenerInstalled = false;
const pendingRequests = new Map<string, PendingRequest>();

function configuredSignerUrl(): URL {
    const environment = (import.meta as ImportMeta & {
        readonly env?: Record<string, string | boolean | undefined>;
    }).env;
    const configured = environment?.VITE_IDENTITY_SIGNER_URL ?? DEFAULT_SIGNER_URL;
    if (typeof configured !== 'string') throw new Error('browser identity signer URL is invalid');
    const url = new URL(configured);
    if (
        url.protocol !== 'https:'
        || url.username.length !== 0
        || url.password.length !== 0
        || url.search.length !== 0
        || url.hash.length !== 0
        || !url.pathname.endsWith('/')
    ) {
        throw new Error('browser identity signer must be one canonical HTTPS directory URL');
    }
    if (url.origin === window.location.origin) {
        throw new Error('browser identity signer must use an isolated origin');
    }
    return url;
}

function installResponseListener(): void {
    if (responseListenerInstalled) return;
    responseListenerInstalled = true;
    window.addEventListener('message', event => {
        if (event.origin !== signerOrigin || event.source !== signerFrame?.contentWindow) return;
        const raw = event.data as unknown;
        if (raw === null || typeof raw !== 'object' || Array.isArray(raw)) return;
        const object = raw as Record<string, unknown>;
        if (object.protocol !== MULTIPLAYER_IDENTITY_PROTOCOL) return;
        if (object.kind === 'ready') {
            resolveSignerReady?.();
            resolveSignerReady = undefined;
            rejectSignerReady = undefined;
            return;
        }
        if (typeof object.requestId !== 'string' || !REQUEST_ID_RE.test(object.requestId)) return;
        const pending = pendingRequests.get(object.requestId);
        if (pending === undefined) return;
        pendingRequests.delete(object.requestId);
        clearTimeout(pending.timeout);
        const response = raw as MultiplayerIdentityResponse;
        if (response.ok) {
            pending.resolve(response.result);
        } else {
            pending.reject(new Error(
                `Secure multiplayer identity ${response.error.code}: ${response.error.message}`,
            ));
        }
    });
}

async function signer(): Promise<HTMLIFrameElement> {
    if (signerFrame !== undefined) {
        await signerReady;
        return signerFrame;
    }
    const url = configuredSignerUrl();
    signerOrigin = url.origin;
    installResponseListener();
    signerReady = new Promise<void>((resolve, reject) => {
        resolveSignerReady = resolve;
        rejectSignerReady = reject;
    });
    const frame = document.createElement('iframe');
    frame.hidden = true;
    frame.tabIndex = -1;
    frame.title = 'Secure Robin Hood multiplayer identity signer';
    frame.referrerPolicy = 'no-referrer';
    frame.sandbox.add('allow-scripts', 'allow-same-origin');
    frame.src = url.href;
    signerFrame = frame;
    const timeout = setTimeout(() => {
        rejectSignerReady?.(new Error('Secure multiplayer identity signer did not become ready'));
        rejectSignerReady = undefined;
        resolveSignerReady = undefined;
    }, REQUEST_TIMEOUT_MS);
    frame.addEventListener('error', () => {
        rejectSignerReady?.(new Error('Secure multiplayer identity signer failed to load'));
        rejectSignerReady = undefined;
        resolveSignerReady = undefined;
    }, { once: true });
    document.body.append(frame);
    try {
        await signerReady;
        return frame;
    } finally {
        clearTimeout(timeout);
    }
}

function randomRequestId(): string {
    return Array.from(
        crypto.getRandomValues(new Uint8Array(16)),
        byte => byte.toString(16).padStart(2, '0'),
    ).join('');
}

async function requestIdentity(
    operation: MultiplayerIdentityRequest['operation'],
    fields: Omit<MultiplayerIdentityRequest, 'protocol' | 'requestId' | 'operation'> = {},
): Promise<unknown> {
    const frame = await signer();
    const receiver = frame.contentWindow;
    if (receiver === null || signerOrigin === undefined) {
        throw new Error('Secure multiplayer identity signer has no message receiver');
    }
    const targetOrigin = signerOrigin;
    const requestId = randomRequestId();
    const request: MultiplayerIdentityRequest = {
        protocol: MULTIPLAYER_IDENTITY_PROTOCOL,
        requestId,
        operation,
        ...fields,
    };
    return await new Promise<unknown>((resolve, reject) => {
        const timeout = setTimeout(() => {
            pendingRequests.delete(requestId);
            reject(new Error(`Secure multiplayer identity ${operation} request timed out`));
        }, REQUEST_TIMEOUT_MS);
        pendingRequests.set(requestId, { resolve, reject, timeout });
        receiver.postMessage(request, targetOrigin);
    });
}

function resultObject(value: unknown, label: string, keys: readonly string[]): Record<string, unknown> {
    if (value === null || typeof value !== 'object' || Array.isArray(value)) {
        throw new Error(`${label} returned a non-object result`);
    }
    const object = value as Record<string, unknown>;
    const actual = Object.keys(object).sort();
    const expected = [...keys].sort();
    if (actual.length !== expected.length || actual.some((key, index) => key !== expected[index])) {
        throw new Error(`${label} returned malformed fields`);
    }
    return object;
}

function decodeBase64Url(value: unknown, bytes: number, label: string): Uint8Array<ArrayBuffer> {
    const pattern = bytes === 32 ? BASE64URL_32_RE : BASE64URL_64_RE;
    if (typeof value !== 'string' || !pattern.test(value)) {
        throw new Error(`${label} is not canonical base64url`);
    }
    const padded = value.replaceAll('-', '+').replaceAll('_', '/')
        + '='.repeat((4 - value.length % 4) % 4);
    const binary = atob(padded);
    const decoded = Uint8Array.from(binary, character => character.charCodeAt(0));
    if (decoded.byteLength !== bytes || encodeBase64Url(decoded) !== value) {
        throw new Error(`${label} does not encode ${bytes} bytes`);
    }
    return decoded;
}

function encodeBase64Url(bytes: Uint8Array): string {
    let binary = '';
    for (const byte of bytes) binary += String.fromCharCode(byte);
    return btoa(binary).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '');
}

function hex(bytes: Uint8Array): string {
    return Array.from(bytes, byte => byte.toString(16).padStart(2, '0')).join('');
}

function parseSeatProofMessage(message: Uint8Array): {
    readonly sessionId: string;
    readonly hostEndpointId: string;
    readonly transportEndpointId: string;
} {
    if (
        !(message instanceof Uint8Array)
        || message.byteLength !== SEAT_PROOF_DOMAIN.byteLength + 96
    ) {
        throw new Error('browser multiplayer signer accepts only one exact seat-proof message');
    }
    for (let index = 0; index < SEAT_PROOF_DOMAIN.byteLength; index += 1) {
        if (message[index] !== SEAT_PROOF_DOMAIN[index]) {
            throw new Error('browser multiplayer signer rejected a non-seat-proof domain');
        }
    }
    const payload = message.subarray(SEAT_PROOF_DOMAIN.byteLength);
    return {
        sessionId: encodeBase64Url(payload.subarray(0, 32)),
        hostEndpointId: hex(payload.subarray(32, 64)),
        transportEndpointId: hex(payload.subarray(64, 96)),
    };
}

function validateSessionId(sessionId: string): void {
    decodeBase64Url(sessionId, 32, 'browser multiplayer session id');
}

export async function wasInvitationRedeemed(sessionId: string): Promise<boolean> {
    validateSessionId(sessionId);
    const result = resultObject(
        await requestIdentity('was_redeemed', { sessionId }),
        'Invitation redemption query',
        ['redeemed'],
    );
    if (typeof result.redeemed !== 'boolean') {
        throw new Error('Invitation redemption query returned a malformed flag');
    }
    return result.redeemed;
}

export async function markInvitationRedeemed(sessionId: string): Promise<void> {
    validateSessionId(sessionId);
    const result = resultObject(
        await requestIdentity('mark_redeemed', { sessionId }),
        'Invitation redemption write',
        ['redeemed'],
    );
    if (result.redeemed !== true) {
        throw new Error('Invitation redemption write was not durably acknowledged');
    }
}

export async function installBrowserMultiplayerIdentity(): Promise<BrowserMultiplayerIdentity> {
    const result = resultObject(
        await requestIdentity('status'),
        'Multiplayer identity status',
        ['publicKey', 'persistent'],
    );
    if (result.persistent !== true && result.persistent !== false && result.persistent !== null) {
        throw new Error('Multiplayer identity status returned a malformed persistence flag');
    }
    if (result.persistent === false) {
        console.warn('secure browser multiplayer identity storage is not eviction-resistant');
    }
    const publicKey = decodeBase64Url(result.publicKey, 32, 'durable browser public key');
    const identity: BrowserMultiplayerIdentity = {
        publicKey,
        sign: async (message): Promise<Uint8Array<ArrayBuffer>> => {
            const proof = parseSeatProofMessage(message);
            const signed = resultObject(
                await requestIdentity('sign_seat_proof', proof),
                'Multiplayer seat signer',
                ['signature'],
            );
            return decodeBase64Url(signed.signature, 64, 'multiplayer seat signature');
        },
    };
    globalThis.robinMultiplayerIdentity = identity;
    globalThis.robinMarkMultiplayerInvitationRedeemed = markInvitationRedeemed;
    return identity;
}
